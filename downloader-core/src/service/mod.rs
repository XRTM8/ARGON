mod executor;
mod handoff;
mod queue;

#[cfg(test)]
mod tests;

pub(crate) use queue::compute_stats;
use crate::{
    disk,
    model::*,
    resolver::{calculate_segments, deduplicate_filename, resolve_universal_link, sanitize_filename},
    throttle::ThrottleController,
    transfer::{build_http_client, TelemetryTracker},
};
use reqwest::Client;
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Instant,
};
use tokio::sync::{watch, Mutex, RwLock};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub enum MediaDownloadUpdate {
    Bytes(u64, u64),
    Finalizing,
}

#[derive(Clone, Debug)]
pub struct ResolvedMediaSpec {
    pub download_url: String,
    pub suggested_filename: Option<String>,
    pub headers: HashMap<String, String>,
}

/// Optional platform media extractor hook (yt-dlp/ffmpeg on desktop) plugged in by host app.
pub trait MediaExtractor: Send + Sync {
    fn supports_hd_mux(&self) -> bool {
        false
    }

    fn resolve_media(
        &self,
        url: String,
        audio_only: bool,
        format_id: Option<String>,
    ) -> BoxFuture<'static, Result<ResolvedMediaSpec, String>>;

    #[allow(clippy::too_many_arguments)]
    fn download_muxed_media(
        &self,
        task_id: String,
        url: String,
        destination_path: PathBuf,
        format_spec: String,
        cancel_flag: Arc<AtomicBool>,
        cancel_rx: watch::Receiver<bool>,
        on_progress: Arc<dyn Fn(MediaDownloadUpdate) + Send + Sync>,
    ) -> BoxFuture<'static, Result<PathBuf, String>>;
}

pub(super) struct ActiveRun {
    run_id: String,
    cancel_flag: Arc<AtomicBool>,
    cancel_tx: watch::Sender<bool>,
    finished: watch::Receiver<()>,
}

pub(super) struct EngineState {
    tasks: HashMap<String, DownloadTask>,
    settings: DownloaderSettings,
    allowed_roots: Vec<PathBuf>,
    speed_history: Vec<f64>,
    retry_deadlines: HashMap<String, Instant>,
    force_start_ids: HashSet<String>,
}

pub struct DownloaderService {
    pub(super) config: Config,
    pub(super) emit: Arc<dyn Fn(DownloadEvent) + Send + Sync>,
    pub(super) media: Option<Arc<dyn MediaExtractor>>,
    pub(super) http_client: Client,
    pub(super) state: RwLock<EngineState>,
    pub(super) active_runs: Mutex<HashMap<String, ActiveRun>>,
    pub(super) telemetry: TelemetryTracker,
    pub(super) throttle: ThrottleController,
    pub(super) memory_budget: Arc<tokio::sync::Semaphore>,
    pub(super) host_limits: Arc<crate::host_limits::HostLimits>,
    #[cfg(not(target_os = "android"))]
    pub(super) torrent_manager: Arc<crate::torrent_local::SharedTorrentManager>,
    pub(super) db: crate::db::StateDb,
    pub(super) persist_lock: Mutex<()>,
    updating: AtomicBool,
    destroyed: AtomicBool,
}

