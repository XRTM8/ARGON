use crate::{
    files,
    identity::{self, Identity},
    model::*,
    wire::{Channel, Message, DATA_SIZE},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU16, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    net::{TcpListener, TcpStream, UdpSocket},
    sync::{oneshot, watch, Mutex, RwLock, Semaphore},
    task::{JoinHandle, JoinSet},
};

type Result<T> = std::result::Result<T, String>;
type Emitter = Arc<dyn Fn(Event) + Send + Sync>;
struct PendingPair {
    info: Pairing,
    answer: oneshot::Sender<bool>,
}
struct PendingOffer {
    info: Incoming,
    answer: oneshot::Sender<bool>,
}
struct Selection {
    file: SelectedFile,
    path: PathBuf,
    expires: u64,
}
struct ActiveTransfer {
    peer: String,
    cancelled: Arc<AtomicBool>,
}
struct Run {
    stop: watch::Sender<bool>,
    listener: JoinHandle<()>,
    discovery: Option<JoinHandle<()>>,
}
#[derive(Default)]
struct State {
    peers: HashMap<String, Peer>,
    trusted: HashMap<String, TrustedPeer>,
    transfers: Vec<Transfer>,
    pairings: HashMap<String, PendingPair>,
    incoming: HashMap<String, PendingOffer>,
    selections: HashMap<String, Selection>,
    active: HashMap<String, ActiveTransfer>,
    clipboard: Vec<ClipboardItem>,
    last_error: Option<String>,
    trust_epoch: u64,
}
#[derive(Serialize, Deserialize)]
struct Preferences {
    receive_directory: PathBuf,
}
#[derive(Serialize, Deserialize)]
struct Beacon {
    protocol: String,
    id: String,
    name: String,
    port: u16,
}

pub struct FlowService {
    config: Config,
    identity: Identity,
    emit: Emitter,
    state: RwLock<State>,
    receive_directory: RwLock<PathBuf>,
    lifecycle: Mutex<Option<Run>>,
    enabled: AtomicBool,
    port: AtomicU16,
    outgoing: Mutex<Vec<JoinHandle<()>>>,
    send_limit: Arc<Semaphore>,
    partials: Arc<std::sync::Mutex<HashSet<String>>>,
    persist: Mutex<()>,
}

