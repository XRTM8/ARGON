mod bridge;
mod flow;
#[cfg(target_os = "android")]
mod flow_mobile;
mod media;
#[cfg(desktop)]
mod tray;
mod updater;

use argon_downloader_core::{
    disk,
    resolver::sanitize_filename,
    AddTaskRequest, BrowserDownloadPayload, Config as DownloaderConfig, DownloadEvent,
    DownloadTask, DownloaderService, DownloaderSettings, DownloaderSettingsPatch,
    DownloaderSnapshot, MediaMetadata, QueueId, UrlProbeResult,
};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock as StdRwLock};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::OnceCell;

#[derive(Default)]
pub struct DownloaderRuntime {
    pub service: OnceCell<Arc<DownloaderService>>,
}

pub struct TaskManager {
    pub frontend_ready: AtomicBool,
    pub updating: AtomicBool,
    pub bridge_listening: AtomicBool,
    pub minimize_to_tray: AtomicBool,
    pub bridge_token: StdRwLock<String>,
}

impl Default for TaskManager {
    fn default() -> Self {
        Self {
            frontend_ready: AtomicBool::new(false),
            updating: AtomicBool::new(false),
            bridge_listening: AtomicBool::new(false),
            minimize_to_tray: AtomicBool::new(true),
            bridge_token: StdRwLock::new(new_bridge_secret()),
        }
    }
}

fn new_bridge_secret() -> String {
    format!(
        "{}{}",
        argon_downloader_core::model::new_id("br"),
        argon_downloader_core::model::new_id("")
    )
    .replace('_', "")
}

fn load_or_create_bridge_token(app: &AppHandle) -> String {
    let Ok(dir) = app.path().app_local_data_dir() else {
        return new_bridge_secret();
    };
    let state_dir = dir.join("downloader-v2");
    let token_path = state_dir.join("bridge.token");
    if let Ok(existing) = std::fs::read_to_string(&token_path) {
        let trimmed = existing.trim();
        if trimmed.len() >= 24 && trimmed.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return trimmed.to_owned();
        }
    }
    let fresh = new_bridge_secret();
    let _ = std::fs::create_dir_all(&state_dir);
    let _ = std::fs::write(&token_path, &fresh);
    fresh
}

fn save_bridge_token(app: &AppHandle, token: &str) -> Result<(), String> {
    let state_dir = app
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?
        .join("downloader-v2");
    std::fs::create_dir_all(&state_dir).map_err(|e| e.to_string())?;
    std::fs::write(state_dir.join("bridge.token"), token).map_err(|e| e.to_string())
}

pub async fn ensure_downloader(app: &AppHandle) -> Result<Arc<DownloaderService>, String> {
    let runtime = app.state::<DownloaderRuntime>();
    let service = runtime
        .service
        .get_or_try_init(|| async {
            let state_dir = app
                .path()
                .app_local_data_dir()
                .map_err(|e| e.to_string())?
                .join("downloader-v2");
            let default_download_dir = get_resolved_download_dir(app);
            let is_android = cfg!(target_os = "android");
            let app_emit = app.clone();
            let media_hook = Arc::new(media::TauriMediaExtractor { app: app.clone() });

            let service = DownloaderService::new(
                DownloaderConfig {
                    state_dir,
                    default_download_dir,
                    is_android,
                },
                Some(media_hook),
                move |event| match &event {
                    DownloadEvent::StateChanged => {
                        let _ = app_emit.emit("download-state-changed", ());
                    }
                    DownloadEvent::TelemetryTick { .. } => {
                        let _ = app_emit.emit("download-event", &event);
                    }
                    DownloadEvent::AllIdleShutdownPending => {
                        let _ = app_emit.emit("download-event", &event);
                        let app_handle = app_emit.clone();
                        tauri::async_runtime::spawn(async move {
                            let action = match app_handle.state::<DownloaderRuntime>().service.get() {
                                Some(mgr) => mgr.settings().await.completion_action,
                                None => "shutdown".to_string(),
                            };
                            let _ = trigger_os_completion_action(&action, false);
                        });
                    }
                    DownloadEvent::TaskCompleted { save_path, .. } => {
                        #[cfg(target_os = "android")]
                        {
                            let app_pub = app_emit.clone();
                            let path_pub = save_path.clone();
                            tauri::async_runtime::spawn(async move {
                                let _ = flow_mobile::call::<serde_json::Value>(
                                    app_pub,
                                    "publishDownload",
                                    serde_json::json!({ "path": path_pub }),
                                )
                                .await;
                            });
                        }
                        #[cfg(not(target_os = "android"))]
                        let _ = save_path;
                        let _ = app_emit.emit("download-event", &event);
                    }
                    DownloadEvent::TaskFailed { .. } | DownloadEvent::MediaDownloadRequested { .. } => {
                        let _ = app_emit.emit("download-event", &event);
                    }
                },
            )
            .await?;

            let snap = service.snapshot().await;
            app.state::<Arc<TaskManager>>()
                .minimize_to_tray
                .store(snap.settings.minimize_to_tray, Ordering::Relaxed);
            #[cfg(desktop)]
            {
                let _ = tray::sync_windows_autostart(snap.settings.start_with_windows);
            }
            Ok::<Arc<DownloaderService>, String>(service)
        })
        .await?;
    Ok(service.clone())
}