impl DownloaderService {
    pub async fn new(
        config: Config,
        media: Option<Arc<dyn MediaExtractor>>,
        emit: impl Fn(DownloadEvent) + Send + Sync + 'static,
    ) -> Result<Arc<Self>, String> {
        tokio::fs::create_dir_all(&config.state_dir)
            .await
            .map_err(|e| format!("Cannot create downloader state dir: {e}"))?;
        tokio::fs::create_dir_all(&config.default_download_dir)
            .await
            .map_err(|e| format!("Cannot create default download dir: {e}"))?;

        let settings_path = config.state_dir.join("settings.json");
        let mut settings: DownloaderSettings = load_json_opt(&settings_path)?.unwrap_or_default();
        if settings.default_directory.trim().is_empty() || config.is_android {
            settings.default_directory = config.default_download_dir.to_string_lossy().into_owned();
        }
        settings.max_concurrent_downloads = settings.max_concurrent_downloads.clamp(1, 32);
        settings.max_connections_per_host = settings.max_connections_per_host.clamp(1, 16);
        settings.default_segment_count = settings.default_segment_count.clamp(1, 32);

        let db = crate::db::StateDb::open(&config.state_dir)?;
        let mut loaded_tasks: Vec<DownloadTask> = db.load_tasks()?;
        for task in &mut loaded_tasks {
            if task.status == TaskStatus::Downloading {
                task.status = TaskStatus::Paused;
                task.speed_bytes_per_sec = 0.0;
                task.eta_seconds = 0;
                for ch in &mut task.chunks {
                    if ch.status == ChunkStatus::Downloading {
                        ch.status = ChunkStatus::Pending;
                    }
                }
            }
        }
        let tasks_map: HashMap<String, DownloadTask> =
            loaded_tasks.into_iter().map(|t| (t.id.clone(), t)).collect();

        let roots_path = config.state_dir.join("allowed_roots.json");
        let mut allowed_roots: Vec<PathBuf> = load_json_opt(&roots_path)?.unwrap_or_default();
        if let Ok(canon) = tokio::fs::canonicalize(&config.default_download_dir).await {
            if !allowed_roots.iter().any(|r| r == &canon) {
                allowed_roots.push(canon);
            }
        }
        if !settings.default_directory.is_empty() {
            if let Ok(canon) = tokio::fs::canonicalize(&settings.default_directory).await {
                if !allowed_roots.iter().any(|r| r == &canon) {
                    allowed_roots.push(canon);
                }
            }
        }

        let http_client = build_http_client()?;
        let throttle = ThrottleController::default();
        throttle.set_global_limit(settings.global_speed_limit_bytes);

        let torrent_dir = config.state_dir.join("torrents");
        let default_dl_dir = config.default_download_dir.clone();
        let service = Arc::new(Self {
            config,
            emit: Arc::new(emit),
            media,
            http_client,
            state: RwLock::new(EngineState {
                tasks: tasks_map,
                settings,
                allowed_roots,
                speed_history: vec![0.0; 20],
                retry_deadlines: HashMap::new(),
                force_start_ids: HashSet::new(),
            }),
            active_runs: Mutex::new(HashMap::new()),
            telemetry: TelemetryTracker::default(),
            throttle,
            memory_budget: Arc::new(tokio::sync::Semaphore::new(64 * 1024 * 1024)),
            host_limits: Arc::new(crate::host_limits::HostLimits::default()),
            #[cfg(not(target_os = "android"))]
            torrent_manager: Arc::new(crate::torrent_local::SharedTorrentManager::new(
                torrent_dir,
                default_dl_dir,
            )),
            db,
            persist_lock: Mutex::new(()),
            updating: AtomicBool::new(false),
            destroyed: AtomicBool::new(false),
        });

        let svc_loop = service.clone();
        tokio::spawn(async move {
            svc_loop.background_ticker().await;
        });

        Ok(service)
    }

    pub fn http_client(&self) -> &Client {
        &self.http_client
    }

    pub fn telemetry(&self) -> &TelemetryTracker {
        &self.telemetry
    }

    pub fn snapshot_telemetry_binary(&self) -> Vec<u8> {
        self.telemetry.snapshot_telemetry_binary()
    }

    pub fn set_updating(&self, active: bool) {
        self.updating.store(active, Ordering::SeqCst);
    }

    pub fn is_updating(&self) -> bool {
        self.updating.load(Ordering::SeqCst)
    }

    pub async fn has_active_transfers(&self) -> bool {
        let state = self.state.read().await;
        let downloading = state
            .tasks
            .values()
            .any(|t| t.status == TaskStatus::Downloading);
        downloading || !self.active_runs.lock().await.is_empty()
    }

    pub async fn allow_root(&self, dir: PathBuf) {
        if let Ok(canon) = tokio::fs::canonicalize(&dir).await {
            let roots_snapshot = {
                let mut state = self.state.write().await;
                if !state.allowed_roots.iter().any(|r| r == &canon) {
                    state.allowed_roots.push(canon);
                    Some(state.allowed_roots.clone())
                } else {
                    None
                }
            };
            if let Some(roots) = roots_snapshot {
                let _ = self.save_allowed_roots(roots).await;
            }
        }
    }