impl FlowService {
    pub async fn new(
        config: Config,
        emit: impl Fn(Event) + Send + Sync + 'static,
    ) -> Result<Arc<Self>> {
        let identity = Identity::load_or_create(&config.storage)?;
        let receive = match std::fs::read(config.storage.join("preferences.json")) {
            Ok(bytes) => {
                serde_json::from_slice::<Preferences>(&bytes)
                    .map_err(|_| "Invalid sharing settings")?
                    .receive_directory
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => config.receive_directory.clone(),
            Err(e) => return Err(e.to_string()),
        };
        let directory = files::validate_directory(&receive).await?;
        let trusted =
            load_json::<HashMap<String, TrustedPeer>>(&config.storage.join("trusted-v2.json"))?;
        if trusted
            .iter()
            .any(|(id, peer)| !valid_hash(id) || peer.id != *id)
        {
            return Err("Invalid trusted device data".into());
        }
        let mut transfers = load_json::<Vec<Transfer>>(&config.storage.join("history-v2.json"))?;
        transfers.truncate(100);
        for transfer in &mut transfers {
            if !["completed", "error", "cancelled"].contains(&transfer.status.as_str()) {
                transfer.status = "error".into();
                transfer.speed = 0.0;
                transfer.error = Some(
                    "The app closed before completion. Select the file again to resume safely."
                        .into(),
                );
            }
        }
        let peers = trusted
            .values()
            .map(|p| {
                (
                    p.id.clone(),
                    Peer {
                        id: p.id.clone(),
                        name: p.name.clone(),
                        address: p.address.clone(),
                        trusted: true,
                        online: false,
                        last_seen: 0,
                    },
                )
            })
            .collect();
        Ok(Arc::new(Self {
            identity,
            receive_directory: RwLock::new(directory),
            config,
            emit: Arc::new(emit),
            state: RwLock::new(State {
                peers,
                trusted,
                transfers,
                ..Default::default()
            }),
            lifecycle: Mutex::new(None),
            enabled: AtomicBool::new(false),
            port: AtomicU16::new(0),
            outgoing: Mutex::new(vec![]),
            send_limit: Arc::new(Semaphore::new(3)),
            partials: Arc::new(std::sync::Mutex::new(HashSet::new())),
            persist: Mutex::new(()),
        }))
    }

    fn changed(&self) {
        (self.emit)(Event::Changed);
    }
    async fn error(&self, error: String) {
        self.state.write().await.last_error = Some(error.clone());
        (self.emit)(Event::Error(error));
    }
    pub async fn snapshot(&self) -> Snapshot {
        self.expire_requests().await;
        let state = self.state.read().await;
        let mut peers: Vec<_> = state.peers.values().cloned().collect();
        peers.sort_by(|a, b| b.trusted.cmp(&a.trusted).then(a.name.cmp(&b.name)));
        let port = self.port.load(Ordering::Relaxed);
        Snapshot {
            enabled: self.enabled.load(Ordering::Relaxed),
            id: self.identity.id.clone(),
            name: safe_name(&self.config.name),
            port,
            addresses: local_addresses(port),
            receive_directory: self
                .receive_directory
                .read()
                .await
                .to_string_lossy()
                .into_owned(),
            peers,
            transfers: state.transfers.clone(),
            pairings: state.pairings.values().map(|p| p.info.clone()).collect(),
            incoming: state.incoming.values().map(|p| p.info.clone()).collect(),
            clipboard: state.clipboard.clone(),
            last_error: state.last_error.clone(),
            protocol: PROTOCOL.into(),
        }
    }

    pub async fn start(self: &Arc<Self>) -> Result<()> {
        let mut lifecycle = self.lifecycle.lock().await;
        if lifecycle.is_some() {
            return Ok(());
        }
        let listener = match TcpListener::bind((self.config.bind_address, self.config.port)).await {
            Ok(listener) => listener,
            Err(_) if self.config.port != 0 => TcpListener::bind((self.config.bind_address, 0))
                .await
                .map_err(|e| e.to_string())?,
            Err(e) => return Err(e.to_string()),
        };
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        self.port.store(port, Ordering::Relaxed);
        let mut discovery_error = None;
        let discovery_socket = if self.config.discovery {
            match bind_discovery() {
                Ok(socket) => Some(socket),
                Err(error) => {
                    discovery_error = Some(format!("Device discovery is unavailable ({error}). Connect using a local address instead."));
                    None
                }
            }
        } else {
            None
        };
        let (stop, mut stopped) = watch::channel(false);
        let service = self.clone();
        let limit = Arc::new(Semaphore::new(8));
        let worker_stop = stopped.clone();
        let accept = tokio::spawn(async move {
            let mut children = JoinSet::new();
            let mut tick = tokio::time::interval(Duration::from_secs(2));
            let mut attempts: HashMap<IpAddr, (u64, u32)> = HashMap::new();
            loop {
                tokio::select! {
                    _ = stopped.changed() => break,
                    _ = tick.tick() => { service.expire_requests().await; service.expire_peers().await; attempts.retain(|_,(time,_)| now().saturating_sub(*time)<60000); },
                    Some(_) = children.join_next(), if !children.is_empty() => {},
                    accepted = listener.accept() => {
                        let Ok((stream,address)) = accepted else { continue; };
                        if !local_ip(address.ip()) { continue; }
                        let entry=attempts.entry(address.ip()).or_insert((now(),0));
                        if now().saturating_sub(entry.0)>=60000 { *entry=(now(),0); }
                        if entry.1>=20 { continue; } entry.1+=1;
                        let Ok(permit)=limit.clone().try_acquire_owned() else { continue; };
                        let service=service.clone(); let mut stop=worker_stop.clone();
                        children.spawn(async move {
                            let _permit=permit;
                            let result=tokio::select! { _=stop.changed()=>Err("Sharing stopped".into()), r=service.receive_connection(stream,address)=>r };
                            if let Err(error)=result { if service.enabled.load(Ordering::Relaxed) { service.error(error).await; } }
                        });
                    }
                }
            }
            children.abort_all();
            while children.join_next().await.is_some() {}
        });
        let discovery = discovery_socket.map(|socket| {
            let service = self.clone();
            let mut stop = stop.subscribe();
            tokio::spawn(async move {
                service.discovery_loop(socket, &mut stop).await;
            })
        });
        self.enabled.store(true, Ordering::Relaxed);
        self.state.write().await.last_error = discovery_error;
        *lifecycle = Some(Run {
            stop,
            listener: accept,
            discovery,
        });
        drop(lifecycle);
        self.changed();
        Ok(())
    }

    pub async fn stop(&self) {
        let run = self.lifecycle.lock().await.take();
        self.enabled.store(false, Ordering::Relaxed);
        if let Some(run) = run {
            let _ = run.stop.send(true);
            {
                let mut state = self.state.write().await;
                for active in state.active.values() {
                    active.cancelled.store(true, Ordering::Relaxed);
                }
                for (_, pair) in state.pairings.drain() {
                    let _ = pair.answer.send(false);
                }
                for (_, offer) in state.incoming.drain() {
                    let _ = offer.answer.send(false);
                }
            }
            let _ = run.listener.await;
            if let Some(task) = run.discovery {
                let _ = task.await;
            }
            for task in self.outgoing.lock().await.drain(..) {
                let _ = task.await;
            }
        }
        {
            let mut state = self.state.write().await;
            for peer in state.peers.values_mut() {
                peer.online = false;
            }
            for transfer in &mut state.transfers {
                if !["completed", "error", "cancelled"].contains(&transfer.status.as_str()) {
                    transfer.status = "cancelled".into();
                    transfer.speed = 0.0;
                    transfer.error = Some(
                        "Sharing stopped; partial data is retained for verified resume".into(),
                    );
                }
            }
            state.active.clear();
        }
        let _ = self.save_history().await;
        self.changed();
    }

    async fn stop_receiver(&self) -> Result<watch::Receiver<bool>> {
        self.lifecycle
            .lock()
            .await
            .as_ref()
            .map(|run| run.stop.subscribe())
            .ok_or_else(|| "Turn on local sharing first".into())
    }
    pub async fn set_receive_directory(&self, directory: PathBuf) -> Result<()> {
        if !self.state.read().await.active.is_empty() {
            return Err("Wait for active transfers before changing the receive folder".into());
        }
        let directory = files::validate_directory(&directory).await?;
        identity::save_json(
            &self.config.storage.join("preferences.json"),
            &Preferences {
                receive_directory: directory.clone(),
            },
        )?;
        *self.receive_directory.write().await = directory;
        self.changed();
        Ok(())
    }
    pub async fn select_files(&self, paths: Vec<PathBuf>) -> Result<Vec<SelectedFile>> {
        if paths.is_empty() || paths.len() > 32 {
            return Err("Select between 1 and 32 files".into());
        }
        let mut selected = Vec::new();
        for path in paths {
            let path = tokio::fs::canonicalize(path)
                .await
                .map_err(|e| e.to_string())?;
            let meta = tokio::fs::metadata(&path)
                .await
                .map_err(|e| e.to_string())?;
            if !meta.is_file() || meta.len() > MAX_FILE {
                return Err("Select regular files no larger than 50 GB".into());
            }
            selected.push((
                SelectedFile {
                    id: uid(),
                    name: files::sanitize_filename(
                        &path.file_name().unwrap_or_default().to_string_lossy(),
                    ),
                    size: meta.len(),
                },
                path,
            ));
        }
        let result = selected.iter().map(|(file, _)| file.clone()).collect();
        let mut state = self.state.write().await;
        state.selections.retain(|_, file| file.expires > now());
        if state.selections.len() + selected.len() > 128 {
            return Err("Too many selected files; wait for previous selections to expire".into());
        }
        for (file, path) in selected {
            state.selections.insert(
                file.id.clone(),
                Selection {
                    file,
                    path,
                    expires: now() + 30 * 60 * 1000,
                },
            );
        }
        Ok(result)
    }
    pub async fn connect(self: &Arc<Self>, address: &str) -> Result<()> {
        let address = files::local_endpoint(address)?;
        let stop = self.stop_receiver().await?;
        let service = self.clone();
        self.spawn_job(stop, async move {
            let mut channel = service.open(address, None).await?;
            channel.send(&Message::Ping).await?;
            if !matches!(channel.receive().await?, Message::Pong) {
                return Err("Unexpected pairing confirmation".into());
            }
            Ok(())
        })
        .await;
        Ok(())
    }
    pub async fn pair_peer(self: &Arc<Self>, id: &str) -> Result<()> {
        let peer = self
            .state
            .read()
            .await
            .peers
            .get(id)
            .cloned()
            .ok_or("Device is no longer available")?;
        let stop = self.stop_receiver().await?;
        let service = self.clone();
        self.spawn_job(stop, async move {
            let mut channel = service.open(peer.endpoint()?, Some(&peer.id)).await?;
            channel.send(&Message::Ping).await?;
            if !matches!(channel.receive().await?, Message::Pong) {
                return Err("Pairing was not confirmed by the other device".into());
            }
            Ok(())
        })
        .await;
        Ok(())
    }
    async fn spawn_job(
        self: &Arc<Self>,
        mut stop: watch::Receiver<bool>,
        job: impl std::future::Future<Output = Result<()>> + Send + 'static,
    ) {
        let service = self.clone();
        let task = tokio::spawn(async move {
            let result = tokio::select! { _=stop.changed()=>Ok(()), result=job=>result };
            if let Err(error) = result {
                service.error(error).await;
            }
        });
        let mut outgoing = self.outgoing.lock().await;
        outgoing.retain(|task| !task.is_finished());
        outgoing.push(task);
    }
    pub async fn confirm_pairing(&self, id: &str, accepted: bool) -> Result<()> {
        let pending = self
            .state
            .write()
            .await
            .pairings
            .remove(id)
            .ok_or("Pairing request expired")?;
        let _ = pending
            .answer
            .send(accepted && pending.info.expires_at > now());
        self.changed();
        Ok(())
    }
    pub async fn respond_incoming(&self, id: &str, accepted: bool) -> Result<()> {
        let pending = self
            .state
            .write()
            .await
            .incoming
            .remove(id)
            .ok_or("Receive request expired")?;
        let _ = pending
            .answer
            .send(accepted && pending.info.expires_at > now());
        self.changed();
        Ok(())
    }
    pub async fn revoke(&self, id: &str) -> Result<()> {
        let mut state = self.state.write().await;
        state.trust_epoch += 1;
        state.trusted.remove(id);
        if let Some(peer) = state.peers.get_mut(id) {
            peer.trusted = false;
        }
        for active in state.active.values().filter(|active| active.peer == id) {
            active.cancelled.store(true, Ordering::Relaxed);
        }
        identity::save_json(&self.config.storage.join("trusted-v2.json"), &state.trusted)?;
        drop(state);
        self.changed();
        Ok(())
    }
    pub async fn cancel(&self, id: &str) -> Result<()> {
        let state = self.state.read().await;
        let active = state
            .active
            .get(id)
            .ok_or("Transfer has already finished")?;
        active.cancelled.store(true, Ordering::Relaxed);
        Ok(())
    }
    pub async fn send_files(
        self: &Arc<Self>,
        peer_id: &str,
        ids: Vec<String>,
    ) -> Result<Vec<String>> {
        if ids.is_empty() || ids.len() > 32 {
            return Err("Select between 1 and 32 files".into());
        }
        let stop = self.stop_receiver().await?;
        let state = self.state.read().await;
        let peer = state
            .peers
            .get(peer_id)
            .filter(|peer| peer.trusted)
            .cloned()
            .ok_or("Pair the device before sending files")?;
        if state.active.len() + ids.len() > 32 {
            return Err("Too many active transfers".into());
        }
        let mut files = Vec::new();
        for id in ids {
            let selected = state
                .selections
                .get(&id)
                .filter(|f| f.expires > now())
                .ok_or("File selection expired; choose the file again")?;
            files.push((selected.file.clone(), selected.path.clone()));
        }
        drop(state);
        let mut result = Vec::new();
        for (file, path) in files {
            let id = uid();
            let cancel = Arc::new(AtomicBool::new(false));
            self.register_transfer(
                Transfer {
                    id: id.clone(),
                    peer_id: peer.id.clone(),
                    peer_name: peer.name.clone(),
                    name: file.name,
                    direction: "send".into(),
                    kind: "file".into(),
                    status: "queued".into(),
                    bytes: 0,
                    total: file.size,
                    speed: 0.0,
                    created_at: now(),
                    path: Some(path.to_string_lossy().into_owned()),
                    error: None,
                    sha256: None,
                },
                cancel.clone(),
            )
            .await?;
            let service = self.clone();
            let peer = peer.clone();
            let transfer_id = id.clone();
            let mut stop = stop.clone();
            let task = tokio::spawn(async move {
                let work = async {
                    let _permit = service
                        .send_limit
                        .clone()
                        .acquire_owned()
                        .await
                        .map_err(|e| e.to_string())?;
                    files::check_cancel(&cancel)?;
                    service
                        .send_file(&peer, &transfer_id, &path, cancel.clone())
                        .await
                };
                let result = tokio::select! { _=stop.changed()=>Err("Sharing stopped".into()), result=work=>result, _=cancelled(&cancel)=>Err("Transfer cancelled".into()) };
                service.finish(&transfer_id, result).await;
            });
            let mut outgoing = self.outgoing.lock().await;
            outgoing.retain(|task| !task.is_finished());
            outgoing.push(task);
            result.push(id);
        }
        self.changed();
        Ok(result)
    }
    pub async fn send_text(self: &Arc<Self>, peer_id: &str, text: String) -> Result<String> {
        if text.trim().is_empty() || text.len() > 16 * 1024 {
            return Err("Clipboard text must be between 1 byte and 16 KB".into());
        }
        let mut stop = self.stop_receiver().await?;
        let peer = self
            .state
            .read()
            .await
            .peers
            .get(peer_id)
            .filter(|p| p.trusted)
            .cloned()
            .ok_or("Pair the device before sending text")?;
        let id = uid();
        let cancel = Arc::new(AtomicBool::new(false));
        let hash = hex(&Sha256::digest(text.as_bytes()));
        self.register_transfer(
            Transfer {
                id: id.clone(),
                peer_id: peer.id.clone(),
                peer_name: peer.name.clone(),
                name: "Clipboard text".into(),
                direction: "send".into(),
                kind: "text".into(),
                status: "waiting".into(),
                bytes: 0,
                total: text.len() as u64,
                speed: 0.0,
                created_at: now(),
                path: None,
                error: None,
                sha256: Some(hash.clone()),
            },
            cancel.clone(),
        )
        .await?;
        let service = self.clone();
        let transfer_id = id.clone();
        let task = tokio::spawn(async move {
            let work = async {
                let mut channel = service.open(peer.endpoint()?, Some(&peer.id)).await?;
                channel
                    .send(&Message::Text {
                        id: transfer_id.clone(),
                        text: text.clone(),
                    })
                    .await?;
                match channel.receive().await? {
                    Message::Complete { size, sha256 }
                        if size == text.len() as u64 && sha256 == hash =>
                    {
                        Ok(())
                    }
                    _ => Err("Clipboard delivery was not confirmed".into()),
                }
            };
            let result = tokio::select! { _=stop.changed()=>Err("Sharing stopped".into()), result=work=>result, _=cancelled(&cancel)=>Err("Transfer cancelled".into()) };
            service.finish(&transfer_id, result).await;
        });
        let mut outgoing = self.outgoing.lock().await;
        outgoing.retain(|task| !task.is_finished());
        outgoing.push(task);
        self.changed();
        Ok(id)
    }

    async fn register_transfer(&self, transfer: Transfer, cancel: Arc<AtomicBool>) -> Result<()> {
        let mut state = self.state.write().await;
        if state.active.len() >= 32 || state.active.contains_key(&transfer.id) {
            return Err("Transfer limit reached or duplicate transfer ID".into());
        }
        state.active.insert(
            transfer.id.clone(),
            ActiveTransfer {
                peer: transfer.peer_id.clone(),
                cancelled: cancel,
            },
        );
        state.transfers.insert(0, transfer);
        if state.transfers.len() > 132 {
            let active: HashSet<_> = state.active.keys().cloned().collect();
            let mut finished = 0;
            state.transfers.retain(|t| {
                if active.contains(&t.id) {
                    true
                } else {
                    finished += 1;
                    finished <= 100
                }
            });
        }
        drop(state);
        self.changed();
        Ok(())
    }
    async fn update(&self, id: &str, update: impl FnOnce(&mut Transfer)) {
        let mut state = self.state.write().await;
        if let Some(transfer) = state.transfers.iter_mut().find(|t| t.id == id) {
            update(transfer);
        }
        drop(state);
        self.changed();
    }
    async fn finish(&self, id: &str, result: Result<()>) {
        let error = result.err();
        self.update(id, |transfer| {
            transfer.speed = 0.0;
            if let Some(error) = error {
                transfer.status =
                    if error.to_lowercase().contains("cancel") || error.contains("stopped") {
                        "cancelled"
                    } else {
                        "error"
                    }
                    .into();
                transfer.error = Some(error);
            } else {
                transfer.status = "completed".into();
                transfer.bytes = transfer.total;
                transfer.error = None;
            }
        })
        .await;
        self.state.write().await.active.remove(id);
        let _ = self.save_history().await;
        self.changed();
    }
    async fn save_history(&self) -> Result<()> {
        let _guard = self.persist.lock().await;
        let state = self.state.read().await;
        let history: Vec<_> = state.transfers.iter().take(100).cloned().collect();
        identity::save_json(&self.config.storage.join("history-v2.json"), &history)
    }

    async fn open(&self, address: SocketAddr, expected: Option<&str>) -> Result<Channel> {
        let stream = tokio::time::timeout(Duration::from_secs(12), TcpStream::connect(address))
            .await
            .map_err(|_| "Connection timed out".to_owned())?
            .map_err(|e| format!("Cannot reach device: {e}"))?;
        let mut channel = tokio::time::timeout(
            Duration::from_secs(15),
            Channel::handshake(stream, &self.identity, true),
        )
        .await
        .map_err(|_| "Secure handshake timed out".to_owned())??;
        self.authorize(&mut channel, address, expected).await?;
        Ok(channel)
    }
    async fn authorize(
        &self,
        channel: &mut Channel,
        address: SocketAddr,
        expected: Option<&str>,
    ) -> Result<Peer> {
        if expected.is_some_and(|id| id != channel.peer_id) {
            return Err("Device identity changed. Revoke the old device and pair again after checking both screens.".into());
        }
        let (trusted, epoch) = {
            let state = self.state.read().await;
            (
                state.trusted.contains_key(&channel.peer_id),
                state.trust_epoch,
            )
        };
        channel
            .send(&Message::Hello {
                id: self.identity.id.clone(),
                name: safe_name(&self.config.name),
                protocol: PROTOCOL.into(),
                port: self.port.load(Ordering::Relaxed),
                paired: trusted,
            })
            .await?;
        let (name, id, port, remote_paired) = match channel.receive().await? {
            Message::Hello {
                name,
                id,
                protocol,
                port,
                paired,
            } if protocol == PROTOCOL
                && id == channel.peer_id
                && id != self.identity.id
                && port > 0 =>
            {
                (safe_name(&name), id, port, paired)
            }
            _ => return Err("The peer did not present a valid ARGON Flow v2 identity".into()),
        };
        if !trusted || !remote_paired {
            let info = Pairing {
                id: uid(),
                peer_id: id.clone(),
                peer_name: name.clone(),
                code: channel.code.clone(),
                expires_at: now() + 90000,
            };
            let (tx, rx) = oneshot::channel();
            {
                let mut state = self.state.write().await;
                if state.pairings.len() >= 4
                    || state.pairings.values().any(|p| p.info.peer_id == id)
                {
                    return Err("Another pairing is already pending".into());
                }
                state.pairings.insert(
                    info.id.clone(),
                    PendingPair {
                        info: info.clone(),
                        answer: tx,
                    },
                );
            }
            (self.emit)(Event::Pairing(info.clone()));
            self.changed();
            let accepted = tokio::time::timeout(Duration::from_secs(90), rx)
                .await
                .ok()
                .and_then(|r| r.ok())
                .unwrap_or(false);
            self.state.write().await.pairings.remove(&info.id);
            self.changed();
            channel.send(&Message::Decision { accepted }).await?;
            if !accepted {
                return Err("Pairing declined or expired".into());
            }
        } else {
            channel.send(&Message::Decision { accepted: true }).await?;
        }
        if !matches!(
            channel.receive().await?,
            Message::Decision { accepted: true }
        ) {
            return Err("The other device declined pairing".into());
        }
        let peer = Peer {
            id: id.clone(),
            name: name.clone(),
            address: SocketAddr::new(address.ip(), port).to_string(),
            trusted: true,
            online: true,
            last_seen: now(),
        };
        let mut state = self.state.write().await;
        if state.trust_epoch != epoch {
            return Err("Device trust changed during this session; reconnect".into());
        }
        state.trusted.insert(
            id.clone(),
            TrustedPeer {
                id: id.clone(),
                name,
                address: peer.address.clone(),
                paired_at: now(),
            },
        );
        identity::save_json(&self.config.storage.join("trusted-v2.json"), &state.trusted)?;
        state.peers.insert(id, peer.clone());
        drop(state);
        self.changed();
        Ok(peer)
    }

    async fn receive_connection(
        self: &Arc<Self>,
        stream: TcpStream,
        address: SocketAddr,
    ) -> Result<()> {
        let mut channel = tokio::time::timeout(
            Duration::from_secs(15),
            Channel::handshake(stream, &self.identity, false),
        )
        .await
        .map_err(|_| "Secure handshake timed out".to_owned())??;
        let peer = self.authorize(&mut channel, address, None).await?;
        match channel.receive().await? {
            Message::Ping => channel.send(&Message::Pong).await,
            Message::Text { id, text } => self.receive_text(&mut channel, &peer, id, text).await,
            Message::File {
                id,
                name,
                size,
                sha256,
            } => {
                self.receive_file(&mut channel, &peer, id, name, size, sha256)
                    .await
            }
            _ => Err("Unsupported encrypted operation".into()),
        }
    }
    async fn consent(
        &self,
        peer: &Peer,
        id: &str,
        name: String,
        size: u64,
        kind: &str,
    ) -> Result<bool> {
        if uuid::Uuid::parse_str(id).is_err() {
            return Err("Invalid transfer ID".into());
        }
        let info = Incoming {
            id: id.into(),
            peer_id: peer.id.clone(),
            peer_name: peer.name.clone(),
            name,
            size,
            kind: kind.into(),
            expires_at: now() + 90000,
        };
        let (tx, rx) = oneshot::channel();
        {
            let mut state = self.state.write().await;
            if state.incoming.len() >= 8 || state.incoming.contains_key(id) {
                return Err("Too many incoming requests".into());
            }
            state.incoming.insert(
                id.into(),
                PendingOffer {
                    info: info.clone(),
                    answer: tx,
                },
            );
        }
        (self.emit)(Event::Incoming(info));
        self.changed();
        let accepted = tokio::time::timeout(Duration::from_secs(90), rx)
            .await
            .ok()
            .and_then(|r| r.ok())
            .unwrap_or(false);
        self.state.write().await.incoming.remove(id);
        self.changed();
        Ok(accepted && self.state.read().await.trusted.contains_key(&peer.id))
    }
    async fn receive_text(
        &self,
        channel: &mut Channel,
        peer: &Peer,
        id: String,
        text: String,
    ) -> Result<()> {
        if text.is_empty() || text.len() > 16 * 1024 {
            return Err("Clipboard text exceeds the allowed size".into());
        }
        if !self
            .consent(
                peer,
                &id,
                "Clipboard text".into(),
                text.len() as u64,
                "text",
            )
            .await?
        {
            channel
                .send(&Message::Error {
                    message: "Clipboard receive request declined".into(),
                })
                .await?;
            return Ok(());
        }
        let hash = hex(&Sha256::digest(text.as_bytes()));
        self.register_transfer(
            Transfer {
                id: id.clone(),
                peer_id: peer.id.clone(),
                peer_name: peer.name.clone(),
                name: "Clipboard text".into(),
                direction: "receive".into(),
                kind: "text".into(),
                status: "receiving".into(),
                bytes: 0,
                total: text.len() as u64,
                speed: 0.0,
                created_at: now(),
                path: None,
                error: None,
                sha256: Some(hash.clone()),
            },
            Arc::new(AtomicBool::new(false)),
        )
        .await?;
        {
            let mut state = self.state.write().await;
            state.clipboard.insert(
                0,
                ClipboardItem {
                    id: id.clone(),
                    peer_name: peer.name.clone(),
                    text: text.clone(),
                    received_at: now(),
                },
            );
            state.clipboard.truncate(8);
        }
        let result = channel
            .send(&Message::Complete {
                size: text.len() as u64,
                sha256: hash,
            })
            .await;
        self.finish(&id, result.clone()).await;
        result
    }

    async fn send_file(
        &self,
        peer: &Peer,
        id: &str,
        path: &Path,
        cancel: Arc<AtomicBool>,
    ) -> Result<()> {
        self.update(id, |t| t.status = "hashing".into()).await;
        let (size, hasher) = files::hash_file(path, None, &cancel).await?;
        let hash = files::digest(hasher);
        if size > MAX_FILE {
            return Err("File exceeds the 50 GB limit".into());
        }
        let name =
            files::sanitize_filename(&path.file_name().unwrap_or_default().to_string_lossy());
        self.update(id, |t| {
            t.sha256 = Some(hash.clone());
            t.total = size;
            t.status = "waiting".into();
        })
        .await;
        let mut channel = self.open(peer.endpoint()?, Some(&peer.id)).await?;
        channel
            .send(&Message::File {
                id: id.into(),
                name,
                size,
                sha256: hash.clone(),
            })
            .await?;
        let (offset, prefix) = match channel.receive().await? {
            Message::Ready {
                offset,
                prefix_sha256,
            } if offset <= size => (offset, prefix_sha256),
            _ => return Err("Invalid resume response".into()),
        };
        files::check_cancel(&cancel)?;
        let (_, prefix_hasher) = files::hash_file(path, Some(offset), &cancel).await?;
        let reset = files::digest(prefix_hasher) != prefix;
        let offset = if reset { 0 } else { offset };
        channel.send(&Message::Resume { reset }).await?;
        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(|e| e.to_string())?;
        file.seek(std::io::SeekFrom::Start(offset))
            .await
            .map_err(|e| e.to_string())?;
        let mut sent = offset;
        let mut buffer = vec![0; DATA_SIZE];
        let start = Instant::now();
        let mut last = Instant::now();
        self.update(id, |t| {
            t.status = "sending".into();
            t.bytes = offset;
        })
        .await;
        while sent < size {
            files::check_cancel(&cancel)?;
            let count = (size - sent).min(buffer.len() as u64) as usize;
            let n = file
                .read(&mut buffer[..count])
                .await
                .map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("Source file changed during transfer".into());
            }
            channel.send_data(&buffer[..n]).await?;
            sent += n as u64;
            if last.elapsed() >= Duration::from_millis(100) {
                self.update(id, |t| {
                    t.bytes = sent;
                    t.speed = (sent - offset) as f64 / start.elapsed().as_secs_f64().max(0.001);
                })
                .await;
                last = Instant::now();
            }
        }
        channel
            .send(&Message::End {
                sha256: hash.clone(),
            })
            .await?;
        self.update(id, |t| {
            t.status = "verifying".into();
            t.bytes = size;
        })
        .await;
        match channel.receive().await? {
            Message::Complete {
                size: received,
                sha256,
            } if received == size && sha256 == hash => Ok(()),
            _ => Err("Receiver did not confirm the saved file and its checksum".into()),
        }
    }

