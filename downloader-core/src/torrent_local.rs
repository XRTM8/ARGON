#[cfg(not(target_os = "android"))]
mod inner {
    use librqbit::api::TorrentIdOrHash;
    use librqbit::{
        AddTorrent, AddTorrentOptions, AddTorrentResponse, Session, SessionOptions,
        SessionPersistenceConfig, TorrentStatsState,
    };
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::sync::{watch, Mutex};

    pub struct LocalTorrentSession {
        session: Arc<Session>,
    }

    impl LocalTorrentSession {
        pub async fn start(state_dir: &Path, default_output: PathBuf) -> Result<Self, String> {
            std::fs::create_dir_all(state_dir).map_err(|e| e.to_string())?;
            let opts = SessionOptions {
                fastresume: true,
                persistence: Some(SessionPersistenceConfig::Json {
                    folder: Some(state_dir.to_path_buf()),
                }),
                ..Default::default()
            };
            let session = Session::new_with_opts(default_output, opts)
                .await
                .map_err(|e| format!("Failed to start BitTorrent session: {e}"))?;
            Ok(Self { session })
        }

        pub async fn download_magnet<F>(
            &self,
            magnet_uri: &str,
            output_dir: &Path,
            cancel_flag: Arc<AtomicBool>,
            mut cancel_rx: watch::Receiver<bool>,
            mut on_progress: F,
        ) -> Result<(String, PathBuf, u64), String>
        where
            F: FnMut(u64, u64, f64),
        {
            std::fs::create_dir_all(output_dir).map_err(|e| e.to_string())?;
            let opts = AddTorrentOptions {
                overwrite: true,
                output_folder: Some(output_dir.to_string_lossy().into_owned()),
                ..Default::default()
            };
            let resp = self
                .session
                .add_torrent(AddTorrent::from_url(magnet_uri.to_string()), Some(opts))
                .await
                .map_err(|e| format!("Failed to add magnet torrent: {e}"))?;

            let (id, handle) = match resp {
                AddTorrentResponse::Added(id, h) | AddTorrentResponse::AlreadyManaged(id, h) => {
                    (id, h)
                }
                AddTorrentResponse::ListOnly(_) => {
                    return Err("Unexpected list-only response for torrent".into());
                }
            };

            let _ = self.session.unpause(&handle).await;

            loop {
                if cancel_flag.load(Ordering::Acquire) || *cancel_rx.borrow() {
                    let _ = self.session.pause(&handle).await;
                    return Err("CANCELLED".into());
                }

                let stats = handle.stats();
                let total = stats.total_bytes;
                let downloaded = stats.progress_bytes;
                let speed_bps = stats
                    .live
                    .as_ref()
                    .map(|l| l.download_speed.mbps * 125_000.0)
                    .unwrap_or(0.0);

                on_progress(downloaded, total, speed_bps);

                if stats.finished {
                    let name = handle
                        .name()
                        .unwrap_or_else(|| format!("torrent_{id}.bin"));
                    let saved_path = output_dir.join(&name);
                    let _ = self
                        .session
                        .delete(TorrentIdOrHash::Id(id), false)
                        .await;
                    return Ok((name, saved_path, total));
                }

                if matches!(stats.state, TorrentStatsState::Error) {
                    let err = stats
                        .error
                        .unwrap_or_else(|| "Unknown BitTorrent error".into());
                    return Err(err);
                }

                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(500)) => {}
                    _ = crate::throttle::wait_cancelled(&mut cancel_rx, &cancel_flag) => {
                        let _ = self.session.pause(&handle).await;
                        return Err("CANCELLED".into());
                    }
                }
            }
        }
    }

    pub struct SharedTorrentManager {
        state_dir: PathBuf,
        default_output: PathBuf,
        session: Mutex<Option<Arc<LocalTorrentSession>>>,
    }

    impl SharedTorrentManager {
        pub fn new(state_dir: PathBuf, default_output: PathBuf) -> Self {
            Self {
                state_dir,
                default_output,
                session: Mutex::new(None),
            }
        }

        pub async fn session(&self) -> Result<Arc<LocalTorrentSession>, String> {
            let mut guard = self.session.lock().await;
            if let Some(ref s) = *guard {
                return Ok(s.clone());
            }
            let s = Arc::new(
                LocalTorrentSession::start(&self.state_dir, self.default_output.clone()).await?,
            );
            *guard = Some(s.clone());
            Ok(s)
        }
    }
}

#[cfg(not(target_os = "android"))]
pub use inner::{LocalTorrentSession, SharedTorrentManager};