pub fn get_resolved_download_dir(app: &AppHandle) -> std::path::PathBuf {
    #[cfg(target_os = "android")]
    {
        if let Ok(dir) = app.path().app_data_dir() {
            let sub = dir.join("Download");
            if std::fs::create_dir_all(&sub).is_ok() {
                let test_probe = sub.join(".otium_write_probe");
                if std::fs::write(&test_probe, b"ok").is_ok() {
                    let _ = std::fs::remove_file(&test_probe);
                    return sub;
                }
            }
        }
        let ext_dir = std::path::PathBuf::from(
            "/storage/emulated/0/Android/data/com.otium.downloader/files/Download",
        );
        if std::fs::create_dir_all(&ext_dir).is_ok() {
            let test_probe = ext_dir.join(".otium_write_probe");
            if std::fs::write(&test_probe, b"ok").is_ok() {
                let _ = std::fs::remove_file(&test_probe);
                return ext_dir;
            }
        }
        if let Ok(dir) = app.path().app_local_data_dir() {
            let sub = dir.join("Download");
            if std::fs::create_dir_all(&sub).is_ok() {
                let test_probe = sub.join(".otium_write_probe");
                if std::fs::write(&test_probe, b"ok").is_ok() {
                    let _ = std::fs::remove_file(&test_probe);
                    return sub;
                }
            }
        }
        if let Ok(dir) = app.path().app_cache_dir() {
            let sub = dir.join("Download");
            let _ = std::fs::create_dir_all(&sub);
            return sub;
        }
    }

    if let Ok(p) = app.path().download_dir() {
        return p;
    }
    if let Ok(p) = app.path().document_dir() {
        return p;
    }
    if let Ok(p) = app.path().app_local_data_dir() {
        return p;
    }
    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
}

pub fn resolve_destination_path(app: &AppHandle, dest: &str) -> std::path::PathBuf {
    let clean = dest.trim();
    let base = get_resolved_download_dir(app);
    if clean.is_empty() {
        return base.join("download.bin");
    }
    let p = Path::new(clean);
    #[cfg(target_os = "android")]
    {
        if p.is_absolute() && p.exists() {
            return p.to_path_buf();
        }
        let fname = p
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("download.bin"));
        base.join(fname)
    }
    #[cfg(not(target_os = "android"))]
    {
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            let fname = p.file_name().unwrap_or_else(|| std::ffi::OsStr::new(clean));
            base.join(fname)
        }
    }
}

#[tauri::command]
async fn dl_snapshot(app: AppHandle) -> Result<DownloaderSnapshot, String> {
    Ok(ensure_downloader(&app).await?.snapshot().await)
}

#[tauri::command]
async fn dl_add_task(app: AppHandle, request: AddTaskRequest) -> Result<DownloadTask, String> {
    ensure_downloader(&app).await?.add_task(request).await
}

#[tauri::command]
async fn dl_add_batch(
    app: AppHandle,
    urls: Vec<String>,
    segment_count: Option<u32>,
    queue_id: Option<QueueId>,
) -> Result<Vec<DownloadTask>, String> {
    let svc = ensure_downloader(&app).await?;
    let mut created = Vec::new();
    for raw in urls {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let task = svc
            .add_task(AddTaskRequest {
                url: trimmed.to_owned(),
                filename: None,
                segment_count,
                expected_sha256: None,
                headers: None,
                post_body: None,
                method: None,
                media_info: None,
                queue_id: queue_id.clone(),
                mirrors: None,
                queue_priority: None,
            })
            .await?;
        created.push(task);
    }
    Ok(created)
}

#[tauri::command]
async fn dl_pause_task(app: AppHandle, id: String) -> Result<(), String> {
    ensure_downloader(&app).await?.pause_task(&id).await
}

#[tauri::command]
async fn dl_resume_task(app: AppHandle, id: String, force: Option<bool>) -> Result<(), String> {
    ensure_downloader(&app)
        .await?
        .resume_task(&id, force.unwrap_or(false))
        .await
}

#[tauri::command]
async fn dl_restart_task(app: AppHandle, id: String) -> Result<(), String> {
    ensure_downloader(&app).await?.restart_task(&id).await
}

#[tauri::command]
async fn dl_refresh_url(app: AppHandle, id: String, url: String) -> Result<(), String> {
    ensure_downloader(&app)
        .await?
        .refresh_task_url(&id, url)
        .await
}

#[tauri::command]
async fn dl_pause_all(app: AppHandle) -> Result<(), String> {
    ensure_downloader(&app).await?.pause_all().await
}

#[tauri::command]
async fn dl_resume_all(app: AppHandle) -> Result<(), String> {
    ensure_downloader(&app).await?.resume_all().await
}

#[tauri::command]
async fn dl_remove_task(
    app: AppHandle,
    id: String,
    delete_file_from_disk: Option<bool>,
) -> Result<(), String> {
    ensure_downloader(&app)
        .await?
        .remove_task(&id, delete_file_from_disk.unwrap_or(false))
        .await
}

#[tauri::command]
async fn dl_clear_completed(app: AppHandle) -> Result<(), String> {
    ensure_downloader(&app).await?.clear_completed().await
}

#[tauri::command]
async fn dl_set_task_queue(app: AppHandle, id: String, queue_id: QueueId) -> Result<(), String> {
    ensure_downloader(&app)
        .await?
        .set_task_queue(&id, queue_id)
        .await
}

#[tauri::command]
async fn dl_set_task_priority(
    app: AppHandle,
    id: String,
    priority: i32,
) -> Result<(), String> {
    ensure_downloader(&app)
        .await?
        .set_task_priority(&id, priority)
        .await
}

#[tauri::command]
async fn dl_bump_task_queue(
    app: AppHandle,
    id: String,
    top: bool,
) -> Result<(), String> {
    ensure_downloader(&app)
        .await?
        .bump_task_queue(&id, top)
        .await
}

#[tauri::command]
async fn dl_set_task_speed_limit(
    app: AppHandle,
    id: String,
    bytes_per_sec: u64,
) -> Result<(), String> {
    ensure_downloader(&app)
        .await?
        .set_task_speed_limit(&id, bytes_per_sec)
        .await
}

#[tauri::command]
async fn dl_verify_hash(app: AppHandle, id: String) -> Result<Option<String>, String> {
    ensure_downloader(&app).await?.verify_task_hash(&id).await
}