    pub async fn snapshot(&self) -> DownloaderSnapshot {
        let state = self.state.read().await;
        let mut tasks: Vec<DownloadTask> = state
            .tasks
            .values()
            .filter(|task| !task.awaiting_browser_commit)
            .cloned()
            .collect();
        tasks.sort_by_key(|a| std::cmp::Reverse(a.created_at));
        let stats = compute_stats(&tasks, self.throttle.get_global_limit());
        DownloaderSnapshot {
            tasks,
            stats,
            settings: state.settings.clone(),
            speed_history: state.speed_history.clone(),
            updating: self.is_updating(),
        }
    }

    pub async fn completed_download_path(&self, id: &str) -> Result<PathBuf, String> {
        let (path, size, roots) = {
            let state = self.state.read().await;
            let task = state.tasks.get(id)
                .filter(|task| task.status == TaskStatus::Completed)
                .ok_or("download_file_missing")?;
            let path = task.save_path.as_deref().ok_or("download_file_missing")?;
            (PathBuf::from(path), task.total_bytes, state.allowed_roots.clone())
        };
        let metadata = tokio::fs::symlink_metadata(&path).await.map_err(|_| "download_file_missing")?;
        let canonical = tokio::fs::canonicalize(path).await.map_err(|_| "download_file_missing")?;
        if !metadata.is_file() || metadata.len() != size || disk::is_forbidden_system_path(&canonical)
            || !roots.iter().any(|root| canonical.starts_with(root))
        {
            return Err("download_file_missing".into());
        }
        Ok(canonical)
    }

    pub async fn settings(&self) -> DownloaderSettings {
        self.state.read().await.settings.clone()
    }

    pub async fn update_settings(
        self: &Arc<Self>,
        patch: DownloaderSettingsPatch,
    ) -> Result<DownloaderSettings, String> {
        let new_dir = if let Some(ref dir) = patch.default_directory {
            let trimmed = dir.trim();
            if !trimmed.is_empty() && trimmed != "Downloads" {
                if self.config.is_android {
                    return Err("تغيير مجلد الحفظ غير متاح على Android في هذا الإصدار".into());
                }
                let p = PathBuf::from(trimmed);
                if !p.is_absolute() {
                    return Err("أدخل مسار مجلد كاملًا، مثل C:\\Downloads".into());
                }
                tokio::fs::create_dir_all(&p)
                    .await
                    .map_err(|e| format!("Cannot create directory: {e}"))?;
                let probe = p.join(format!(".argon-write-check-{}", now_ms()));
                let f = tokio::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&probe)
                    .await
                    .map_err(|e| format!("Download directory is not writable: {e}"))?;
                drop(f);
                let _ = tokio::fs::remove_file(&probe).await;
                self.allow_root(p.clone()).await;
                Some(p.to_string_lossy().into_owned())
            } else {
                None
            }
        } else {
            None
        };