    async fn receive_file(
        &self,
        channel: &mut Channel,
        peer: &Peer,
        id: String,
        name: String,
        size: u64,
        hash: String,
    ) -> Result<()> {
        if size > MAX_FILE || !valid_hash(&hash) || name.len() > 1024 {
            return Err("Invalid file offer".into());
        }
        let name = files::sanitize_filename(&name);
        if !self.consent(peer, &id, name.clone(), size, "file").await? {
            channel
                .send(&Message::Error {
                    message: "File receive request declined".into(),
                })
                .await?;
            return Ok(());
        }
        let partial_id = files::partial_id(&hash, &name);
        let key = format!("{}-{partial_id}", peer.id);
        let _partial_guard = PartialGuard::acquire(self.partials.clone(), key)?;
        let cancel = Arc::new(AtomicBool::new(false));
        self.register_transfer(
            Transfer {
                id: id.clone(),
                peer_id: peer.id.clone(),
                peer_name: peer.name.clone(),
                name: name.clone(),
                direction: "receive".into(),
                kind: "file".into(),
                status: "receiving".into(),
                bytes: 0,
                total: size,
                speed: 0.0,
                created_at: now(),
                path: None,
                error: None,
                sha256: Some(hash.clone()),
            },
            cancel.clone(),
        )
        .await?;
        let work = async {
            let directory = self.receive_directory.read().await.clone();
            let partial = files::partial_path(&directory, &peer.id, &partial_id).await?;
            let mut file = tokio::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(&partial)
                .await
                .map_err(|e| e.to_string())?;
            let mut offset = file.metadata().await.map_err(|e| e.to_string())?.len();
            if offset > size {
                file.set_len(0).await.map_err(|e| e.to_string())?;
                offset = 0;
            }
            files::ensure_space(&directory, size.saturating_sub(offset))?;
            let (_, mut hasher) = files::hash_file(&partial, Some(offset), &cancel).await?;
            channel
                .send(&Message::Ready {
                    offset,
                    prefix_sha256: files::digest(hasher.clone()),
                })
                .await?;
            match channel.receive().await? {
                Message::Resume { reset } => {
                    if reset {
                        file.set_len(0).await.map_err(|e| e.to_string())?;
                        offset = 0;
                        hasher = Sha256::new();
                    }
                }
                _ => return Err("Expected resume confirmation".into()),
            }
            file.seek(std::io::SeekFrom::Start(offset))
                .await
                .map_err(|e| e.to_string())?;
            let mut received = offset;
            let start = Instant::now();
            let mut last = Instant::now();
            self.update(&id, |t| t.bytes = offset).await;
            loop {
                files::check_cancel(&cancel)?;
                let record =
                    tokio::time::timeout(Duration::from_secs(30), channel.receive_record())
                        .await
                        .map_err(|_| "Transfer stalled".to_owned())??;
                match record[0] {
                    1 => {
                        let body = &record[1..];
                        if body.is_empty()
                            || body.len() > DATA_SIZE
                            || received + body.len() as u64 > size
                        {
                            return Err("Peer sent data outside the offered file size".into());
                        }
                        file.write_all(body)
                            .await
                            .map_err(|e| format!("Disk write failed: {e}"))?;
                        hasher.update(body);
                        received += body.len() as u64;
                        if last.elapsed() >= Duration::from_millis(100) {
                            self.update(&id, |t| {
                                t.bytes = received;
                                t.speed = (received - offset) as f64
                                    / start.elapsed().as_secs_f64().max(0.001);
                            })
                            .await;
                            last = Instant::now();
                        }
                    }
                    0 => {
                        let end: Message =
                            serde_json::from_slice(&record[1..]).map_err(|e| e.to_string())?;
                        if !matches!(end,Message::End { ref sha256 } if sha256==&hash)
                            || received != size
                        {
                            return Err("File ended before all authenticated bytes arrived".into());
                        }
                        break;
                    }
                    _ => return Err("Unknown encrypted record".into()),
                }
            }
            self.update(&id, |t| {
                t.status = "verifying".into();
                t.bytes = received;
            })
            .await;
            if files::digest(hasher) != hash {
                return Err(
                    "SHA-256 verification failed; partial file retained for safe retry".into(),
                );
            }
            file.flush().await.map_err(|e| e.to_string())?;
            file.sync_all().await.map_err(|e| e.to_string())?;
            if file.metadata().await.map_err(|e| e.to_string())?.len() != size {
                return Err("Saved file length is incorrect".into());
            }
            drop(file);
            files::check_cancel(&cancel)?;
            let saved = files::commit_file(&partial, &directory, &name).await?;
            self.update(&id, |t| {
                t.path = Some(saved.to_string_lossy().into_owned());
                t.name = saved
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
            })
            .await;
            channel
                .send(&Message::Complete { size, sha256: hash })
                .await?;
            Ok::<(), String>(())
        };
        let result = tokio::select! { result=work=>result, _=cancelled(&cancel)=>Err("Transfer cancelled".into()) };
        if let Err(error) = &result {
            let _ = channel
                .send(&Message::Error {
                    message: error.clone(),
                })
                .await;
        }
        self.finish(&id, result.clone()).await;
        result
    }

