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
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::Mutex;

#[derive(Default)]
pub struct DownloaderRuntime {
    pub service: Mutex<Option<Arc<DownloaderService>>>,
}

pub struct TaskManager {
    pub frontend_ready: AtomicBool,
    pub updating: AtomicBool,
    pub bridge_listening: AtomicBool,
    pub minimize_to_tray: AtomicBool,
}

impl Default for TaskManager {
    fn default() -> Self {
        Self {
            frontend_ready: AtomicBool::new(false),
            updating: AtomicBool::new(false),
            bridge_listening: AtomicBool::new(false),
            minimize_to_tray: AtomicBool::new(true),
        }
    }
}

pub async fn ensure_downloader(app: &AppHandle) -> Result<Arc<DownloaderService>, String> {
    let runtime = app.state::<DownloaderRuntime>();
    let mut guard = runtime.service.lock().await;
    if let Some(ref existing) = *guard {
        return Ok(existing.clone());
    }
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
            DownloadEvent::AllIdleShutdownPending => {
                let _ = app_emit.emit("download-event", &event);
                let _ = trigger_os_shutdown(false);
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
    *guard = Some(service.clone());
    Ok(service)
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
async fn dl_set_updating(
    app: AppHandle,
    state: tauri::State<'_, Arc<TaskManager>>,
    active: bool,
) -> Result<(), String> {
    state.updating.store(active, Ordering::SeqCst);
    let svc = ensure_downloader(&app).await?;
    if active && svc.has_active_transfers().await {
        state.updating.store(false, Ordering::SeqCst);
        return Err("transfers_active".into());
    }
    svc.set_updating(active);
    if !active {
        svc.process_queue().await;
    }
    Ok(())
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
    let runtime = app.state::<DownloaderRuntime>();
    let guard = runtime.service.lock().await;
    let bytes = guard
        .as_ref()
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
async fn browser_bridge_status(
    app: AppHandle,
    state: tauri::State<'_, Arc<TaskManager>>,
) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "listening": state.bridge_listening.load(Ordering::Relaxed),
        "ready": true,
        "frontendMounted": state.frontend_ready.load(Ordering::Relaxed),
        "hasFfmpeg": media::has_ffmpeg_installed(&app),
        "port": 41523
    }))
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
                let client_header = request_head
                    .lines()
                    .find(|line| line.to_ascii_lowercase().starts_with("x-otium-client:"))
                    .and_then(|line| {
                        line.split_once(':')
                            .map(|(_, value)| value.trim().to_owned())
                    });

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
                        "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: {}\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization, X-Otium-Client\r\nAccess-Control-Max-Age: 86400\r\nContent-Length: 0\r\n\r\n",
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

                if request.starts_with("GET /ping ") || request.starts_with("GET /status ") {
                    let mut caps = vec!["persisted-handoff", "direct-files", "cookies-post"];
                    if has_ffmpeg {
                        caps.push("ffmpeg-hls");
                    }
                    let body = serde_json::json!({
                        "status": "ok",
                        "app": "ARGON",
                        "version": env!("CARGO_PKG_VERSION"),
                        "capabilities": caps
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

                if request.starts_with("POST /add ") {
                    let is_extension_origin = origin_header
                        .as_deref()
                        .map(|o| {
                            o.starts_with("chrome-extension://")
                                || o.starts_with("moz-extension://")
                                || o.starts_with("safari-web-extension://")
                        })
                        .unwrap_or(false);
                    let is_extension_client =
                        client_header.as_deref() == Some("browser-extension");

                    if !is_extension_origin && !is_extension_client {
                        let body = r#"{"error":"unauthorized_client","message":"Missing valid X-Otium-Client header or extension origin"}"#;
                        let response = format!(
                            "HTTP/1.1 401 Unauthorized\r\n{}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                            cors_origin_str,
                            body.len(),
                            body
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
                            let is_playlist = parsed_opt.as_ref().is_some_and(|u| {
                                let p = u.path().to_ascii_lowercase();
                                p.ends_with(".m3u8") || p.ends_with(".mpd")
                            });

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

                            let add_result = if is_playlist && has_ffmpeg {
                                dl.add_task(AddTaskRequest {
                                    url: url_trim,
                                    filename: payload.filename,
                                    segment_count: Some(4),
                                    expected_sha256: None,
                                    headers: payload.headers,
                                    post_body: payload.post_body,
                                    method: payload.method,
                                    media_info: Some(MediaMetadata {
                                        selected_format_id: Some("bestvideo+bestaudio/best".into()),
                                        use_ytdlp_mux: Some(true),
                                        ..Default::default()
                                    }),
                                    queue_id: Some(QueueId::Media),
                                    mirrors: None,
                                })
                                .await
                            } else {
                                dl.add_browser_download(payload).await
                            };

                            match add_result {
                                Ok(task) => {
                                    let body = serde_json::json!({
                                        "status": "accepted",
                                        "taskId": task.id,
                                        "message": "Download queued in ARGON"
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

fn trigger_os_shutdown(cancel: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::process::Command;
        if cancel {
            let _ = Command::new("shutdown").args(["/a"]).status();
        } else {
            let _ = Command::new("shutdown")
                .args(["/s", "/t", "60", "/c", "ARGON: Scheduled shutdown in 60s"])
                .status();
        }
    }
    #[cfg(not(windows))]
    let _ = cancel;
    Ok(())
}

#[tauri::command]
async fn system_shutdown(cancel: bool) -> Result<(), String> {
    trigger_os_shutdown(cancel)
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
            flow::flow_pair_peer,
            flow::flow_confirm_pairing,
            flow::flow_respond_incoming,
            flow::flow_revoke_peer,
            flow::flow_cancel_transfer,
            flow::flow_send_files,
            flow::flow_send_text,
            flow::flow_choose_files,
            flow::flow_choose_receive_directory,
            flow::flow_read_clipboard,
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
            dl_set_task_speed_limit,
            dl_verify_hash,
            dl_update_settings,
            dl_migrate_legacy,
            dl_set_updating,
            probe_url,
            poll_telemetry_binary,
            get_default_download_directory,
            validate_download_directory,
            media::media_tools_status,
            media::install_ffmpeg_tool,
            media::update_ytdlp_tool,
            media::resolve_media,
            media::list_media_formats,
            set_browser_bridge_ready,
            browser_bridge_status,
            verify_file_sha256,
            open_destination_folder,
            open_file_item,
            share_file_item,
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
                    let dl = app
                        .state::<DownloaderRuntime>()
                        .service
                        .lock()
                        .await
                        .clone();
                    if let Some(dl) = dl {
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