#[tauri::command]
async fn dl_update_settings(
    app: AppHandle,
    state: tauri::State<'_, Arc<TaskManager>>,
    patch: DownloaderSettingsPatch,
) -> Result<DownloaderSettings, String> {
    let svc = ensure_downloader(&app).await?;
    let autostart_changed = patch.start_with_windows;
    let updated = svc.update_settings(patch).await?;
    state
        .minimize_to_tray
        .store(updated.minimize_to_tray, Ordering::Relaxed);
    if let Some(enabled) = autostart_changed {
        #[cfg(desktop)]
        tray::sync_windows_autostart(enabled)?;
        #[cfg(not(desktop))]
        let _ = enabled;
    }
    Ok(updated)
}

#[tauri::command]
async fn dl_migrate_legacy(app: AppHandle, tasks: Vec<DownloadTask>) -> Result<usize, String> {
    ensure_downloader(&app)
        .await?
        .migrate_legacy_tasks(tasks)
        .await
}

#[tauri::command]
async fn zip_peek_list(
    app: AppHandle,
    url: String,
    custom_headers: Option<HashMap<String, String>>,
) -> Result<Vec<argon_downloader_core::zip_peek::RemoteZipEntry>, String> {
    let svc = ensure_downloader(&app).await?;
    argon_downloader_core::zip_peek::list_remote_zip(
        svc.http_client(),
        &url,
        custom_headers.as_ref(),
    )
    .await
}

#[tauri::command]
async fn zip_peek_extract(
    app: AppHandle,
    url: String,
    entry: argon_downloader_core::zip_peek::RemoteZipEntry,
    dest_path: String,
    custom_headers: Option<HashMap<String, String>>,
) -> Result<u64, String> {
    let svc = ensure_downloader(&app).await?;
    let dest = std::path::PathBuf::from(dest_path);
    argon_downloader_core::zip_peek::extract_remote_zip_entry(
        svc.http_client(),
        &url,
        custom_headers.as_ref(),
        &entry,
        &dest,
    )
    .await
}

#[tauri::command]
async fn hf_fetch_repo(
    app: AppHandle,
    input: String,
    use_mirror: bool,
    token: Option<String>,
) -> Result<argon_downloader_core::huggingface::HfRepoSummary, String> {
    let svc = ensure_downloader(&app).await?;
    argon_downloader_core::huggingface::fetch_hf_repo(
        svc.http_client(),
        &input,
        use_mirror,
        token.as_deref(),
    )
    .await
}

#[tauri::command]
async fn antivirus_scan_file(
    kind: Option<String>,
    path: String,
) -> Result<argon_downloader_core::antivirus::ScanResult, String> {
    Ok(argon_downloader_core::antivirus::scan_file(
        kind.as_deref().unwrap_or("auto"),
        None,
        std::path::Path::new(&path),
    )
    .await)
}

#[tauri::command]
async fn repair_browser_integration(app: AppHandle) -> Result<Vec<String>, String> {
    #[cfg(all(windows, desktop))]
    {
        Ok(register_native_messaging_host(&app))
    }
    #[cfg(not(all(windows, desktop)))]
    {
        let _ = app;
        Ok(Vec::new())
    }
}

#[tauri::command]
async fn probe_url(
    app: AppHandle,
    url: String,
    custom_headers: Option<HashMap<String, String>>,
) -> Result<UrlProbeResult, String> {
    let svc = ensure_downloader(&app).await?;
    argon_downloader_core::probe_resource(svc.http_client(), &url, custom_headers.as_ref()).await
}