    async fn expire_requests(&self) {
        let mut state = self.state.write().await;
        let time = now();
        state.pairings.retain(|_, p| p.info.expires_at > time);
        state.incoming.retain(|_, p| p.info.expires_at > time);
        state.selections.retain(|_, file| file.expires > time);
    }
    async fn expire_peers(&self) {
        let mut state = self.state.write().await;
        for peer in state.peers.values_mut() {
            if now().saturating_sub(peer.last_seen) > 12000 {
                peer.online = false;
            }
        }
        state
            .peers
            .retain(|_, p| p.trusted || now().saturating_sub(p.last_seen) < 60000);
    }
    async fn discovery_loop(self: Arc<Self>, socket: UdpSocket, stop: &mut watch::Receiver<bool>) {
        let mut tick = tokio::time::interval(Duration::from_secs(3));
        let mut buffer = [0; 2048];
        loop {
            tokio::select! {
                _=stop.changed()=>break,
                _=tick.tick()=> {
                    let message=Beacon { protocol:PROTOCOL.into(),id:self.identity.id.clone(),name:safe_name(&self.config.name),port:self.port.load(Ordering::Relaxed) };
                    if let Ok(bytes)=serde_json::to_vec(&message) { let _=socket.send_to(&bytes,(Ipv4Addr::BROADCAST,DISCOVERY_PORT)).await; }
                }
                received=socket.recv_from(&mut buffer)=> {
                    let Ok((size,source))=received else { continue; };
                    if !local_ip(source.ip()) { continue; }
                    let Ok(beacon)=serde_json::from_slice::<Beacon>(&buffer[..size]) else { continue; };
                    if beacon.protocol!=PROTOCOL || beacon.id==self.identity.id || !valid_hash(&beacon.id) || beacon.port==0 { continue; }
                    let mut state=self.state.write().await;
                    if state.peers.len()>=64 && !state.peers.contains_key(&beacon.id) { continue; }
                    let trusted=state.trusted.contains_key(&beacon.id);
                    state.peers.insert(beacon.id.clone(),Peer { id:beacon.id,name:safe_name(&beacon.name),address:SocketAddr::new(source.ip(),beacon.port).to_string(),trusted,online:true,last_seen:now() });
                    drop(state);self.changed();
                }
            }
        }
    }
}