        let settings = {
            let mut state = self.state.write().await;
            if let Some(dir) = new_dir {
                state.settings.default_directory = dir;
            }
            if let Some(v) = patch.max_concurrent_downloads {
                state.settings.max_concurrent_downloads = v.clamp(1, 32);
            }
            if let Some(v) = patch.max_connections_per_host {
                state.settings.max_connections_per_host = v.clamp(1, 16);
            }
            if let Some(v) = patch.global_speed_limit_bytes {
                state.settings.global_speed_limit_bytes = v;
                self.throttle.set_global_limit(v);
            }
            if let Some(v) = patch.enable_notifications {
                state.settings.enable_notifications = v;
            }
            if let Some(v) = patch.enable_sound {
                state.settings.enable_sound = v;
            }
            if let Some(v) = patch.sound_volume {
                state.settings.sound_volume = v.clamp(0.0, 1.0);
            }
            if let Some(v) = patch.default_segment_count {
                state.settings.default_segment_count = v.clamp(1, 32);
            }
            if let Some(v) = patch.auto_shutdown_on_complete {
                state.settings.auto_shutdown_on_complete = v;
            }
            if let Some(v) = patch.night_scheduler_enabled {
                state.settings.night_scheduler_enabled = v;
            }
            if let Some(v) = patch.night_scheduler_start_hour {
                state.settings.night_scheduler_start_hour = v.min(23);
            }
            if let Some(v) = patch.night_scheduler_end_hour {
                state.settings.night_scheduler_end_hour = v.min(23);
            }
            if let Some(v) = patch.theme {
                if v == "dark" || v == "light" {
                    state.settings.theme = v;
                }
            }
            if let Some(v) = patch.dynamic_work_stealing {
                state.settings.dynamic_work_stealing = v;
            }
            if let Some(v) = patch.clipboard_monitor_enabled {
                state.settings.clipboard_monitor_enabled = v;
            }
            if let Some(v) = patch.auto_verify_integrity {
                state.settings.auto_verify_integrity = v;
            }
            if let Some(v) = patch.auto_retry_enabled {
                state.settings.auto_retry_enabled = v;
            }
            if let Some(v) = patch.max_auto_retries {
                state.settings.max_auto_retries = v.min(20);
            }
            if let Some(v) = patch.minimize_to_tray {
                state.settings.minimize_to_tray = v;
            }
            if let Some(v) = patch.start_with_windows {
                state.settings.start_with_windows = v;
            }
            if let Some(v) = patch.debrid_enabled {
                state.settings.debrid_enabled = v;
            }
            if let Some(v) = patch.debrid_api_key {
                state.settings.debrid_api_key = Some(v);
            }
            if let Some(v) = patch.completion_action {
                state.settings.completion_action = v;
            }
            if let Some(v) = patch.proxy_mode {
                state.settings.proxy_mode = v;
            }
            if let Some(v) = patch.proxy_url {
                state.settings.proxy_url = if v.trim().is_empty() { None } else { Some(v) };
            }
            if let Some(v) = patch.antivirus_enabled {
                state.settings.antivirus_enabled = v;
            }
            if let Some(v) = patch.antivirus_kind {
                state.settings.antivirus_kind = v;
            }
            if let Some(v) = patch.antivirus_extensions {
                state.settings.antivirus_extensions = v;
            }
            if let Some(v) = patch.category_directories {
                state.settings.category_directories = v;
            }
            state.settings.clone()
        };