#[tauri::command]
async fn poll_telemetry_binary(app: AppHandle) -> Result<tauri::ipc::Response, String> {
    let bytes = app
        .state::<DownloaderRuntime>()
        .service
        .get()
        .map(|svc| svc.snapshot_telemetry_binary())
        .unwrap_or_else(|| vec![0x33, 0x47, 0x52, 0x41, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    Ok(tauri::ipc::Response::new(bytes))
}

#[tauri::command]
async fn get_default_download_directory(app: AppHandle) -> Result<String, String> {
    let svc = ensure_downloader(&app).await?;
    let snap = svc.snapshot().await;
    let path = if !snap.settings.default_directory.is_empty() {
        std::path::PathBuf::from(&snap.settings.default_directory)
    } else {
        get_resolved_download_dir(&app)
    };
    tokio::fs::create_dir_all(&path)
        .await
        .map_err(|e| format!("Download folder unavailable: {e}"))?;
    svc.allow_root(path.clone()).await;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
async fn validate_download_directory(app: AppHandle, directory: String) -> Result<String, String> {
    let svc = ensure_downloader(&app).await?;
    let updated = svc
        .update_settings(DownloaderSettingsPatch {
            default_directory: Some(directory),
            ..Default::default()
        })
        .await?;
    Ok(updated.default_directory)
}

#[tauri::command]
async fn set_browser_bridge_ready(state: tauri::State<'_, Arc<TaskManager>>) -> Result<(), String> {
    state.frontend_ready.store(true, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
fn open_browser_test(app: AppHandle) -> Result<(), String> {
    app.opener()
        .open_url("http://127.0.0.1:41523/sample.txt", None::<&str>)
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn browser_bridge_status(
    app: AppHandle,
    state: tauri::State<'_, Arc<TaskManager>>,
) -> Result<serde_json::Value, String> {
    let token = state.bridge_token.read().map(|g| g.clone()).unwrap_or_default();
    Ok(serde_json::json!({
        "listening": state.bridge_listening.load(Ordering::Relaxed),
        "ready": true,
        "frontendMounted": state.frontend_ready.load(Ordering::Relaxed),
        "hasFfmpeg": media::has_ffmpeg_installed(&app),
        "port": 41523,
        "token": token
    }))
}

#[tauri::command]
async fn regenerate_bridge_token(
    app: AppHandle,
    state: tauri::State<'_, Arc<TaskManager>>,
) -> Result<String, String> {
    let fresh = new_bridge_secret();
    save_bridge_token(&app, &fresh)?;
    if let Ok(mut guard) = state.bridge_token.write() {
        *guard = fresh.clone();
    }
    Ok(fresh)
}

#[tauri::command]
async fn open_media_sniffer(
    app: AppHandle,
    url: Option<String>,
) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "android")]
    {
        let target = url.unwrap_or_else(|| "https://google.com".to_string());
        flow_mobile::call(
            app,
            "openMediaSniffer",
            serde_json::json!({ "url": target }),
        )
        .await
    }
    #[cfg(not(target_os = "android"))]
    {
        let target = url.unwrap_or_else(|| "https://google.com".to_string());
        app.opener()
            .open_url(&target, None::<&str>)
            .map_err(|e| format!("تعذر فتح المتصفح: {e}"))?;
        Ok(serde_json::json!({ "status": "opened_in_browser", "url": target }))
    }
}

#[tauri::command]
fn parse_torrent_info(
    raw_uri_or_path: String,
) -> Result<argon_downloader_core::TorrentMetadata, String> {
    if argon_downloader_core::is_magnet_uri(&raw_uri_or_path) {
        argon_downloader_core::parse_magnet_uri(&raw_uri_or_path)
    } else {
        let path = std::path::Path::new(&raw_uri_or_path);
        if path.exists() {
            let bytes = std::fs::read(path).map_err(|e| format!("تعذر قراءة ملف التورنت: {e}"))?;
            argon_downloader_core::parse_torrent_bytes(&bytes)
        } else {
            Err("الرابط أو الملف ليس رابط ماغنت صالحاً أو ملف تورنت موجود".to_string())
        }
    }
}

#[tauri::command]
async fn verify_file_sha256(app: AppHandle, file_path: String) -> Result<String, String> {
    let resolved = resolve_destination_path(&app, &file_path);
    disk::verify_file_sha256(&resolved).await
}

#[tauri::command]
async fn open_destination_folder(app: AppHandle, file_path: String) -> Result<(), String> {
    let target = resolve_destination_path(&app, &file_path);
    if !target.exists() {
        return Err("Destination path does not exist".to_string());
    }
    let target_str = target.to_string_lossy().to_string();
    #[cfg(target_os = "android")]
    {
        flow_mobile::call::<serde_json::Value>(
            app,
            "openFile",
            serde_json::json!({ "path": target_str }),
        )
        .await?;
        Ok(())
    }
    #[cfg(not(target_os = "android"))]
    {
        app.opener()
            .reveal_item_in_dir(&target_str)
            .map_err(|e| e.to_string())
    }
}

#[tauri::command]
async fn share_file_item(app: AppHandle, file_path: String) -> Result<(), String> {
    let target = resolve_destination_path(&app, &file_path);
    if !target.exists() {
        return Err("File does not exist on disk".to_string());
    }
    let target_str = target.to_string_lossy().to_string();
    #[cfg(target_os = "android")]
    {
        flow_mobile::call::<serde_json::Value>(
            app,
            "shareFile",
            serde_json::json!({ "path": target_str }),
        )
        .await?;
        Ok(())
    }
    #[cfg(not(target_os = "android"))]
    {
        app.opener()
            .reveal_item_in_dir(&target_str)
            .map_err(|e| e.to_string())
    }
}

#[tauri::command]
async fn sync_android_transfer_notification(
    app: AppHandle,
    active_count: u32,
    title: String,
    text: String,
    progress: u32,
    indeterminate: bool,
) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        flow_mobile::call::<serde_json::Value>(
            app,
            "syncTransferNotification",
            serde_json::json!({
                "activeCount": active_count,
                "title": title,
                "text": text,
                "progress": progress,
                "indeterminate": indeterminate
            }),
        )
        .await?;
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, active_count, title, text, progress, indeterminate);
    }
    Ok(())
}

#[tauri::command]
async fn scan_media_file(app: AppHandle, file_path: String) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        let target = resolve_destination_path(&app, &file_path);
        let target_str = target.to_string_lossy().to_string();
        flow_mobile::call::<serde_json::Value>(
            app,
            "scanMediaFile",
            serde_json::json!({
                "path": target_str
            }),
        )
        .await?;
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, file_path);
    }
    Ok(())
}

#[tauri::command]
async fn request_battery_exemption(app: AppHandle) -> Result<(), String> {
    #[cfg(target_os = "android")]
    {
        flow_mobile::call::<serde_json::Value>(
            app,
            "requestIgnoreBatteryOptimizations",
            serde_json::json!({}),
        )
        .await?;
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
    }
    Ok(())
}

#[tauri::command]
async fn open_file_item(app: AppHandle, file_path: String) -> Result<(), String> {
    let target = resolve_destination_path(&app, &file_path);
    if !target.exists() {
        return Err("File does not exist on disk".to_string());
    }
    let target_str = target.to_string_lossy().to_string();
    #[cfg(not(target_os = "android"))]
    {
        app.opener()
            .open_path(&target_str, None::<&str>)
            .map_err(|e| e.to_string())
    }
    #[cfg(target_os = "android")]
    {
        flow_mobile::call::<serde_json::Value>(
            app,
            "openFile",
            serde_json::json!({"path":target_str}),
        )
        .await?;
        Ok(())
    }
}