async fn cancelled(flag: &Arc<AtomicBool>) {
    while !flag.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}
fn local_ip(ip: IpAddr) -> bool {
    matches!(ip,IpAddr::V4(ip) if ip.is_private() || ip.is_loopback() || ip.is_link_local())
}
fn local_addresses(port: u16) -> Vec<String> {
    let mut result = vec![format!("127.0.0.1:{port}")];
    if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if socket.connect("192.0.2.1:9").is_ok() {
            if let Ok(address) = socket.local_addr() {
                if !address.ip().is_loopback() && !address.ip().is_unspecified() {
                    result.insert(0, SocketAddr::new(address.ip(), port).to_string());
                }
            }
        }
    }
    result
}
fn bind_discovery() -> Result<UdpSocket> {
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )
    .map_err(|e| e.to_string())?;
    socket.set_reuse_address(true).map_err(|e| e.to_string())?;
    socket.set_broadcast(true).map_err(|e| e.to_string())?;
    socket.set_nonblocking(true).map_err(|e| e.to_string())?;
    socket
        .bind(&SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), DISCOVERY_PORT).into())
        .map_err(|e| format!("Cannot start device discovery: {e}"))?;
    UdpSocket::from_std(socket.into()).map_err(|e| e.to_string())
}
fn load_json<T: serde::de::DeserializeOwned + Default>(path: &Path) -> Result<T> {
    match std::fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|e| format!("Invalid sharing data: {e}"))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e.to_string()),
    }
}
struct PartialGuard {
    map: Arc<std::sync::Mutex<HashSet<String>>>,
    key: String,
}
impl PartialGuard {
    fn acquire(map: Arc<std::sync::Mutex<HashSet<String>>>, key: String) -> Result<Self> {
        if !map
            .lock()
            .map_err(|_| "Partial lock failed")?
            .insert(key.clone())
        {
            return Err("This file is already being received from that device".into());
        }
        Ok(Self { map, key })
    }
}
impl Drop for PartialGuard {
    fn drop(&mut self) {
        if let Ok(mut map) = self.map.lock() {
            map.remove(&self.key);
        }
    }
}