        self.save_settings(&settings).await?;
        (self.emit)(DownloadEvent::StateChanged);
        self.process_queue().await;
        Ok(settings)
    }

    pub async fn pause_task(self: &Arc<Self>, id: &str) -> Result<(), String> {
        let changed = {
            let mut state = self.state.write().await;
            state.retry_deadlines.remove(id);
            state.force_start_ids.remove(id);
            let Some(task) = state.tasks.get_mut(id) else {
                return Ok(());
            };
            if !matches!(task.status, TaskStatus::Downloading | TaskStatus::Queued) {
                return Ok(());
            }
            task.status = TaskStatus::Paused;
            task.speed_bytes_per_sec = 0.0;
            task.eta_seconds = 0;
            true
        };
        if changed {
            self.cancel_run(id).await;
            self.save_tasks().await?;
            (self.emit)(DownloadEvent::StateChanged);
            self.process_queue().await;
        }
        Ok(())
    }

    pub async fn resume_task(self: &Arc<Self>, id: &str, force: bool) -> Result<(), String> {
        let changed = {
            let mut state = self.state.write().await;
            let Some(task) = state.tasks.get_mut(id) else {
                return Ok(());
            };
            if !matches!(
                task.status,
                TaskStatus::Paused | TaskStatus::Error | TaskStatus::Queued
            ) || task.awaiting_browser_commit {
                return Ok(());
            }
            task.status = TaskStatus::Queued;
            task.error_message = None;
            task.retry_count = 0;
            state.retry_deadlines.remove(id);
            if force {
                state.force_start_ids.insert(id.to_owned());
            }
            true
        };
        if changed {
            self.save_tasks().await?;
            (self.emit)(DownloadEvent::StateChanged);
            self.process_queue().await;
        }
        Ok(())
    }

    pub async fn restart_task(self: &Arc<Self>, id: &str) -> Result<(), String> {
        self.cancel_run(id).await;
        {
            let mut state = self.state.write().await;
            state.retry_deadlines.remove(id);
            let Some(task) = state.tasks.get_mut(id) else {
                return Ok(());
            };
            task.downloaded_bytes = 0;
            task.resume_notice = None;
            task.completed_at = None;
            task.error_message = None;
            task.probed = false;
            task.retry_count = 0;
            task.active_mirror_index = 0;
            if let Some(ref mut cs) = task.checksum {
                cs.actual = None;
                cs.verified = Some(false);
            }
            task.chunks = calculate_segments(0, 1);
            task.status = TaskStatus::Queued;
            state.force_start_ids.insert(id.to_owned());
        }
        self.save_tasks().await?;
        (self.emit)(DownloadEvent::StateChanged);
        self.process_queue().await;
        Ok(())
    }

    pub async fn refresh_task_url(
        self: &Arc<Self>,
        id: &str,
        new_url: String,
    ) -> Result<(), String> {
        let trimmed = new_url.trim().to_owned();
        let parsed = reqwest::Url::parse(&trimmed).map_err(|_| "Enter a complete HTTP or HTTPS URL.")?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err("Enter a complete HTTP or HTTPS URL.".into());
        }
        let resolved = resolve_universal_link(&trimmed);
        {
            let mut state = self.state.write().await;
            let Some(task) = state.tasks.get_mut(id) else {
                return Err("Task not found".into());
            };
            if !matches!(task.status, TaskStatus::Paused | TaskStatus::Error) {
                return Err("Pause the download before replacing its URL.".into());
            }
            // Keep the partial until the new URL has been probed against its saved identity.
            task.probed = false;
            task.original_url = trimmed;
            task.url = resolved.resolved_url;
            task.active_mirror_index = 0;
            task.media_info = None;
            task.headers = resolved.suggested_headers;
            task.method = None;
            task.post_body = None;
            task.error_message = None;
            task.retry_count = 0;
            if let Some(ref mut cs) = task.checksum {
                cs.actual = None;
                cs.verified = Some(false);
            }
        }
        self.resume_task(id, true).await
    }

    pub async fn pause_all(self: &Arc<Self>) -> Result<(), String> {
        let ids: Vec<String> = {
            let mut state = self.state.write().await;
            let mut list = Vec::new();
            for task in state.tasks.values_mut() {
                if matches!(task.status, TaskStatus::Downloading | TaskStatus::Queued) {
                    task.status = TaskStatus::Paused;
                    task.speed_bytes_per_sec = 0.0;
                    task.eta_seconds = 0;
                    list.push(task.id.clone());
                }
            }
            state.retry_deadlines.clear();
            list
        };
        for id in ids {
            self.cancel_run(&id).await;
        }
        self.save_tasks().await?;
        (self.emit)(DownloadEvent::StateChanged);
        Ok(())
    }

    pub async fn resume_all(self: &Arc<Self>) -> Result<(), String> {
        {
            let mut state = self.state.write().await;
            for task in state.tasks.values_mut() {
                if !task.awaiting_browser_commit && matches!(task.status, TaskStatus::Paused | TaskStatus::Error) {
                    task.status = TaskStatus::Queued;
                    task.error_message = None;
                    task.retry_count = 0;
                }
            }
            state.retry_deadlines.clear();
        }
        self.save_tasks().await?;
        (self.emit)(DownloadEvent::StateChanged);
        self.process_queue().await;
        Ok(())
    }

    pub async fn remove_task(
        self: &Arc<Self>,
        id: &str,
        delete_file_from_disk: bool,
    ) -> Result<(), String> {
        self.cancel_run(id).await;
        let (save_path, allowed_roots) = {
            let mut state = self.state.write().await;
            state.retry_deadlines.remove(id);
            state.force_start_ids.remove(id);
            let sp = state.tasks.get(id).and_then(|t| t.save_path.clone());
            if delete_file_from_disk {
                if let Some(task) = state.tasks.get_mut(id) {
                    if matches!(task.status, TaskStatus::Downloading | TaskStatus::Queued) {
                        task.status = TaskStatus::Paused;
                    }
                }
            }
            (sp, state.allowed_roots.clone())
        };

        if delete_file_from_disk {
            if let Some(path) = save_path {
                if let Err(err) = disk::delete_safe_file(Path::new(&path), &allowed_roots).await {
                    let mut state = self.state.write().await;
                    if let Some(task) = state.tasks.get_mut(id) {
                        task.error_message = Some(format!("Could not delete file: {err}"));
                    }
                    drop(state);
                    self.save_tasks().await?;
                    (self.emit)(DownloadEvent::StateChanged);
                    return Err(err);
                }
            }
        }

        {
            let mut state = self.state.write().await;
            state.tasks.remove(id);
        }
        self.throttle.remove_task(id);
        self.telemetry.clear_task(id).await;
        self.save_tasks().await?;
        (self.emit)(DownloadEvent::StateChanged);
        self.process_queue().await;
        Ok(())
    }

    pub async fn clear_completed(self: &Arc<Self>) -> Result<(), String> {
        let removed_ids: Vec<String> = {
            let mut state = self.state.write().await;
            let ids: Vec<String> = state
                .tasks
                .iter()
                .filter(|(_, t)| t.status == TaskStatus::Completed)
                .map(|(k, _)| k.clone())
                .collect();
            for id in &ids {
                state.tasks.remove(id);
            }
            ids
        };
        for id in removed_ids {
            self.throttle.remove_task(&id);
            self.telemetry.clear_task(&id).await;
        }
        self.save_tasks().await?;
        (self.emit)(DownloadEvent::StateChanged);
        Ok(())
    }

    pub async fn set_task_queue(self: &Arc<Self>, id: &str, queue_id: QueueId) -> Result<(), String> {
        {
            let mut state = self.state.write().await;
            if let Some(t) = state.tasks.get_mut(id) {
                t.queue_id = queue_id;
            }
        }
        self.save_tasks().await?;
        (self.emit)(DownloadEvent::StateChanged);
        self.process_queue().await;
        Ok(())
    }

    pub async fn set_task_priority(self: &Arc<Self>, id: &str, priority: i32) -> Result<(), String> {
        {
            let mut state = self.state.write().await;
            if let Some(t) = state.tasks.get_mut(id) {
                t.queue_priority = priority;
            }
        }
        self.save_tasks().await?;
        (self.emit)(DownloadEvent::StateChanged);
        self.process_queue().await;
        Ok(())
    }

    pub async fn bump_task_queue(self: &Arc<Self>, id: &str, top: bool) -> Result<(), String> {
        {
            let mut state = self.state.write().await;
            if state.tasks.contains_key(id) {
                let target_prio = if top {
                    let max_p = state
                        .tasks
                        .values()
                        .filter(|t| t.status == TaskStatus::Queued)
                        .map(|t| t.queue_priority)
                        .max()
                        .unwrap_or(0);
                    max_p + 10
                } else {
                    let min_p = state
                        .tasks
                        .values()
                        .filter(|t| t.status == TaskStatus::Queued)
                        .map(|t| t.queue_priority)
                        .min()
                        .unwrap_or(0);
                    min_p - 10
                };
                if let Some(t) = state.tasks.get_mut(id) {
                    t.queue_priority = target_prio;
                }
            }
        }
        self.save_tasks().await?;
        (self.emit)(DownloadEvent::StateChanged);
        self.process_queue().await;
        Ok(())
    }

    pub async fn set_task_speed_limit(&self, id: &str, bytes_per_sec: u64) -> Result<(), String> {
        {
            let mut state = self.state.write().await;
            if let Some(t) = state.tasks.get_mut(id) {
                t.speed_limit_bytes_per_sec = bytes_per_sec;
            }
        }
        self.throttle.set_task_limit(id, bytes_per_sec);
        self.save_tasks().await?;
        (self.emit)(DownloadEvent::StateChanged);
        Ok(())
    }

    pub async fn verify_task_hash(self: &Arc<Self>, id: &str) -> Result<Option<String>, String> {
        let save_path = {
            let state = self.state.read().await;
            state
                .tasks
                .get(id)
                .and_then(|t| t.save_path.clone())
        };
        let Some(path) = save_path else {
            return Ok(None);
        };
        let hash = disk::verify_file_sha256(Path::new(&path)).await?;
        {
            let mut state = self.state.write().await;
            if let Some(task) = state.tasks.get_mut(id) {
                let expected = task.checksum.as_ref().and_then(|c| c.expected.clone());
                let verified = expected.as_deref().map(|e| e == hash.as_str());
                task.checksum = Some(ChecksumInfo {
                    algorithm: "sha256".to_owned(),
                    expected,
                    actual: Some(hash.clone()),
                    verified,
                });
            }
        }
        self.save_tasks().await?;
        (self.emit)(DownloadEvent::StateChanged);
        Ok(Some(hash))
    }

    pub(super) fn resolve_task_target_path(&self, task: &DownloadTask, settings: &DownloaderSettings) -> PathBuf {
        if let Some(ref saved) = task.save_path {
            let p = Path::new(saved);
            if self.config.is_android {
                let leaf = p
                    .file_name()
                    .unwrap_or_else(|| std::ffi::OsStr::new(&task.filename));
                return self.config.default_download_dir.join(leaf);
            }
            if p.is_absolute() {
                return p.to_path_buf();
            }
        }
        let cat_key = serde_json::to_value(&task.category)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string));
        let custom_cat_dir = if !self.config.is_android {
            cat_key
                .as_deref()
                .and_then(|k| settings.category_directories.get(k))
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
        } else {
            None
        };
        let base = if let Some(cat_dir) = custom_cat_dir {
            PathBuf::from(cat_dir)
        } else if !self.config.is_android && !settings.default_directory.trim().is_empty() {
            PathBuf::from(&settings.default_directory)
        } else {
            self.config.default_download_dir.clone()
        };
        let target_dir = if self.config.is_android {
            base
        } else {
            match task.category {
                TaskCategory::Programs => base.join("Programs"),
                TaskCategory::Media => base.join("Videos"),
                TaskCategory::Documents => base.join("Documents"),
                TaskCategory::Archives => base.join("Compressed"),
                TaskCategory::Torrents => base.join("Torrents"),
                TaskCategory::Other => base,
            }
        };
        let _ = std::fs::create_dir_all(&target_dir);
        target_dir.join(sanitize_filename(&task.filename))
    }

    pub(super) async fn unique_filename(&self, exclude_id: &str, desired: &str) -> String {
        let state = self.state.read().await;
        let existing: Vec<String> = state
            .tasks
            .values()
            .filter(|t| t.id != exclude_id)
            .map(|t| t.filename.clone())
            .collect();
        deduplicate_filename(desired, &existing)
    }

    pub(super) async fn save_tasks(&self) -> Result<(), String> {
        let _guard = self.persist_lock.lock().await;
        let state = self.state.read().await;
        self.write_tasks(&state).await
    }

    pub(super) async fn write_tasks(&self, state: &EngineState) -> Result<(), String> {
        let mut list: Vec<&DownloadTask> = state.tasks.values().collect();
        list.sort_by_key(|task| std::cmp::Reverse(task.created_at));
        self.db.compact(&list)
    }

    async fn save_settings(&self, settings: &DownloaderSettings) -> Result<(), String> {
        let _guard = self.persist_lock.lock().await;
        let bytes = serde_json::to_vec(settings).map_err(|e| e.to_string())?;
        disk::save_json_atomic(self.config.state_dir.join("settings.json"), bytes).await
    }

    async fn save_allowed_roots(&self, roots: Vec<PathBuf>) -> Result<(), String> {
        let _guard = self.persist_lock.lock().await;
        let bytes = serde_json::to_vec(&roots).map_err(|e| e.to_string())?;
        disk::save_json_atomic(self.config.state_dir.join("allowed_roots.json"), bytes).await
    }

    pub async fn shutdown(&self) {
        self.destroyed.store(true, Ordering::SeqCst);
        let ids: Vec<String> = self.active_runs.lock().await.keys().cloned().collect();
        for id in ids {
            self.cancel_run(&id).await;
        }
        let _ = self.save_tasks().await;
    }
}

fn load_json_opt<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice::<T>(&bytes)
            .map(Some)
            .map_err(|e| format!("Corrupt state file {}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