#[tauri::command]
async fn delete_file_item(app: AppHandle, file_path: String) -> Result<(), String> {
    let clean = file_path.trim();
    if clean.is_empty() {
        return Ok(());
    }
    let target_path = resolve_destination_path(&app, clean);
    let mut allowed = Vec::new();
    if let Ok(d) = get_resolved_download_dir(&app).canonicalize() {
        allowed.push(d);
    }
    if let Ok(d) = app.path().download_dir().and_then(|p| p.canonicalize().map_err(Into::into)) {
        allowed.push(d);
    }
    if let Ok(h) = app.path().home_dir().and_then(|p| p.canonicalize().map_err(Into::into)) {
        allowed.push(h);
    }
    if let Ok(l) = app
        .path()
        .app_local_data_dir()
        .and_then(|p| p.canonicalize().map_err(Into::into))
    {
        allowed.push(l);
    }
    if let Ok(dl) = ensure_downloader(&app).await {
        let snap = dl.snapshot().await;
        if !snap.settings.default_directory.is_empty() {
            if let Ok(c) = std::path::PathBuf::from(&snap.settings.default_directory).canonicalize() {
                allowed.push(c);
            }
        }
    }
    disk::delete_safe_file(&target_path, &allowed).await
}

#[cfg(desktop)]
fn start_clipboard_monitor(app_handle: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut last_clipboard = String::new();
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(800));
        loop {
            interval.tick().await;
            let Ok(dl) = ensure_downloader(&app_handle).await else {
                continue;
            };
            let snap = dl.snapshot().await;
            if !snap.settings.clipboard_monitor_enabled {
                continue;
            }
            let current = tokio::task::spawn_blocking(|| {
                arboard::Clipboard::new().ok().and_then(|mut cb| cb.get_text().ok())
            })
            .await
            .ok()
            .flatten();

            if let Some(text) = current {
                let trimmed = text.trim();
                if trimmed.len() > 10 && trimmed.len() < 2048 && trimmed != last_clipboard {
                    last_clipboard = trimmed.to_string();
                    if let Ok(url) = reqwest::Url::parse(trimmed) {
                        if matches!(url.scheme(), "http" | "https") {
                            let path = url.path().to_ascii_lowercase();
                            let is_downloadable = path.ends_with(".zip")
                                || path.ends_with(".rar")
                                || path.ends_with(".7z")
                                || path.ends_with(".tar")
                                || path.ends_with(".gz")
                                || path.ends_with(".exe")
                                || path.ends_with(".msi")
                                || path.ends_with(".iso")
                                || path.ends_with(".dmg")
                                || path.ends_with(".apk")
                                || path.ends_with(".mp4")
                                || path.ends_with(".mkv")
                                || path.ends_with(".mp3")
                                || path.ends_with(".pdf")
                                || path.ends_with(".epub");
                            let is_media = !["direct", "other"].contains(&argon_downloader_core::resolver::detect_platform(trimmed));
                            if is_downloadable || is_media {
                                let _ = app_handle.emit("argon-clipboard-url", trimmed);
                            }
                        }
                    }
                }
            }
        }
    });
}

#[cfg(all(windows, desktop))]
fn register_native_messaging_host(app: &AppHandle) -> Vec<String> {
    let mut registered = Vec::new();
    if let Ok(exe_path) = std::env::current_exe() {
        if let Ok(local_data) = app.path().app_local_data_dir() {
            let manifest_path = local_data.join("com.otium.downloader.json");
            let json_content = serde_json::json!({
                "name": "com.otium.downloader",
                "description": "ARGON Native Messaging Host",
                "path": exe_path.to_string_lossy(),
                "type": "stdio",
                "allowed_origins": [
                    "chrome-extension://ikdgdlmiillcilhcjcbmplacomoiemhk/"
                ]
            });
            if let Ok(content_str) = serde_json::to_string_pretty(&json_content) {
                let _ = std::fs::create_dir_all(&local_data);
                let _ = std::fs::write(&manifest_path, content_str);
            }

            let firefox_manifest_path = local_data.join("com.otium.downloader.firefox.json");
            let firefox_json = serde_json::json!({
                "name": "com.otium.downloader",
                "description": "ARGON Native Messaging Host",
                "path": exe_path.to_string_lossy(),
                "type": "stdio",
                "allowed_extensions": [
                    "argon@xrtm8.github.io"
                ]
            });
            if let Ok(ff_str) = serde_json::to_string_pretty(&firefox_json) {
                let _ = std::fs::write(&firefox_manifest_path, ff_str);
            }

            use winreg::enums::HKEY_CURRENT_USER;
            use winreg::RegKey;
            let hkcu = RegKey::predef(HKEY_CURRENT_USER);
            let manifest_str = manifest_path.to_string_lossy().to_string();
            for subkey in [
                r"Software\Google\Chrome\NativeMessagingHosts\com.otium.downloader",
                r"Software\Microsoft\Edge\NativeMessagingHosts\com.otium.downloader",
                r"Software\Chromium\NativeMessagingHosts\com.otium.downloader",
                r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.otium.downloader",
                r"Software\Vivaldi\NativeMessagingHosts\com.otium.downloader",
            ] {
                if let Ok((key, _)) = hkcu.create_subkey(subkey) {
                    if key.set_value("", &manifest_str).is_ok() {
                        registered.push(subkey.to_string());
                    }
                }
            }
            let ff_subkey = r"Software\Mozilla\NativeMessagingHosts\com.otium.downloader";
            if let Ok((key, _)) = hkcu.create_subkey(ff_subkey) {
                if key
                    .set_value("", &firefox_manifest_path.to_string_lossy().to_string())
                    .is_ok()
                {
                    registered.push(ff_subkey.to_string());
                }
            }
        }
    }
    registered
}

