use crate::{
    disk::{self, CoalescedDiskWriter},
    model::*,
    resolver::{
        calculate_segments, deduplicate_filename, detect_category, detect_platform,
        extract_filename_from_url, resolve_universal_link, same_resource_url, sanitize_filename,
    },
    throttle::ThrottleController,
    transfer::{
        build_http_client, download_segment, is_retryable_network_error, probe_resource,
        verify_destination_size, SegmentOutcome, SegmentTransfer, TelemetryTracker,
    },
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
    time::{Duration, Instant},
};
use tokio::{
    sync::{watch, Mutex, RwLock},
    task::JoinSet,
};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

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

    fn download_muxed_media(
        &self,
        task_id: String,
        url: String,
        destination_path: PathBuf,
        format_spec: String,
        cancel_flag: Arc<AtomicBool>,
        cancel_rx: watch::Receiver<bool>,
        on_progress: Arc<dyn Fn(u64, u64) + Send + Sync>,
    ) -> BoxFuture<'static, Result<PathBuf, String>>;
}

struct ActiveRun {
    run_id: String,
    cancel_flag: Arc<AtomicBool>,
    cancel_tx: watch::Sender<bool>,
}

struct EngineState {
    tasks: HashMap<String, DownloadTask>,
    settings: DownloaderSettings,
    allowed_roots: Vec<PathBuf>,
    speed_history: Vec<f64>,
    retry_deadlines: HashMap<String, Instant>,
    force_start_ids: HashSet<String>,
}

pub struct DownloaderService {
    config: Config,
    emit: Arc<dyn Fn(DownloadEvent) + Send + Sync>,
    media: Option<Arc<dyn MediaExtractor>>,
    http_client: Client,
    state: RwLock<EngineState>,
    active_runs: Mutex<HashMap<String, ActiveRun>>,
    telemetry: TelemetryTracker,
    throttle: ThrottleController,
    persist_lock: Mutex<()>,
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