#[cfg(desktop)]
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(desktop)]
fn start_browser_bridge(app_handle: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let addr = "127.0.0.1:41523";
        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => {
                app_handle
                    .state::<Arc<TaskManager>>()
                    .bridge_listening
                    .store(true, Ordering::Relaxed);
                log::info!("ARGON Browser Bridge listening on http://{}", addr);
                l
            }
            Err(e) => {
                log::warn!("Could not bind Browser Bridge port {}: {}", addr, e);
                return;
            }
        };

        let conn_limit = Arc::new(tokio::sync::Semaphore::new(16));

        loop {
            let (mut socket, _) = match listener.accept().await {
                Ok(conn) => conn,
                Err(_) => continue,
            };
            let Ok(permit) = conn_limit.clone().try_acquire_owned() else {
                continue;
            };

            let app = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                let _permit = permit;
                use tokio::io::AsyncWriteExt;
                let request = match bridge::read_request(&mut socket).await {
                    Ok(request) => request,
                    Err(_) => {
                        let _ = socket
                            .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")
                            .await;
                        return;
                    }
                };

                let request_head = request.split("\r\n\r\n").next().unwrap_or("");
                let origin_header = request_head
                    .lines()
                    .find(|line| line.to_ascii_lowercase().starts_with("origin:"))
                    .map(|line| line[7..].trim().to_string());
                let _client_header = request_head
                    .lines()
                    .find(|line| line.to_ascii_lowercase().starts_with("x-otium-client:"))
                    .and_then(|line| {
                        line.split_once(':')
                            .map(|(_, value)| value.trim().to_owned())
                    });
                let auth_token = request_head
                    .lines()
                    .find(|line| line.to_ascii_lowercase().starts_with("authorization:"))
                    .and_then(|line| line.split_once(':').map(|(_, v)| v.trim().to_owned()))
                    .or_else(|| {
                        request_head
                            .lines()
                            .find(|line| line.to_ascii_lowercase().starts_with("x-argon-token:"))
                            .and_then(|line| line.split_once(':').map(|(_, v)| v.trim().to_owned()))
                    })
                    .map(|token| {
                        if let Some(stripped) = token.strip_prefix("Bearer ") {
                            stripped.trim().to_owned()
                        } else {
                            token.trim().to_owned()
                        }
                    });
                let expected_token = app
                    .state::<Arc<TaskManager>>()
                    .bridge_token
                    .read()
                    .map(|g| g.clone())
                    .unwrap_or_default();
                let token_valid = !expected_token.is_empty()
                    && auth_token
                        .as_ref()
                        .is_some_and(|tok| constant_time_eq(tok.as_bytes(), expected_token.as_bytes()));

                if let Some(ref orig) = origin_header {
                    let lower = orig.to_ascii_lowercase();
                    if lower.starts_with("http://") || lower.starts_with("https://") {
                        let body = r#"{"error":"forbidden_origin","message":"Web origins are blocked from communicating with ARGON loopback bridge"}"#;
                        let response = format!(
                            "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = socket.write_all(response.as_bytes()).await;
                        return;
                    }
                }

                if request.starts_with("OPTIONS") {
                    let allowed_origin = match origin_header.as_deref() {
                        Some(orig)
                            if orig.starts_with("chrome-extension://")
                                || orig.starts_with("moz-extension://")
                                || orig.starts_with("safari-web-extension://") =>
                        {
                            orig
                        }
                        _ => {
                            let _ = socket
                                .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
                                .await;
                            return;
                        }
                    };
                    let response = format!(
                        "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: {}\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization, X-Otium-Client, X-Argon-Token\r\nAccess-Control-Max-Age: 86400\r\nContent-Length: 0\r\n\r\n",
                        allowed_origin
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    return;
                }

                let cors_origin_str = match origin_header.as_deref() {
                    Some(orig)
                        if orig.starts_with("chrome-extension://")
                            || orig.starts_with("moz-extension://")
                            || orig.starts_with("safari-web-extension://") =>
                    {
                        format!("Access-Control-Allow-Origin: {}\r\n", orig)
                    }
                    _ => String::new(),
                };

                if app
                    .state::<Arc<TaskManager>>()
                    .updating
                    .load(Ordering::SeqCst)
                {
                    let _ = socket
                        .write_all(
                            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n",
                        )
                        .await;
                    return;
                }

                let has_ffmpeg = media::has_ffmpeg_installed(&app);

                if request.starts_with("GET /sample.txt ") {
                    let body = "ARGON browser integration test.\r\n";
                    let reply = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=argon-browser-test.txt\r\nCache-Control: no-store\r\nContent-Length: {}\r\n\r\n",
                        body.len(),
                    );
                    if socket.write_all(reply.as_bytes()).await.is_ok() {
                        // Leave time for the browser's onCreated listener to claim the test file.
                        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
                        let _ = socket.write_all(body.as_bytes()).await;
                    }
                    return;
                }

                if request.starts_with("GET /ping ") || request.starts_with("GET /status ") {
                    let mut caps = vec!["persisted-handoff", "staged-handoff", "direct-files", "cookies-post"];
                    if has_ffmpeg {
                        caps.push("ffmpeg-hls");
                    }
                    let body = serde_json::json!({
                        "status": "ok",
                        "app": "ARGON",
                        "version": env!("CARGO_PKG_VERSION"),
                        "capabilities": caps,
                        "authRequired": true,
                        "authenticated": token_valid,
                    })
                    .to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\n{}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                        cors_origin_str,
                        body.len(),
                        body
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    return;
                }

                let handoff_action = ["status", "commit", "abort"].into_iter().find(|action| {
                    request.starts_with(&format!("POST /handoff/{action} "))
                });
                if request.starts_with("POST /add ") || handoff_action.is_some() {
                    if !token_valid {
                        let body = r#"{"error":"unauthorized_client","message":"Unauthorized client: pair the browser extension using your ARGON Bridge Key"}"#;
                        let response = format!(
                            "HTTP/1.1 401 Unauthorized\r\n{}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                            cors_origin_str,
                            body.len(),
                            body
                        );
                        let _ = socket.write_all(response.as_bytes()).await;
                        return;
                    }

                    if let Some(action) = handoff_action {
                        let request_id = request.split_once("\r\n\r\n")
                            .and_then(|(_, body)| serde_json::from_str::<serde_json::Value>(body).ok())
                            .and_then(|body| body["requestId"].as_str().map(str::to_owned));
                        let (status, body) = if let Some(request_id) = request_id {
                            if let Ok(dl) = ensure_downloader(&app).await {
                                match dl.browser_handoff(&request_id, action).await {
                                    Ok(Some(task)) => ("200 OK", serde_json::json!({
                                        "status": if task.awaiting_browser_commit { "prepared" } else { "accepted" },
                                        "taskId": task.id,
                                    })),
                                    Ok(None) => ("200 OK", serde_json::json!({ "status": "missing" })),
                                    Err(_) => ("503 Service Unavailable", serde_json::json!({ "error": "handoff_unavailable" })),
                                }
                            } else {
                                ("503 Service Unavailable", serde_json::json!({ "error": "handoff_unavailable" }))
                            }
                        } else {
                            ("400 Bad Request", serde_json::json!({ "error": "invalid_request_id" }))
                        };
                        let body = body.to_string();
                        let response = format!(
                            "HTTP/1.1 {status}\r\n{cors_origin_str}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                            body.len(),
                        );
                        let _ = socket.write_all(response.as_bytes()).await;
                        return;
                    }

                    if let Some(body_start) = request.find("\r\n\r\n") {
                        let json_part = &request[body_start + 4..];
                        if let Ok(mut payload) =
                            serde_json::from_str::<BrowserDownloadPayload>(json_part.trim())
                        {
                            let url_trim = payload.url.trim().to_owned();
                            let parsed_opt = reqwest::Url::parse(&url_trim).ok().filter(|url| {
                                matches!(url.scheme(), "http" | "https") && url.host_str().is_some()
                            });
                            let is_playlist = argon_downloader_core::resolver::is_stream_manifest(&url_trim)
                                || matches!(payload.stream_kind.as_deref(), Some("HLS" | "DASH"));

                            if parsed_opt.is_none() || (is_playlist && !has_ffmpeg) {
                                let body = r#"{"error":"unsupported_url","message":"Use a direct HTTP/HTTPS file URL; install ffmpeg for HLS/DASH streams"}"#;
                                let response = format!(
                                    "HTTP/1.1 400 Bad Request\r\n{}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                                    cors_origin_str,
                                    body.len(),
                                    body
                                );
                                let _ = socket.write_all(response.as_bytes()).await;
                                return;
                            }

                            if let Some(ref fname) = payload.filename {
                                payload.filename = Some(sanitize_filename(fname));
                            }

                            let Ok(dl) = ensure_downloader(&app).await else {
                                let _ = socket
                                    .write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n")
                                    .await;
                                return;
                            };

                            let media_info = is_playlist.then(|| MediaMetadata {
                                selected_format_id: Some("bestvideo+bestaudio/best".into()),
                                use_ytdlp_mux: Some(true),
                                ..Default::default()
                            });
                            let add_result = dl.add_browser_download(payload, media_info).await;

                            match add_result {
                                Ok(task) => {
                                    let body = serde_json::json!({
                                        "status": if task.awaiting_browser_commit { "prepared" } else { "accepted" },
                                        "taskId": task.id,
                                    })
                                    .to_string();
                                    let response = format!(
                                        "HTTP/1.1 200 OK\r\n{}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                                        cors_origin_str,
                                        body.len(),
                                        body
                                    );
                                    let _ = socket.write_all(response.as_bytes()).await;
                                }
                                Err(_) => {
                                    let body = r#"{"error":"handoff_not_saved","message":"ARGON could not save the task; keep the browser download"}"#;
                                    let response = format!(
                                        "HTTP/1.1 503 Service Unavailable\r\n{}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                                        cors_origin_str,
                                        body.len(),
                                        body
                                    );
                                    let _ = socket.write_all(response.as_bytes()).await;
                                }
                            }
                            return;
                        }
                    }
                    let body = r#"{"error":"invalid_json"}"#;
                    let response = format!(
                        "HTTP/1.1 400 Bad Request\r\n{}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                        cors_origin_str,
                        body.len(),
                        body
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    return;
                }

                let body = r#"{"error":"not_found"}"#;
                let response = format!(
                    "HTTP/1.1 404 Not Found\r\n{}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    cors_origin_str,
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
}

fn trigger_os_completion_action(action: &str, cancel: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::process::Command;
        if cancel {
            let _ = Command::new("shutdown").args(["/a"]).status();
        } else {
            match action {
                "sleep" => {
                    let _ = Command::new("rundll32.exe")
                        .args(["powrprof.dll,SetSuspendState", "0,1,0"])
                        .status();
                }
                "hibernate" => {
                    let _ = Command::new("shutdown").args(["/h"]).status();
                }
                _ => {
                    let _ = Command::new("shutdown")
                        .args(["/s", "/t", "60", "/c", "ARGON: Scheduled shutdown in 60s"])
                        .status();
                }
            }
        }
    }
    #[cfg(not(windows))]
    let _ = (action, cancel);
    Ok(())
}

#[allow(dead_code)]
fn trigger_os_shutdown(cancel: bool) -> Result<(), String> {
    trigger_os_completion_action("shutdown", cancel)
}

#[tauri::command]
async fn system_shutdown(
    app: AppHandle,
    action: Option<String>,
    cancel: bool,
) -> Result<(), String> {
    if cancel {
        return trigger_os_completion_action("shutdown", true);
    }
    let act = if let Some(a) = action {
        a
    } else {
        match ensure_downloader(&app).await {
            Ok(mgr) => mgr.settings().await.completion_action,
            Err(_) => "shutdown".to_string(),
        }
    };
    trigger_os_completion_action(&act, cancel)
}

#[tauri::command]
async fn open_browser_extension_dir(app: AppHandle) -> Result<(), String> {
    let ext_dir = app
        .path()
        .resource_dir()
        .map_err(|e| e.to_string())?
        .join("browser-extension");
    if ext_dir.exists() {
        app.opener()
            .open_path(ext_dir.to_string_lossy().to_string(), None::<&str>)
            .map_err(|e| e.to_string())
    } else {
        Err("Browser extension folder not found".to_string())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let task_manager = Arc::new(TaskManager::default());

    let builder = tauri::Builder::default();
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            tray::show_main_window(app);
        }))
        .plugin(
            tauri_plugin_updater::Builder::new()
                .target("windows-x86_64-nsis")
                .default_version_comparator(|current, release| {
                    release.version.pre.is_empty() && release.version > current
                })
                .build(),
        );
    builder
        .manage(task_manager)
        .manage(DownloaderRuntime::default())
        .manage(updater::UpdateRuntime::default())
        .manage(flow::FlowRuntime::default())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let persisted_token = load_or_create_bridge_token(app.handle());
            if let Ok(mut guard) = app.state::<Arc<TaskManager>>().bridge_token.write() {
                *guard = persisted_token;
            }
            #[cfg(target_os = "android")]
            app.handle().plugin(flow_mobile::init())?;
            #[cfg(desktop)]
            {
                tray::setup_tray(app.handle())?;
                if std::env::args().any(|a| a == "--minimized") {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.hide();
                    }
                }
                start_browser_bridge(app.handle().clone());
                start_clipboard_monitor(app.handle().clone());
                #[cfg(all(windows, desktop))]
                let _ = register_native_messaging_host(app.handle());
            }
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let _ = ensure_downloader(&handle).await;
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            #[cfg(desktop)]
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    let tm = window.app_handle().state::<Arc<TaskManager>>();
                    if !tm.updating.load(Ordering::SeqCst)
                        && tm.minimize_to_tray.load(Ordering::Relaxed)
                    {
                        api.prevent_close();
                        let _ = window.hide();
                    }
                }
            }
            #[cfg(not(desktop))]
            let _ = (window, event);
        })
        .invoke_handler(tauri::generate_handler![
            flow::flow_snapshot,
            flow::flow_start,
            flow::flow_stop,
            flow::flow_connect,
            flow::flow_connect_invited,
            flow::flow_pair_peer,
            flow::flow_confirm_pairing,
            flow::flow_respond_incoming,
            flow::flow_revoke_peer,
            flow::flow_cancel_transfer,
            flow::flow_send_files,
            flow::flow_send_download,
            flow::flow_retry_transfer,
            flow::flow_send_text,
            flow::flow_choose_files,
            flow::flow_choose_receive_directory,
            flow::flow_read_clipboard,
            flow::flow_write_clipboard,
            flow::flow_clean_text,
            flow::flow_open_received,
            flow::flow_copy_received,
            dl_snapshot,
            dl_add_task,
            dl_add_batch,
            dl_pause_task,
            dl_resume_task,
            dl_restart_task,
            dl_refresh_url,
            dl_pause_all,
            dl_resume_all,
            dl_remove_task,
            dl_clear_completed,
            dl_set_task_queue,
            dl_set_task_priority,
            dl_bump_task_queue,
            dl_set_task_speed_limit,
            dl_verify_hash,
            dl_update_settings,
            dl_migrate_legacy,
            probe_url,
            zip_peek_list,
            zip_peek_extract,
            hf_fetch_repo,
            antivirus_scan_file,
            repair_browser_integration,
            poll_telemetry_binary,
            get_default_download_directory,
            validate_download_directory,
            media::media_tools_status,
            media::install_ffmpeg_tool,
            media::update_ytdlp_tool,
            media::resolve_media,
            media::list_media_formats,
            media::save_cookies_text,
            media::load_cookies_text,
            set_browser_bridge_ready,
            browser_bridge_status,
            regenerate_bridge_token,
            open_media_sniffer,
            parse_torrent_info,
            open_browser_test,
            verify_file_sha256,
            open_destination_folder,
            open_file_item,
            share_file_item,
            scan_media_file,
            request_battery_exemption,
            sync_android_transfer_notification,
            delete_file_item,
            open_browser_extension_dir,
            system_shutdown,
            updater::update_status,
            updater::update_check,
            updater::update_download,
            updater::update_install,
            updater::fetch_release_feed
        ])
        .build(tauri::generate_context!())
        .expect("error while building ARGON")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                tauri::async_runtime::block_on(async {
                    if let Some(dl) = app.state::<DownloaderRuntime>().service.get() {
                        dl.shutdown().await;
                    }
                    let engine = app.state::<flow::FlowRuntime>().engine.lock().await.clone();
                    if let Some(engine) = engine {
                        engine.stop().await;
                    }
                });
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use argon_downloader_core::resolver::{parse_content_disposition_filename, percent_decode};

    #[test]
    fn test_sanitize_filename_delegated() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_filename("CON.txt"), "_CON.txt");
    }

    #[test]
    fn test_percent_decode_and_cd() {
        assert_eq!(percent_decode("hello%20world"), "hello world");
        let rfc5987 = "attachment; filename*=UTF-8''%D8%AA%D9%82%D8%B1%D9%8A%D8%B1.pdf";
        assert_eq!(
            parse_content_disposition_filename(rfc5987),
            Some("تقرير.pdf".to_string())
        );
    }

    #[test]
    fn test_browser_payload_deserialization() {
        let json = r#"{"url":"https://example.com/app.exe","filename":"app.exe","headers":{"Referer":"https://example.com/","Cookie":"sid=123"},"method":"POST","postBody":"x=1"}"#;
        let payload: BrowserDownloadPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.url, "https://example.com/app.exe");
        assert_eq!(payload.method.as_deref(), Some("POST"));
        assert_eq!(payload.post_body.as_deref(), Some("x=1"));
    }
}