        let tasks_path = config.state_dir.join("tasks.json");
        let mut loaded_tasks: Vec<DownloadTask> = load_json_opt(&tasks_path)?.unwrap_or_default();
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
        state
            .tasks
            .values()
            .any(|t| t.status == TaskStatus::Downloading)
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
        let mut tasks: Vec<DownloadTask> = state.tasks.values().cloned().collect();
        tasks.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        let stats = compute_stats(&tasks, self.throttle.get_global_limit());
        DownloaderSnapshot {
            tasks,
            stats,
            settings: state.settings.clone(),
            speed_history: state.speed_history.clone(),
            updating: self.is_updating(),
        }
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
            state.settings.clone()
        };

        self.save_settings(&settings).await?;
        (self.emit)(DownloadEvent::StateChanged);
        self.process_queue().await;
        Ok(settings)
    }

    pub async fn add_task(self: &Arc<Self>, req: AddTaskRequest) -> Result<DownloadTask, String> {
        let task = self.insert_task(req, false).await?;
        self.save_tasks().await?;
        (self.emit)(DownloadEvent::StateChanged);
        if task.status == TaskStatus::Queued {
            self.process_queue().await;
        }
        Ok(task)
    }

    pub async fn add_browser_download(
        self: &Arc<Self>,
        payload: BrowserDownloadPayload,
    ) -> Result<DownloadTask, String> {
        let req = AddTaskRequest {
            url: payload.url,
            filename: payload.filename,
            segment_count: None,
            expected_sha256: None,
            headers: payload.headers,
            post_body: payload.post_body,
            method: payload.method,
            media_info: None,
            queue_id: None,
            mirrors: None,
        };
        let task = self.insert_task(req, true).await?;
        // Persist atomically BEFORE returning ok so the browser never cancels on lost memory!
        self.save_tasks().await?;
        (self.emit)(DownloadEvent::StateChanged);
        if task.status == TaskStatus::Queued {
            self.process_queue().await;
        }
        Ok(task)
    }

    pub async fn migrate_legacy_tasks(
        self: &Arc<Self>,
        legacy_tasks: Vec<DownloadTask>,
    ) -> Result<usize, String> {
        if legacy_tasks.is_empty() {
            return Ok(0);
        }
        let added = {
            let mut state = self.state.write().await;
            if !state.tasks.is_empty() {
                return Ok(0);
            }
            let mut count = 0usize;
            for mut t in legacy_tasks {
                if t.status == TaskStatus::Downloading {
                    t.status = TaskStatus::Paused;
                    t.speed_bytes_per_sec = 0.0;
                    t.eta_seconds = 0;
                }
                state.tasks.insert(t.id.clone(), t);
                count += 1;
            }
            count
        };
        if added > 0 {
            self.save_tasks().await?;
            (self.emit)(DownloadEvent::StateChanged);
        }
        Ok(added)
    }

    async fn insert_task(
        &self,
        req: AddTaskRequest,
        reject_invalid: bool,
    ) -> Result<DownloadTask, String> {
        let original_url = req.url.trim().to_owned();
        let resolved = resolve_universal_link(&original_url);
        let url_valid = reqwest::Url::parse(&resolved.resolved_url)
            .ok()
            .is_some_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some());

        let mut error_message = if !url_valid {
            Some("رابط غير صالح؛ أدخل رابط HTTP أو HTTPS كاملًا.".to_owned())
        } else {
            None
        };

        let expected_sha = req.expected_sha256.as_deref().map(str::trim).unwrap_or("");
        let checksum = if !expected_sha.is_empty() {
            let valid = expected_sha.len() == 64
                && expected_sha.bytes().all(|b| b.is_ascii_hexdigit());
            if !valid {
                error_message.get_or_insert_with(|| "بصمة SHA-256 غير صالحة.".to_owned());
                None
            } else {
                Some(ChecksumInfo {
                    algorithm: "sha256".to_owned(),
                    expected: Some(expected_sha.to_ascii_lowercase()),
                    actual: None,
                    verified: Some(false),
                })
            }
        } else {
            None
        };

        if reject_invalid {
            if let Some(err) = error_message {
                return Err(err);
            }
        }

        let mut headers = resolved.suggested_headers;
        if let Some(custom) = req.headers {
            for (k, v) in custom {
                headers.insert(k, v);
            }
        }

        let is_post = req
            .method
            .as_deref()
            .is_some_and(|m| m.eq_ignore_ascii_case("POST"))
            || req.post_body.is_some();

        let state = &mut *self.state.write().await;
        let existing_names: Vec<String> =
            state.tasks.values().map(|t| t.filename.clone()).collect();
        let raw_name = req
            .filename
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .or_else(|| {
                req.media_info
                    .as_ref()
                    .and_then(|m| m.title.as_deref())
                    .filter(|s| !s.trim().is_empty())
            })
            .map(str::to_owned)
            .unwrap_or_else(|| extract_filename_from_url(&resolved.resolved_url));
        let filename = deduplicate_filename(&sanitize_filename(&raw_name), &existing_names);

        let segments = if is_post {
            1
        } else {
            req.segment_count
                .unwrap_or(state.settings.default_segment_count)
                .clamp(1, 32)
        };

        let platform = detect_platform(&original_url);
        let queue_id = req.queue_id.unwrap_or_else(|| {
            if !["direct", "other"].contains(&platform) {
                QueueId::Media
            } else {
                QueueId::Main
            }
        });

        let id = new_id("task");
        let task = DownloadTask {
            id: id.clone(),
            url: resolved.resolved_url,
            original_url,
            filename: filename.clone(),
            total_bytes: 0,
            downloaded_bytes: 0,
            status: if error_message.is_some() {
                TaskStatus::Error
            } else {
                TaskStatus::Queued
            },
            speed_bytes_per_sec: 0.0,
            eta_seconds: 0,
            chunks: calculate_segments(0, 1),
            created_at: now_ms(),
            completed_at: None,
            error_message,
            checksum,
            save_path: None,
            headers,
            post_body: req.post_body,
            method: req.method,
            category: if !["direct", "other"].contains(&platform) {
                TaskCategory::Media
            } else {
                detect_category(&filename)
            },
            media_info: req.media_info,
            requested_segments: segments,
            custom_filename: req
                .filename
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty()),
            file_prepared: false,
            accept_ranges: false,
            etag: None,
            last_modified: None,
            probed: false,
            queue_id,
            speed_limit_bytes_per_sec: 0,
            mirrors: req.mirrors.unwrap_or_default(),
            active_mirror_index: 0,
            retry_count: 0,
        };
        state.tasks.insert(id, task.clone());
        Ok(task)
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
            ) {
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
            // Retain existing chunks and progress; re-probe will verify content length
            task.probed = false;
            task.original_url = trimmed;
            task.url = resolved.resolved_url;
            task.media_info = None;
            task.headers = resolved.suggested_headers;
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
                if matches!(task.status, TaskStatus::Paused | TaskStatus::Error) {
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

    pub async fn process_queue(self: &Arc<Self>) {
        if self.destroyed.load(Ordering::Relaxed) || self.is_updating() {
            return;
        }
        let to_start: Vec<(DownloadTask, DownloaderSettings)> = {
            let mut state = self.state.write().await;
            let settings = state.settings.clone();
            let now_inst = Instant::now();
            let in_window = is_within_night_window(
                settings.night_scheduler_start_hour,
                settings.night_scheduler_end_hour,
            );

            let mut active_total = 0usize;
            let mut active_per_host: HashMap<String, usize> = HashMap::new();
            for t in state.tasks.values() {
                if t.status == TaskStatus::Downloading {
                    active_total += 1;
                    if let Some(h) = url_host(&t.url) {
                        *active_per_host.entry(h).or_insert(0) += 1;
                    }
                }
            }

            let mut queued: Vec<DownloadTask> = state
                .tasks
                .values()
                .filter(|t| t.status == TaskStatus::Queued)
                .cloned()
                .collect();
            queued.sort_by_key(|t| t.created_at);

            let mut picked = Vec::new();
            for candidate in queued {
                if active_total >= settings.max_concurrent_downloads {
                    break;
                }
                if let Some(deadline) = state.retry_deadlines.get(&candidate.id) {
                    if *deadline > now_inst {
                        continue;
                    }
                }
                let forced = state.force_start_ids.contains(&candidate.id);
                if !forced {
                    if (settings.night_scheduler_enabled || candidate.queue_id == QueueId::Night)
                        && !in_window
                    {
                        continue;
                    }
                }
                let host = url_host(&candidate.url);
                if let Some(ref h) = host {
                    let count = active_per_host.get(h).copied().unwrap_or(0);
                    if count >= settings.max_connections_per_host {
                        continue;
                    }
                    *active_per_host.entry(h.clone()).or_insert(0) += 1;
                }
                if let Some(t) = state.tasks.get_mut(&candidate.id) {
                    t.status = TaskStatus::Downloading;
                    t.error_message = None;
                }
                state.retry_deadlines.remove(&candidate.id);
                state.force_start_ids.remove(&candidate.id);
                active_total += 1;
                picked.push((candidate, settings.clone()));
            }
            picked
        };

        if to_start.is_empty() {
            return;
        }
        let _ = self.save_tasks().await;
        (self.emit)(DownloadEvent::StateChanged);

        for (task, settings) in to_start {
            self.spawn_task_execution(task, settings).await;
        }
    }

    fn spawn_task_execution<'a>(
        self: &'a Arc<Self>,
        task: DownloadTask,
        settings: DownloaderSettings,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let id = task.id.clone();
            self.cancel_run(&id).await;

            let run_id = uuid::Uuid::new_v4().to_string();
            let cancel_flag = Arc::new(AtomicBool::new(false));
            let (cancel_tx, cancel_rx) = watch::channel(false);

            {
                let mut runs = self.active_runs.lock().await;
                runs.insert(
                    id.clone(),
                    ActiveRun {
                        run_id: run_id.clone(),
                        cancel_flag: cancel_flag.clone(),
                        cancel_tx,
                    },
                );
            }

            let service = self.clone();
            tokio::spawn(async move {
                let outcome = service
                    .execute_download(task, settings, &run_id, cancel_flag.clone(), cancel_rx)
                    .await;
                service.finalize_run(&id, &run_id, outcome).await;
            });
        })
    }

    async fn cancel_run(&self, id: &str) {
        let old = self.active_runs.lock().await.remove(id);
        if let Some(run) = old {
            run.cancel_flag.store(true, Ordering::Release);
            let _ = run.cancel_tx.send(true);
        }
    }

    async fn is_current_run(&self, id: &str, run_id: &str, cancel_flag: &AtomicBool) -> bool {
        if cancel_flag.load(Ordering::Acquire) || self.destroyed.load(Ordering::Relaxed) {
            return false;
        }
        self.active_runs
            .lock()
            .await
            .get(id)
            .is_some_and(|r| r.run_id == run_id)
    }

    async fn execute_download(
        self: &Arc<Self>,
        mut task: DownloadTask,
        settings: DownloaderSettings,
        run_id: &str,
        cancel_flag: Arc<AtomicBool>,
        cancel_rx: watch::Receiver<bool>,
    ) -> Result<DownloadTask, String> {
        let task_id = task.id.clone();
        let pacer = self
            .throttle
            .get_or_create_task_pacer(&task_id, task.speed_limit_bytes_per_sec);

        // 1. Optional platform media resolution / muxing
        let platform = detect_platform(&task.original_url);
        if !["direct", "other"].contains(&platform) {
            if let Some(ref media_hook) = self.media {
                let explicit_mux = task
                    .media_info
                    .as_ref()
                    .is_some_and(|m| m.use_ytdlp_mux == Some(true) && m.selected_format_id.is_some());
                let audio_only = task
                    .media_info
                    .as_ref()
                    .and_then(|m| m.is_audio_only)
                    .unwrap_or(false);
                let auto_hd_mux = !explicit_mux
                    && !audio_only
                    && task
                        .media_info
                        .as_ref()
                        .and_then(|m| m.selected_format_id.as_ref())
                        .is_none()
                    && platform == "youtube"
                    && media_hook.supports_hd_mux();

                if explicit_mux || auto_hd_mux {
                    let fmt = if explicit_mux {
                        task.media_info
                            .as_ref()
                            .and_then(|m| m.selected_format_id.clone())
                            .unwrap_or_default()
                    } else {
                        "bestvideo[ext=mp4]+bestaudio[ext=m4a]/bestvideo+bestaudio/best[ext=mp4]/best"
                            .to_owned()
                    };
                    if auto_hd_mux && !task.custom_filename && !task.file_prepared {
                        if let Ok(meta) = crate::social::resolve_social_media(
                            &self.http_client,
                            &task.original_url,
                            false,
                            None,
                        )
                        .await
                        {
                            task.filename =
                                self.unique_filename(&task_id, &meta.suggested_filename).await;
                            task.category = detect_category(&task.filename);
                            self.sync_task_metadata(&task).await;
                        }
                    }
                    let dest = self.resolve_task_target_path(&task, &settings);
                    self.telemetry.init_task(&task_id, task.total_bytes, &[(0, 0)]);
                    let svc_prog = self.clone();
                    let tid = task_id.clone();
                    let saved = media_hook
                        .download_muxed_media(
                            task_id.clone(),
                            task.original_url.clone(),
                            dest,
                            fmt,
                            cancel_flag.clone(),
                            cancel_rx,
                            Arc::new(move |dl, total| {
                                svc_prog.telemetry.record_segment_progress(&tid, 0, dl, total);
                            }),
                        )
                        .await?;
                    if !self.is_current_run(&task_id, run_id, &cancel_flag).await {
                        return Err("CANCELLED".into());
                    }
                    let size = tokio::fs::metadata(&saved).await.map(|m| m.len()).unwrap_or(0);
                    task.save_path = Some(saved.to_string_lossy().into_owned());
                    task.filename = saved
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned();
                    task.category = detect_category(&task.filename);
                    task.file_prepared = true;
                    task.total_bytes = size;
                    task.downloaded_bytes = size;
                    task.status = TaskStatus::Completed;
                    task.completed_at = Some(now_ms());
                    return Ok(task);
                }

                let fmt_id = task
                    .media_info
                    .as_ref()
                    .and_then(|m| m.selected_format_id.clone());
                let resolved = media_hook
                    .resolve_media(task.original_url.clone(), audio_only, fmt_id)
                    .await?;
                if !self.is_current_run(&task_id, run_id, &cancel_flag).await {
                    return Err("CANCELLED".into());
                }
                task.url = resolved.download_url;
                for (k, v) in resolved.headers {
                    task.headers.insert(k, v);
                }
                if !task.custom_filename && !task.file_prepared {
                    if let Some(name) = resolved.suggested_filename {
                        task.filename = self.unique_filename(&task_id, &name).await;
                        task.category = detect_category(&task.filename);
                    }
                }
            } else {
                let audio_only = task
                    .media_info
                    .as_ref()
                    .and_then(|m| m.is_audio_only)
                    .unwrap_or(false);
                let fmt_id = task
                    .media_info
                    .as_ref()
                    .and_then(|m| m.selected_format_id.as_deref());
                let resolved = crate::social::resolve_social_media(
                    &self.http_client,
                    &task.original_url,
                    audio_only,
                    fmt_id,
                )
                .await?;
                if !self.is_current_run(&task_id, run_id, &cancel_flag).await {
                    return Err("CANCELLED".into());
                }
                task.url = resolved.download_url;
                for (k, v) in resolved.headers {
                    task.headers.insert(k, v);
                }
                if !task.custom_filename && !task.file_prepared {
                    task.filename = self
                        .unique_filename(&task_id, &resolved.suggested_filename)
                        .await;
                    task.category = detect_category(&task.filename);
                }
            }
        }

        // 2. Select active URL (primary or mirror fallback)
        let effective_url = if task.active_mirror_index > 0 {
            task.mirrors
                .get(task.active_mirror_index - 1)
                .cloned()
                .unwrap_or_else(|| task.url.clone())
        } else {
            task.url.clone()
        };

        let is_post = task
            .method
            .as_deref()
            .is_some_and(|m| m.eq_ignore_ascii_case("POST"))
            || task.post_body.is_some();

        // 3. Probe resource (skip range probe for POST requests)
        let probe = if is_post {
            UrlProbeResult {
                content_length: None,
                accept_ranges: false,
                content_type: None,
                suggested_filename: None,
                etag: None,
                last_modified: None,
                final_url: None,
            }
        } else {
            let mut probe_cancel_rx = cancel_rx.clone();
            let p = tokio::select! {
                res = probe_resource(&self.http_client, &effective_url, Some(&task.headers)) => res?,
                _ = crate::throttle::wait_cancelled(&mut probe_cancel_rx, &cancel_flag) => {
                    return Err("CANCELLED".into());
                }
            };
            if let Some(ref ct) = p.content_type {
                let lower = ct.to_ascii_lowercase();
                if lower.starts_with("text/html")
                    || lower.starts_with("application/xhtml+xml")
                    || lower.starts_with("application/vnd.apple.mpegurl")
                    || lower.starts_with("application/dash+xml")
                    || lower.starts_with("audio/x-mpegurl")
                {
                    return Err(
                        "الرابط يعيد صفحة ويب أو قائمة بث، وليس ملفًا مباشرًا قابلًا للتنزيل."
                            .into(),
                    );
                }
            }
            p
        };

        if !self.is_current_run(&task_id, run_id, &cancel_flag).await {
            return Err("CANCELLED".into());
        }

        let probe_len = probe.content_length.unwrap_or(0);
        let same_resource = !is_post
            && probe.accept_ranges
            && (
                // When refreshing an expired URL or resuming: if Content-Length matches and server accepts ranges, resume cleanly
                (task.total_bytes > 0 && task.total_bytes == probe_len)
                || (task.etag.is_some() && probe.etag.as_ref() == task.etag.as_ref())
                || (task.last_modified.is_some() && task.last_modified == probe.last_modified)
            );

        let resume = task.downloaded_bytes > 0 && same_resource;
        if !resume {
            task.downloaded_bytes = 0;
            let segs = if probe.accept_ranges && !is_post {
                task.requested_segments
            } else {
                1
            };
            task.chunks = calculate_segments(probe_len, segs);
        }

        task.total_bytes = probe_len;
        task.accept_ranges = probe.accept_ranges && !is_post;
        task.etag = probe.etag.clone();
        task.last_modified = probe.last_modified.clone();
        task.probed = true;
        if !task.custom_filename && !task.file_prepared {
            if let Some(ref suggested) = probe.suggested_filename {
                task.filename = self.unique_filename(&task_id, suggested).await;
            }
        }
        task.category = detect_category(&task.filename);

        let initial_target = self.resolve_task_target_path(&task, &settings);
        let final_path = disk::prepare_destination_file(
            &initial_target,
            task.total_bytes,
            !task.file_prepared,
            !resume,
        )
        .await?;
        if let Some(parent) = final_path.parent() {
            self.allow_root(parent.to_path_buf()).await;
        }
        task.file_prepared = true;
        task.save_path = Some(final_path.to_string_lossy().into_owned());
        task.filename = final_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();

        self.sync_task_metadata(&task).await;

        let initial_pairs: Vec<(u32, u64)> =
            task.chunks.iter().map(|c| (c.index, c.downloaded)).collect();
        self.telemetry
            .init_task(&task_id, task.total_bytes, &initial_pairs);

        // 4. Execute transfer (Ranged parallel with work-stealing, falling back to single stream on RANGE_UNSUPPORTED)
        let transfer_url = probe
            .final_url
            .filter(|u| same_resource_url(u, &effective_url))
            .unwrap_or(effective_url);

        let use_range = task.accept_ranges && task.total_bytes > 0;
        if use_range {
            match self
                .run_chunk_pool(
                    &mut task,
                    &transfer_url,
                    &final_path,
                    true,
                    probe.content_length,
                    settings.dynamic_work_stealing,
                    pacer.clone(),
                    cancel_flag.clone(),
                    cancel_rx.clone(),
                )
                .await
            {
                Ok(()) => {}
                Err(e) if e.contains("RANGE_UNSUPPORTED") => {
                    if !self.is_current_run(&task_id, run_id, &cancel_flag).await {
                        return Err("CANCELLED".into());
                    }
                    task.accept_ranges = false;
                    task.downloaded_bytes = 0;
                    task.chunks = calculate_segments(0, 1);
                    self.sync_task_metadata(&task).await;
                    self.run_chunk_pool(
                        &mut task,
                        &transfer_url,
                        &final_path,
                        false,
                        probe.content_length,
                        false,
                        pacer,
                        cancel_flag.clone(),
                        cancel_rx,
                    )
                    .await?;
                }
                Err(e) => return Err(e),
            }
        } else {
            self.run_chunk_pool(
                &mut task,
                &transfer_url,
                &final_path,
                false,
                probe.content_length,
                false,
                pacer,
                cancel_flag.clone(),
                cancel_rx,
            )
            .await?;
        }

        if !self.is_current_run(&task_id, run_id, &cancel_flag).await {
            return Err("CANCELLED".into());
        }

        task.downloaded_bytes = task.chunks.iter().map(|c| c.downloaded).sum();
        if task.downloaded_bytes != task.total_bytes {
            return Err("الملف غير مكتمل: عدد البايتات المستلمة لا يطابق الحجم المتوقع.".into());
        }
        verify_destination_size(&final_path, task.total_bytes).await?;

        // 5. SHA-256 verification if expected or auto-verify enabled
        if let Some(ref mut cs) = task.checksum {
            if let Some(ref expected) = cs.expected {
                let actual = disk::verify_file_sha256(&final_path).await?;
                if !self.is_current_run(&task_id, run_id, &cancel_flag).await {
                    return Err("CANCELLED".into());
                }
                cs.actual = Some(actual.clone());
                cs.verified = Some(&actual == expected);
                if cs.verified != Some(true) {
                    return Err("فشل التحقق من SHA-256؛ الملف لا يطابق البصمة المطلوبة.".into());
                }
            }
        } else if settings.auto_verify_integrity {
            if let Ok(actual) = disk::verify_file_sha256(&final_path).await {
                task.checksum = Some(ChecksumInfo {
                    algorithm: "sha256".to_owned(),
                    expected: None,
                    actual: Some(actual),
                    verified: None,
                });
            }
        }

        task.status = TaskStatus::Completed;
        task.speed_bytes_per_sec = 0.0;
        task.eta_seconds = 0;
        task.completed_at = Some(now_ms());
        Ok(task)
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_chunk_pool(
        self: &Arc<Self>,
        task: &mut DownloadTask,
        url: &str,
        final_path: &Path,
        use_range: bool,
        expected_total: Option<u64>,
        dynamic_work_stealing: bool,
        pacer: Arc<crate::throttle::TaskPacer>,
        cancel_flag: Arc<AtomicBool>,
        cancel_rx: watch::Receiver<bool>,
    ) -> Result<(), String> {
        let disk = CoalescedDiskWriter::open(final_path)?;
        let validator = task
            .etag
            .clone()
            .or_else(|| task.last_modified.clone());

        // Attempt-local cancel token so one failing sibling aborts the rest cleanly on RANGE_UNSUPPORTED/error.
        let attempt_flag = Arc::new(AtomicBool::new(false));
        let (attempt_tx, _) = watch::channel(false);

        let parent_flag = cancel_flag.clone();
        let mut parent_rx = cancel_rx.clone();
        let af = attempt_flag.clone();
        let atx = attempt_tx.clone();
        let forwarder = tokio::spawn(async move {
            crate::throttle::wait_cancelled(&mut parent_rx, &parent_flag).await;
            af.store(true, Ordering::Release);
            let _ = atx.send(true);
        });

        let mut join_set: JoinSet<Result<SegmentOutcome, String>> = JoinSet::new();
        let mut next_chunk_index = task.chunks.iter().map(|c| c.index + 1).max().unwrap_or(0);

        let spawn_chunk = |join_set: &mut JoinSet<Result<SegmentOutcome, String>>,
                           chunk: &DownloadChunk| {
            let service = self.clone();
            let client = self.http_client.clone();
            let disk = disk.clone();
            let pacer = pacer.clone();
            let af = attempt_flag.clone();
            let arx = attempt_tx.subscribe();
            let task_id = task.id.clone();
            let url = url.to_owned();
            let validator = validator.clone();
            let headers = task.headers.clone();
            let method = task.method.clone();
            let post_body = task.post_body.clone();
            let chunk = chunk.clone();
            join_set.spawn(async move {
                download_segment(
                    SegmentTransfer {
                        task_id: &task_id,
                        url: &url,
                        start: chunk.start_byte,
                        end: chunk.end_byte,
                        downloaded: chunk.downloaded,
                        segment_index: chunk.index,
                        use_range,
                        expected_total,
                        validator: validator.as_deref(),
                        custom_headers: Some(&headers),
                        method: method.as_deref(),
                        post_body: post_body.as_deref(),
                    },
                    &client,
                    disk,
                    &service.telemetry,
                    &service.throttle,
                    Some(pacer),
                    af,
                    arx,
                    |seg_idx, dl, total, finished| {
                        service.update_chunk_progress_nonblocking(
                            &task_id, seg_idx, dl, total, finished,
                        );
                    },
                )
                .await
            });
        };

        for chunk in &mut task.chunks {
            if chunk.status != ChunkStatus::Completed {
                chunk.status = ChunkStatus::Downloading;
                spawn_chunk(&mut join_set, chunk);
            }
        }

        let mut errors: Vec<String> = Vec::new();
        while let Some(joined) = join_set.join_next().await {
            match joined {
                Ok(Ok(outcome)) => {
                    if let Some(c) = task
                        .chunks
                        .iter_mut()
                        .find(|c| c.index == outcome.segment_index)
                    {
                        c.downloaded = outcome.downloaded_bytes;
                        c.end_byte = outcome.end_byte;
                        c.status = ChunkStatus::Completed;
                    }
                    if !use_range {
                        task.total_bytes = outcome.total_bytes;
                    }
                    if use_range && dynamic_work_stealing && errors.is_empty() && !attempt_flag.load(Ordering::Acquire) {
                        if let Some((victim_idx, victim_new_end, stolen_start, stolen_end)) =
                            self.telemetry
                                .steal_largest_segment(&task.id, outcome.segment_index, 524_288)
                                .await
                        {
                            if let Some(v) = task.chunks.iter_mut().find(|c| c.index == victim_idx) {
                                v.end_byte = victim_new_end;
                            }
                            let stolen = DownloadChunk {
                                index: next_chunk_index,
                                start_byte: stolen_start,
                                end_byte: stolen_end,
                                downloaded: 0,
                                status: ChunkStatus::Downloading,
                                speed_bytes_per_sec: None,
                            };
                            next_chunk_index += 1;
                            task.chunks.push(stolen.clone());
                            self.sync_task_metadata(task).await;
                            spawn_chunk(&mut join_set, &stolen);
                        }
                    }
                }
                Ok(Err(err)) => {
                    errors.push(err);
                    attempt_flag.store(true, Ordering::Release);
                    let _ = attempt_tx.send(true);
                }
                Err(join_err) => {
                    errors.push(join_err.to_string());
                    attempt_flag.store(true, Ordering::Release);
                    let _ = attempt_tx.send(true);
                }
            }
        }

        forwarder.abort();

        // Always sync flushed bytes on disk and persist latest chunk progress before returning.
        if errors.is_empty() {
            disk.sync().await?;
            Ok(())
        } else {
            let _ = disk.flush().await;
            self.sync_chunks_from_telemetry(&mut task.chunks, &task.id);
            {
                let mut state = self.state.write().await;
                if let Some(t) = state.tasks.get_mut(&task.id) {
                    t.chunks = task.chunks.clone();
                    t.downloaded_bytes = t.chunks.iter().map(|c| c.downloaded).sum();
                }
            }
            let cause = errors
                .iter()
                .find(|e| e.contains("RANGE_UNSUPPORTED"))
                .or_else(|| errors.iter().find(|e| !e.contains("CANCELLED")))
                .unwrap_or(&errors[0])
                .clone();
            Err(cause)
        }
    }

    fn sync_chunks_from_telemetry(&self, chunks: &mut [DownloadChunk], task_id: &str) {
        let segs = self.telemetry.segment_progress_map(task_id);
        for chunk in chunks.iter_mut() {
            if let Some(&dl) = segs.get(&chunk.index) {
                chunk.downloaded = dl.max(chunk.downloaded);
            }
            if chunk.status == ChunkStatus::Downloading {
                chunk.status = ChunkStatus::Pending;
            }
        }
    }

    fn update_chunk_progress_nonblocking(
        &self,
        task_id: &str,
        segment_index: u32,
        downloaded: u64,
        total_bytes: u64,
        finished: bool,
    ) {
        if let Ok(mut state) = self.state.try_write() {
            if let Some(task) = state.tasks.get_mut(task_id) {
                if task.status != TaskStatus::Downloading {
                    return;
                }
                if let Some(c) = task.chunks.iter_mut().find(|c| c.index == segment_index) {
                    c.downloaded = downloaded;
                    c.status = if finished {
                        ChunkStatus::Completed
                    } else {
                        ChunkStatus::Downloading
                    };
                }
                if total_bytes > 0 {
                    task.total_bytes = total_bytes;
                }
                task.downloaded_bytes = task.chunks.iter().map(|c| c.downloaded).sum();
            }
        }
    }

    async fn sync_task_metadata(&self, snapshot_task: &DownloadTask) {
        {
            let mut state = self.state.write().await;
            if let Some(t) = state.tasks.get_mut(&snapshot_task.id) {
                t.url = snapshot_task.url.clone();
                t.filename = snapshot_task.filename.clone();
                t.save_path = snapshot_task.save_path.clone();
                t.total_bytes = snapshot_task.total_bytes;
                t.downloaded_bytes = snapshot_task.downloaded_bytes;
                t.chunks = snapshot_task.chunks.clone();
                t.accept_ranges = snapshot_task.accept_ranges;
                t.etag = snapshot_task.etag.clone();
                t.last_modified = snapshot_task.last_modified.clone();
                t.probed = snapshot_task.probed;
                t.file_prepared = snapshot_task.file_prepared;
                t.category = snapshot_task.category.clone();
                t.headers = snapshot_task.headers.clone();
            }
        }
        let _ = self.save_tasks().await;
        (self.emit)(DownloadEvent::StateChanged);
    }

    async fn finalize_run(
        self: &Arc<Self>,
        id: &str,
        run_id: &str,
        outcome: Result<DownloadTask, String>,
    ) {
        let still_current = {
            let mut runs = self.active_runs.lock().await;
            if runs.get(id).is_some_and(|r| r.run_id == run_id) {
                runs.remove(id);
                true
            } else {
                false
            }
        };
        if !still_current {
            return;
        }
        self.telemetry.clear_task(id).await;

        let mut completed_event: Option<DownloadEvent> = None;
        let mut failed_event: Option<DownloadEvent> = None;
        let mut trigger_shutdown = false;

        {
            let mut state = self.state.write().await;
            let settings = state.settings.clone();
            if let Some(task) = state.tasks.get_mut(id) {
                task.speed_bytes_per_sec = 0.0;
                task.eta_seconds = 0;
                if task.status == TaskStatus::Downloading {
                    match outcome {
                        Ok(done) => {
                            let fname = done.filename.clone();
                            let spath = done.save_path.clone().unwrap_or_default();
                            *task = done;
                            completed_event = Some(DownloadEvent::TaskCompleted {
                                task_id: id.to_owned(),
                                filename: fname,
                                save_path: spath,
                            });
                            if settings.auto_shutdown_on_complete
                                && !self.is_updating()
                                && state
                                    .tasks
                                    .values()
                                    .all(|t| t.status == TaskStatus::Completed)
                            {
                                trigger_shutdown = true;
                            }
                        }
                        Err(err) => {
                            if err == "CANCELLED" {
                                // Paused/removed already updated status; keep paused if still Downloading.
                                task.status = TaskStatus::Paused;
                            } else if task.active_mirror_index < task.mirrors.len() {
                                // Immediate mirror fallback!
                                task.active_mirror_index += 1;
                                task.status = TaskStatus::Queued;
                                task.probed = false;
                                task.error_message = None;
                            } else if settings.auto_retry_enabled
                                && task.retry_count < settings.max_auto_retries
                                && is_retryable_network_error(&err)
                            {
                                task.retry_count += 1;
                                let delay_secs = (2u64 << (task.retry_count - 1).min(4)).min(30);
                                task.status = TaskStatus::Queued;
                                task.error_message = Some(format!(
                                    "إعادة محاولة تلقائية ({}/{}) خلال {delay_secs}ث: {err}",
                                    task.retry_count, settings.max_auto_retries
                                ));
                                state.retry_deadlines.insert(
                                    id.to_owned(),
                                    Instant::now() + Duration::from_secs(delay_secs),
                                );
                            } else {
                                let fname = task.filename.clone();
                                task.status = TaskStatus::Error;
                                task.error_message = Some(err.clone());
                                failed_event = Some(DownloadEvent::TaskFailed {
                                    task_id: id.to_owned(),
                                    filename: fname,
                                    error: err,
                                });
                            }
                        }
                    }
                }
            }
        }

        let _ = self.save_tasks().await;
        if let Some(ev) = completed_event {
            (self.emit)(ev);
        }
        if let Some(ev) = failed_event {
            (self.emit)(ev);
        }
        if trigger_shutdown {
            (self.emit)(DownloadEvent::AllIdleShutdownPending);
        }
        (self.emit)(DownloadEvent::StateChanged);
        let svc = self.clone();
        tokio::spawn(async move {
            svc.process_queue().await;
        });
    }

    async fn background_ticker(self: &Arc<Self>) {
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        loop {
            tick.tick().await;
            if self.destroyed.load(Ordering::Relaxed) {
                break;
            }
            let mut needs_queue = false;
            let mut active_changed = false;
            let mut pause_outside_window = Vec::new();
            {
                let mut state = self.state.write().await;
                let now_inst = Instant::now();
                let settings = state.settings.clone();
                let has_ready_retry = state.tasks.values().any(|t| {
                    t.status == TaskStatus::Queued
                        && state
                            .retry_deadlines
                            .get(&t.id)
                            .is_none_or(|d| *d <= now_inst)
                });
                let in_night_window = is_within_night_window(
                    settings.night_scheduler_start_hour,
                    settings.night_scheduler_end_hour,
                );
                let has_queued = state.tasks.values().any(|t| t.status == TaskStatus::Queued);
                if has_queued
                    && (!settings.night_scheduler_enabled || in_night_window)
                    && (has_ready_retry || state.tasks.values().filter(|t| t.status == TaskStatus::Downloading).count() < settings.max_concurrent_downloads)
                {
                    needs_queue = true;
                }

                if settings.night_scheduler_enabled && !in_night_window {
                    let force_ids = state.force_start_ids.clone();
                    for task in state.tasks.values_mut() {
                        if task.status == TaskStatus::Downloading && !force_ids.contains(&task.id) {
                            task.status = TaskStatus::Queued;
                            task.speed_bytes_per_sec = 0.0;
                            task.eta_seconds = 0;
                            pause_outside_window.push(task.id.clone());
                            active_changed = true;
                        }
                    }
                }

                let mut total_speed = 0.0f64;
                for task in state.tasks.values_mut() {
                    if task.status == TaskStatus::Downloading {
                        if let Some((dl, total, speed)) =
                            self.telemetry.effective_task_speed(&task.id)
                        {
                            task.downloaded_bytes = dl;
                            if total > 0 {
                                task.total_bytes = total;
                            }
                            task.speed_bytes_per_sec = speed;
                            task.eta_seconds = if task.total_bytes > 0 && speed > 0.0 {
                                ((task.total_bytes.saturating_sub(dl)) as f64 / speed).ceil()
                                    as u64
                            } else {
                                0
                            };
                            total_speed += speed;
                            active_changed = true;
                        }
                    }
                }
                if active_changed || state.speed_history.last().copied().unwrap_or(0.0) > 0.0 {
                    state.speed_history.push(total_speed);
                    if state.speed_history.len() > 20 {
                        state.speed_history.remove(0);
                    }
                }
            }

            for tid in pause_outside_window {
                self.cancel_run(&tid).await;
            }

            if active_changed {
                (self.emit)(DownloadEvent::StateChanged);
            }
            if needs_queue {
                self.process_queue().await;
            }
        }
    }

    fn resolve_task_target_path(&self, task: &DownloadTask, settings: &DownloaderSettings) -> PathBuf {
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
        let base = if !self.config.is_android && !settings.default_directory.trim().is_empty() {
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

    async fn unique_filename(&self, exclude_id: &str, desired: &str) -> String {
        let state = self.state.read().await;
        let existing: Vec<String> = state
            .tasks
            .values()
            .filter(|t| t.id != exclude_id)
            .map(|t| t.filename.clone())
            .collect();
        deduplicate_filename(desired, &existing)
    }

    async fn save_tasks(&self) -> Result<(), String> {
        let _guard = self.persist_lock.lock().await;
        let bytes = {
            let state = self.state.read().await;
            let mut list: Vec<&DownloadTask> = state.tasks.values().collect();
            list.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            serde_json::to_vec(&list).map_err(|e| e.to_string())?
        };
        disk::save_json_atomic(self.config.state_dir.join("tasks.json"), bytes).await
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

fn url_host(raw: &str) -> Option<String> {
    reqwest::Url::parse(raw)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_ascii_lowercase()))
}

pub fn is_within_night_window(start_hour: u8, end_hour: u8) -> bool {
    let hour = disk::local_hour();
    if start_hour <= end_hour {
        hour >= start_hour && hour < end_hour
    } else {
        hour >= start_hour || hour < end_hour
    }
}

fn compute_stats(tasks: &[DownloadTask], speed_limit: u64) -> DownloadStats {
    let mut active_count = 0;
    let mut queued_count = 0;
    let mut completed_count = 0;
    let mut total_bytes_downloaded = 0;
    let mut current_total_speed = 0.0;

    for t in tasks {
        match t.status {
            TaskStatus::Downloading => {
                active_count += 1;
                current_total_speed += t.speed_bytes_per_sec;
            }
            TaskStatus::Queued => queued_count += 1,
            TaskStatus::Completed => completed_count += 1,
            _ => {}
        }
        total_bytes_downloaded += t.downloaded_bytes;
    }

    DownloadStats {
        active_count,
        queued_count,
        completed_count,
        total_bytes_downloaded,
        current_total_speed,
        speed_limit_bytes_per_sec: speed_limit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::sync::atomic::AtomicUsize;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let p = std::env::temp_dir()
                .join(format!("argon-dl-test-{}", uuid::Uuid::new_v4().simple()));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct HttpServer {
        base_url: String,
        task: tokio::task::JoinHandle<()>,
    }
    impl Drop for HttpServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    async fn spawn_server(
        handler: impl Fn(&str) -> Vec<u8> + Send + Sync + 'static,
    ) -> HttpServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handler = Arc::new(handler);
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let handler = handler.clone();
                tokio::spawn(async move {
                    let mut req = Vec::new();
                    let mut buf = [0u8; 4096];
                    while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                        let Ok(n) = socket.read(&mut buf).await else {
                            return;
                        };
                        if n == 0 {
                            return;
                        }
                        req.extend_from_slice(&buf[..n]);
                    }
                    let resp = handler(&String::from_utf8_lossy(&req));
                    let _ = socket.write_all(&resp).await;
                });
            }
        });
        HttpServer {
            base_url: format!("http://{addr}"),
            task,
        }
    }

    fn http_resp(status: &str, headers: &str, body: &[u8]) -> Vec<u8> {
        let mut out = format!("HTTP/1.1 {status}\r\nConnection: close\r\n{headers}\r\n").into_bytes();
        out.extend_from_slice(body);
        out
    }

    async fn new_service(root: &TempDir) -> Arc<DownloaderService> {
        DownloaderService::new(
            Config {
                state_dir: root.0.join("state"),
                default_download_dir: root.0.join("downloads"),
                is_android: false,
            },
            None,
            |_| {},
        )
        .await
        .unwrap()
    }

    async fn wait_settled(svc: &DownloaderService, id: &str) -> DownloadTask {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let snap = svc.snapshot().await;
                if let Some(t) = snap.tasks.into_iter().find(|t| t.id == id) {
                    if matches!(t.status, TaskStatus::Completed | TaskStatus::Error) {
                        return t;
                    }
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("download timed out")
    }

    #[tokio::test]
    async fn segmented_download_persists_verifies_sha256_and_survives_restart() {
        let root = TempDir::new();
        let data: Vec<u8> = (0..(128 * 1024)).map(|i| (i % 251) as u8).collect();
        let expected_hash = format!("{:x}", Sha256::digest(&data));
        let payload = data.clone();

        let srv = spawn_server(move |req| {
            let range = req.lines().find_map(|l| {
                l.to_ascii_lowercase()
                    .strip_prefix("range: bytes=")
                    .map(str::to_owned)
            });
            if let Some(r) = range {
                let (s, e) = r.split_once('-').unwrap();
                let (s, e): (usize, usize) = (s.parse().unwrap(), e.parse().unwrap());
                http_resp(
                    "206 Partial Content",
                    &format!(
                        "Content-Length: {}\r\nContent-Range: bytes {s}-{e}/{}\r\nETag: \"v1\"\r\n",
                        e - s + 1,
                        payload.len()
                    ),
                    &payload[s..=e],
                )
            } else {
                http_resp(
                    "200 OK",
                    &format!("Content-Length: {}\r\n", payload.len()),
                    &payload,
                )
            }
        })
        .await;

        let svc = new_service(&root).await;
        let task = svc
            .add_browser_download(BrowserDownloadPayload {
                url: format!("{}/archive.bin", srv.base_url),
                filename: Some("archive.bin".into()),
                headers: None,
                method: None,
                post_body: None,
                request_id: None,
            })
            .await
            .unwrap();
        let done = wait_settled(&svc, &task.id).await;
        assert_eq!(done.status, TaskStatus::Completed, "{:?}", done.error_message);
        assert_eq!(done.downloaded_bytes, data.len() as u64);
        let verified = svc.verify_task_hash(&task.id).await.unwrap().unwrap();
        assert_eq!(verified, expected_hash);
        svc.shutdown().await;

        // Restart service: persisted task and completion remain intact.
        let svc2 = new_service(&root).await;
        let snap2 = svc2.snapshot().await;
        assert_eq!(snap2.tasks.len(), 1);
        assert_eq!(snap2.tasks[0].status, TaskStatus::Completed);
        svc2.shutdown().await;
    }

    #[tokio::test]
    async fn mirror_fallback_recovers_when_primary_returns_404() {
        let root = TempDir::new();
        let srv = spawn_server(move |req| {
            if req.starts_with("GET /dead ") {
                http_resp("404 Not Found", "Content-Length: 0\r\n", b"")
            } else {
                http_resp("200 OK", "Content-Length: 5\r\n", b"argon")
            }
        })
        .await;

        let svc = new_service(&root).await;
        let task = svc
            .add_task(AddTaskRequest {
                url: format!("{}/dead", srv.base_url),
                filename: Some("mirror.bin".into()),
                segment_count: Some(4),
                expected_sha256: None,
                headers: None,
                post_body: None,
                method: None,
                media_info: None,
                queue_id: None,
                mirrors: Some(vec![format!("{}/live", srv.base_url)]),
            })
            .await
            .unwrap();
        let done = wait_settled(&svc, &task.id).await;
        assert_eq!(done.status, TaskStatus::Completed, "{:?}", done.error_message);
        assert_eq!(
            std::fs::read(done.save_path.unwrap()).unwrap(),
            b"argon"
        );
        svc.shutdown().await;
    }

    #[tokio::test]
    async fn auto_retry_recovers_after_transient_503_across_queue() {
        let root = TempDir::new();
        let hits = Arc::new(AtomicUsize::new(0));
        let hits_clone = hits.clone();
        let srv = spawn_server(move |_| {
            let n = hits_clone.fetch_add(1, Ordering::SeqCst);
            if n < 2 {
                http_resp("503 Service Unavailable", "Retry-After: 0\r\nContent-Length: 0\r\n", b"")
            } else {
                http_resp("200 OK", "Content-Length: 4\r\n", b"done")
            }
        })
        .await;

        let svc = new_service(&root).await;
        let task = svc
            .add_task(AddTaskRequest {
                url: format!("{}/retry.bin", srv.base_url),
                filename: Some("retry.bin".into()),
                segment_count: Some(1),
                expected_sha256: None,
                headers: None,
                post_body: None,
                method: None,
                media_info: None,
                queue_id: None,
                mirrors: None,
            })
            .await
            .unwrap();
        let done = wait_settled(&svc, &task.id).await;
        assert_eq!(done.status, TaskStatus::Completed, "{:?}", done.error_message);
        assert!(done.retry_count >= 1);
        svc.shutdown().await;
    }

    #[tokio::test]
    async fn range_unsupported_falls_back_cleanly_to_single_stream() {
        let root = TempDir::new();
        let data = b"hello-fallback-stream";
        let srv = spawn_server(move |req| {
            if req.contains("Range: bytes=0-0") {
                // Probe says 206 supported, then real segment returns 200 OK ignoring Range!
                http_resp(
                    "206 Partial Content",
                    &format!("Content-Length: 1\r\nContent-Range: bytes 0-0/{}\r\n", data.len()),
                    &data[0..1],
                )
            } else {
                http_resp(
                    "200 OK",
                    &format!("Content-Length: {}\r\n", data.len()),
                    data,
                )
            }
        })
        .await;

        let svc = new_service(&root).await;
        let task = svc
            .add_task(AddTaskRequest {
                url: format!("{}/fallback.bin", srv.base_url),
                filename: Some("fallback.bin".into()),
                segment_count: Some(4),
                expected_sha256: None,
                headers: None,
                post_body: None,
                method: None,
                media_info: None,
                queue_id: None,
                mirrors: None,
            })
            .await
            .unwrap();
        let done = wait_settled(&svc, &task.id).await;
        assert_eq!(done.status, TaskStatus::Completed, "{:?}", done.error_message);
        assert!(!done.accept_ranges);
        assert_eq!(std::fs::read(done.save_path.unwrap()).unwrap(), data);
        svc.shutdown().await;
    }

    #[tokio::test]
    async fn per_host_concurrency_cap_limits_simultaneous_same_host_downloads() {
        let root = TempDir::new();
        let active = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let active_c = active.clone();
        let max_c = max_seen.clone();
        let srv_task = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let active_c = active_c.clone();
                let max_c = max_c.clone();
                tokio::spawn(async move {
                    let mut req = Vec::new();
                    let mut buf = [0u8; 2048];
                    while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                        let Ok(n) = socket.read(&mut buf).await else { return; };
                        if n == 0 { return; }
                        req.extend_from_slice(&buf[..n]);
                    }
                    let req_str = String::from_utf8_lossy(&req);
                    if !req_str.contains("Range: bytes=0-0") {
                        let cur = active_c.fetch_add(1, Ordering::SeqCst) + 1;
                        max_c.fetch_max(cur, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(80)).await;
                        active_c.fetch_sub(1, Ordering::SeqCst);
                    }
                    let _ = socket
                        .write_all(&http_resp("200 OK", "Content-Length: 2\r\n", b"ok"))
                        .await;
                });
            }
        });

        let svc = new_service(&root).await;
        svc.update_settings(DownloaderSettingsPatch {
            max_concurrent_downloads: Some(4),
            max_connections_per_host: Some(1),
            ..Default::default()
        })
        .await
        .unwrap();

        let t1 = svc
            .add_task(AddTaskRequest {
                url: format!("{base_url}/a.bin"),
                filename: Some("a.bin".into()),
                segment_count: Some(1),
                expected_sha256: None,
                headers: None,
                post_body: None,
                method: None,
                media_info: None,
                queue_id: None,
                mirrors: None,
            })
            .await
            .unwrap();
        let t2 = svc
            .add_task(AddTaskRequest {
                url: format!("{base_url}/b.bin"),
                filename: Some("b.bin".into()),
                segment_count: Some(1),
                expected_sha256: None,
                headers: None,
                post_body: None,
                method: None,
                media_info: None,
                queue_id: None,
                mirrors: None,
            })
            .await
            .unwrap();

        assert_eq!(wait_settled(&svc, &t1.id).await.status, TaskStatus::Completed);
        assert_eq!(wait_settled(&svc, &t2.id).await.status, TaskStatus::Completed);
        assert_eq!(max_seen.load(Ordering::SeqCst), 1);
        srv_task.abort();
        svc.shutdown().await;
    }

    #[tokio::test]
    async fn post_request_and_custom_cookies_are_forwarded_on_download() {
        let root = TempDir::new();
        let srv = spawn_server(move |req| {
            let lower = req.to_ascii_lowercase();
            if req.starts_with("POST /export ")
                && lower.contains("cookie: session=xyz")
                && req.contains("token=99")
            {
                http_resp("200 OK", "Content-Length: 7\r\n", b"payload")
            } else {
                http_resp("403 Forbidden", "Content-Length: 0\r\n", b"")
            }
        })
        .await;

        let svc = new_service(&root).await;
        let mut headers = HashMap::new();
        headers.insert("Cookie".to_owned(), "session=xyz".to_owned());
        let task = svc
            .add_task(AddTaskRequest {
                url: format!("{}/export", srv.base_url),
                filename: Some("export.dat".into()),
                segment_count: Some(8),
                expected_sha256: None,
                headers: Some(headers),
                post_body: Some("token=99".into()),
                method: Some("POST".into()),
                media_info: None,
                queue_id: None,
                mirrors: None,
            })
            .await
            .unwrap();
        let done = wait_settled(&svc, &task.id).await;
        assert_eq!(done.status, TaskStatus::Completed, "{:?}", done.error_message);
        assert_eq!(
            std::fs::read(done.save_path.unwrap()).unwrap(),
            b"payload"
        );
        svc.shutdown().await;
    }

    #[tokio::test]
    async fn pause_and_resume_preserves_partial_bytes_without_corruption() {
        let root = TempDir::new();
        let data: Vec<u8> = (0..(256 * 1024)).map(|i| ((i * 31) % 251) as u8).collect();
        let data_clone = data.clone();
        let stalled_once = Arc::new(AtomicBool::new(false));
        let (tx_started, mut rx_started) = tokio::sync::watch::channel(false);
        let (tx_release, rx_release) = tokio::sync::watch::channel(false);

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let srv_task = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let payload = data_clone.clone();
                let stalled = stalled_once.clone();
                let tx_started = tx_started.clone();
                let mut rx_release = rx_release.clone();
                tokio::spawn(async move {
                    let mut req = Vec::new();
                    let mut buf = [0u8; 4096];
                    while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                        let Ok(n) = socket.read(&mut buf).await else { return; };
                        if n == 0 { return; }
                        req.extend_from_slice(&buf[..n]);
                    }
                    let req_str = String::from_utf8_lossy(&req);
                    let range = req_str.lines().find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("range: bytes=")
                            .map(str::to_owned)
                    });
                    let Some(r) = range else {
                        let _ = socket
                            .write_all(&http_resp(
                                "200 OK",
                                &format!("Content-Length: {}\r\nETag: \"p1\"\r\n", payload.len()),
                                &payload,
                            ))
                            .await;
                        return;
                    };
                    let (s_str, e_str) = r.split_once('-').unwrap();
                    let s: usize = s_str.parse().unwrap();
                    let e: usize = e_str.parse().unwrap();
                    if s > 0 && e - s + 1 > 65536 && !stalled.swap(true, Ordering::SeqCst) {
                        // Hold the very first chunk-1 request open while chunk 0 finishes and test pauses.
                        let _ = tx_started.send(true);
                        let _ = rx_release.wait_for(|v| *v).await;
                        return;
                    }
                    let _ = socket
                        .write_all(&http_resp(
                            "206 Partial Content",
                            &format!(
                                "Content-Length: {}\r\nContent-Range: bytes {s}-{e}/{}\r\nETag: \"p1\"\r\n",
                                e - s + 1,
                                payload.len()
                            ),
                            &payload[s..=e],
                        ))
                        .await;
                });
            }
        });

        let svc = new_service(&root).await;
        let task = svc
            .add_task(AddTaskRequest {
                url: format!("{base_url}/partial.bin"),
                filename: Some("partial.bin".into()),
                segment_count: Some(2),
                expected_sha256: None,
                headers: None,
                post_body: None,
                method: None,
                media_info: None,
                queue_id: None,
                mirrors: None,
            })
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(5), rx_started.wait_for(|v| *v))
            .await
            .expect("chunk 1 request should start")
            .unwrap();
        // Wait until chunk 0's partial bytes are recorded in snapshot before pausing.
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let snap = svc.snapshot().await;
                if snap.tasks.iter().any(|t| t.id == task.id && t.downloaded_bytes > 0) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("chunk 0 progress should become visible");

        svc.pause_task(&task.id).await.unwrap();
        let _ = tx_release.send(true);

        let snap = svc.snapshot().await;
        let paused = snap.tasks.iter().find(|t| t.id == task.id).unwrap();
        assert_eq!(paused.status, TaskStatus::Paused);
        assert!(paused.downloaded_bytes > 0);
        assert!(paused.downloaded_bytes < data.len() as u64);

        svc.resume_task(&task.id, false).await.unwrap();
        let done = wait_settled(&svc, &task.id).await;
        assert_eq!(done.status, TaskStatus::Completed, "{:?}", done.error_message);
        assert_eq!(done.downloaded_bytes, data.len() as u64);
        assert_eq!(std::fs::read(done.save_path.unwrap()).unwrap(), data);
        srv_task.abort();
        svc.shutdown().await;
    }

    #[tokio::test]
    async fn dynamic_work_stealing_splits_slow_segment_and_completes_intact() {
        let root = TempDir::new();
        let data: Vec<u8> = (0..(2 * 1024 * 1024)).map(|i| ((i * 17) % 251) as u8).collect();
        let data_clone = data.clone();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let srv_task = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                let payload = data_clone.clone();
                tokio::spawn(async move {
                    let mut req = Vec::new();
                    let mut buf = [0u8; 4096];
                    while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                        let Ok(n) = socket.read(&mut buf).await else { return; };
                        if n == 0 { return; }
                        req.extend_from_slice(&buf[..n]);
                    }
                    let req_str = String::from_utf8_lossy(&req);
                    let range = req_str.lines().find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("range: bytes=")
                            .map(str::to_owned)
                    });
                    let Some(r) = range else {
                        let _ = socket
                            .write_all(&http_resp(
                                "200 OK",
                                &format!("Content-Length: {}\r\nETag: \"ws1\"\r\n", payload.len()),
                                &payload,
                            ))
                            .await;
                        return;
                    };
                    let (s_str, e_str) = r.split_once('-').unwrap();
                    let s: usize = s_str.parse().unwrap();
                    let e: usize = e_str.parse().unwrap();
                    if s == payload.len() / 2 {
                        // Delay the initial second half long enough for chunk 0 to finish and steal its tail.
                        tokio::time::sleep(Duration::from_millis(180)).await;
                    }
                    let _ = socket
                        .write_all(&http_resp(
                            "206 Partial Content",
                            &format!(
                                "Content-Length: {}\r\nContent-Range: bytes {s}-{e}/{}\r\nETag: \"ws1\"\r\n",
                                e - s + 1,
                                payload.len()
                            ),
                            &payload[s..=e],
                        ))
                        .await;
                });
            }
        });

        let svc = new_service(&root).await;
        let task = svc
            .add_task(AddTaskRequest {
                url: format!("{base_url}/steal.bin"),
                filename: Some("steal.bin".into()),
                segment_count: Some(2),
                expected_sha256: None,
                headers: None,
                post_body: None,
                method: None,
                media_info: None,
                queue_id: None,
                mirrors: None,
            })
            .await
            .unwrap();
        let done = wait_settled(&svc, &task.id).await;
        assert_eq!(done.status, TaskStatus::Completed, "{:?}", done.error_message);
        assert!(done.chunks.len() >= 3, "expected work stealing to add a chunk: {:?}", done.chunks);
        assert_eq!(std::fs::read(done.save_path.unwrap()).unwrap(), data);
        srv_task.abort();
        svc.shutdown().await;
    }

    #[tokio::test]
    async fn migrate_legacy_tasks_recovers_in_flight_status_and_skips_when_non_empty() {
        let root = TempDir::new();
        let svc = new_service(&root).await;

        let legacy_active = DownloadTask {
            id: "leg_1".into(),
            url: "https://example.com/big.iso".into(),
            original_url: "https://example.com/big.iso".into(),
            filename: "big.iso".into(),
            total_bytes: 1024,
            downloaded_bytes: 256,
            status: TaskStatus::Downloading,
            speed_bytes_per_sec: 12345.0,
            eta_seconds: 9,
            chunks: vec![DownloadChunk {
                index: 0,
                start_byte: 0,
                end_byte: 1023,
                downloaded: 256,
                status: ChunkStatus::Downloading,
                speed_bytes_per_sec: None,
            }],
            created_at: 100,
            completed_at: None,
            error_message: None,
            checksum: None,
            save_path: None,
            headers: HashMap::new(),
            post_body: None,
            method: None,
            category: TaskCategory::Archives,
            media_info: None,
            requested_segments: 1,
            custom_filename: false,
            file_prepared: false,
            accept_ranges: true,
            etag: None,
            last_modified: None,
            probed: true,
            queue_id: QueueId::Main,
            speed_limit_bytes_per_sec: 0,
            mirrors: Vec::new(),
            active_mirror_index: 0,
            retry_count: 0,
        };

        let migrated = svc.migrate_legacy_tasks(vec![legacy_active.clone()]).await.unwrap();
        assert_eq!(migrated, 1);

        // Second migration call when non-empty must be a no-op.
        let again = svc.migrate_legacy_tasks(vec![legacy_active]).await.unwrap();
        assert_eq!(again, 0);

        let snap = svc.snapshot().await;
        assert_eq!(snap.tasks.len(), 1);
        assert_eq!(snap.tasks[0].status, TaskStatus::Paused);
        assert_eq!(snap.tasks[0].speed_bytes_per_sec, 0.0);
        assert_eq!(snap.tasks[0].eta_seconds, 0);
        svc.shutdown().await;
    }
}
