mod bridge;
mod flow;
#[cfg(target_os = "android")]
mod flow_mobile;
mod media;
mod transfer;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::RwLock;

pub const DEFAULT_USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36 ARGON/1.5 (+https://github.com/XRTM8/ARGON)";

#[derive(Default)]
pub struct TaskManager {
    cancellations: RwLock<HashMap<String, Arc<AtomicBool>>>,
    speed_limit_bytes_per_sec: AtomicU64,
    frontend_ready: AtomicBool,
    next_transfer: tokio::sync::Mutex<Option<tokio::time::Instant>>,
}

impl TaskManager {
    pub async fn get_or_create_token(&self, task_id: &str) -> Arc<AtomicBool> {
        let mut map = self.cancellations.write().await;
        if let Some(token) = map.get(task_id) {
            token.clone()
        } else {
            let token = Arc::new(AtomicBool::new(false));
            map.insert(task_id.to_string(), token.clone());
            token
        }
    }

    pub async fn cancel(&self, task_id: &str) {
        let mut map = self.cancellations.write().await;
        map.entry(task_id.to_owned())
            .or_insert_with(|| Arc::new(AtomicBool::new(true)))
            .store(true, Ordering::Relaxed);
    }

    pub async fn remove(&self, task_id: &str) {
        let mut map = self.cancellations.write().await;
        map.remove(task_id);
    }

    pub async fn throttle(&self, bytes: u64) {
        let limit = self.get_speed_limit();
        if limit == 0 {
            return;
        }
        let until = {
            let mut next = self.next_transfer.lock().await;
            let now = tokio::time::Instant::now();
            let until = next.unwrap_or(now).max(now)
                + std::time::Duration::from_secs_f64(bytes as f64 / limit as f64);
            *next = Some(until);
            until
        };
        tokio::time::sleep_until(until).await;
    }

    pub fn set_speed_limit(&self, bytes: u64) {
        self.speed_limit_bytes_per_sec
            .store(bytes, Ordering::Relaxed);
    }

    pub fn get_speed_limit(&self) -> u64 {
        self.speed_limit_bytes_per_sec.load(Ordering::Relaxed)
    }
}

pub fn sanitize_filename(raw: &str) -> String {
    // 1. Strip directories, keeping only the final segment
    let clean_name = raw
        .replace('\\', "/")
        .split('/')
        .filter(|s| !s.is_empty() && *s != "." && *s != "..")
        .last()
        .unwrap_or("download")
        .to_string();

    // 2. Map illegal chars on Windows/Linux to '_'
    let mut sanitized: String = clean_name
        .chars()
        .map(|c| {
            if matches!(
                c,
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0'..='\x1f'
            ) {
                '_'
            } else {
                c
            }
        })
        .collect();

    // 3. Trim whitespace, dots and underscores from edges
    let trimmed = sanitized.trim().trim_matches(|c| c == '.' || c == ' ');
    if trimmed.is_empty() {
        return "download".to_string();
    }
    sanitized = trimmed.to_string();

    // 4. Guard against Windows reserved names: CON, PRN, AUX, NUL, COM1-9, LPT1-9
    let stem = sanitized
        .split('.')
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    let is_reserved = matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    );

    if is_reserved {
        format!("_{}", sanitized)
    } else {
        sanitized
    }
}

fn percent_decode(input: &str) -> String {
    let mut bytes = Vec::new();
    let mut chars = input.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let h1 = chars.next();
            let h2 = chars.next();
            if let (Some(c1), Some(c2)) = (h1, h2) {
                let hex_str = [c1, c2];
                if let Ok(val) = std::str::from_utf8(&hex_str) {
                    if let Ok(byte) = u8::from_str_radix(val, 16) {
                        bytes.push(byte);
                        continue;
                    }
                }
                bytes.push(b'%');
                bytes.push(c1);
                bytes.push(c2);
            } else {
                bytes.push(b'%');
                if let Some(c1) = h1 {
                    bytes.push(c1);
                }
            }
        } else {
            bytes.push(b);
        }
    }
    String::from_utf8_lossy(&bytes).to_string()
}

fn parse_content_disposition_filename(cd: &str) -> Option<String> {
    // 1. Try filename*=UTF-8''... (RFC 5987 / RFC 6266)
    if let Some(pos) = cd.to_ascii_lowercase().find("filename*=") {
        let after = &cd[pos + 10..];
        let raw_val = after.split(';').next().unwrap_or("").trim();
        let encoded_val = if let Some(stripped) = raw_val
            .strip_prefix("UTF-8''")
            .or_else(|| raw_val.strip_prefix("utf-8''"))
        {
            stripped
        } else {
            raw_val
        };
        let decoded = percent_decode(encoded_val);
        let s = decoded.trim_matches(|c| c == '"' || c == '\'');
        if !s.is_empty() {
            return Some(sanitize_filename(s));
        }
    }

    // 2. Fallback to standard filename="..."
    if let Some(pos) = cd.to_ascii_lowercase().find("filename=") {
        let after = &cd[pos + 9..];
        let raw_val = after.split(';').next().unwrap_or("").trim();
        let clean = raw_val.trim_matches(|c| c == '"' || c == '\'');
        if !clean.is_empty() {
            return Some(sanitize_filename(clean));
        }
    }

    None
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UrlProbeResult {
    pub content_length: Option<u64>,
    pub accept_ranges: bool,
    pub content_type: Option<String>,
    pub suggested_filename: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentProgressEvent {
    pub task_id: String,
    pub segment_index: u32,
    pub delta_bytes: u64,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub is_finished: bool,
}

#[tauri::command]
async fn probe_url(
    url: String,
    custom_headers: Option<HashMap<String, String>>,
) -> Result<UrlProbeResult, String> {
    probe_resource(&url, &custom_headers).await
}

async fn probe_resource(
    url: &str,
    custom: &Option<HashMap<String, String>>,
) -> Result<UrlProbeResult, String> {
    let parsed = reqwest::Url::parse(url.trim()).map_err(|_| "Invalid URL")?;
    if !["http", "https"].contains(&parsed.scheme()) {
        return Err("Only HTTP and HTTPS URLs are allowed".into());
    }
    let client = transfer::client()?;
    let headers = transfer::headers(custom)?;
    // Verify range support with GET; Accept-Ranges on HEAD is often inaccurate.
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        client
            .get(parsed)
            .headers(headers)
            .header(reqwest::header::RANGE, "bytes=0-0")
            .send(),
    )
    .await
    .map_err(|_| "URL probe timed out")?
    .map_err(|e| format!("URL probe failed: {e}"))?;
    let empty = response.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE
        && response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            == Some("bytes */0");
    if !response.status().is_success() && !empty {
        return Err(format!("Server returned HTTP {}", response.status()));
    }
    let h = response.headers();
    let range = h
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(transfer::content_range);
    let accept_ranges = response.status() == reqwest::StatusCode::PARTIAL_CONTENT
        && range.is_some_and(|(start, end, _)| start == 0 && end == 0);
    if response.status() == reqwest::StatusCode::PARTIAL_CONTENT && !accept_ranges {
        return Err("Invalid probe Content-Range".into());
    }
    let content_length = if empty {
        Some(0)
    } else if accept_ranges {
        range.map(|(_, _, total)| total)
    } else {
        response.content_length()
    };
    let content_type = h
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let suggested_filename = h
        .get(reqwest::header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_content_disposition_filename)
        .or_else(|| {
            response
                .url()
                .path_segments()?
                .filter(|s| !s.is_empty())
                .next_back()
                .filter(|s| s.contains('.'))
                .map(|s| sanitize_filename(&percent_decode(s)))
        });
    Ok(UrlProbeResult {
        content_length,
        accept_ranges,
        content_type,
        suggested_filename,
        etag: h
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.starts_with("W/"))
            .map(str::to_owned),
        last_modified: h
            .get(reqwest::header::LAST_MODIFIED)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned),
    })
}

#[tauri::command]
async fn set_global_speed_limit(
    state: tauri::State<'_, Arc<TaskManager>>,
    bytes_per_sec: u64,
) -> Result<(), String> {
    state.set_speed_limit(bytes_per_sec);
    Ok(())
}

pub fn get_resolved_download_dir(app: &AppHandle) -> std::path::PathBuf {
    #[cfg(target_os = "android")]
    {
        // 1. Android external files directory: always writable without MANAGE_EXTERNAL_STORAGE
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

        // 2. Internal app local data directory: 100% accessible to the app sandbox
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

        // 3. Public Download dir if permission is granted
        let pub_dir = std::path::PathBuf::from("/storage/emulated/0/Download");
        if std::fs::create_dir_all(&pub_dir).is_ok() {
            let test_probe = pub_dir.join(".otium_write_probe");
            if std::fs::write(&test_probe, b"ok").is_ok() {
                let _ = std::fs::remove_file(&test_probe);
                return pub_dir;
            }
        }

        // 4. Cache directory fallback
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
        // On Android, always extract the leaf file name and place inside the verified writable base!
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
async fn get_default_download_directory(app: AppHandle) -> Result<String, String> {
    let path = get_resolved_download_dir(&app);
    tokio::fs::create_dir_all(&path)
        .await
        .map_err(|e| format!("Download folder unavailable: {e}"))?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
async fn validate_download_directory(directory: String) -> Result<String, String> {
    let path = std::path::PathBuf::from(directory.trim());
    if !path.is_absolute() {
        return Err("أدخل مسار مجلد كاملًا، مثل C:\\Downloads".into());
    }
    #[cfg(target_os = "android")]
    {
        return Err("تغيير مجلد الحفظ غير متاح على Android في هذا الإصدار".into());
    }
    #[cfg(not(target_os = "android"))]
    {
        tokio::fs::create_dir_all(&path)
            .await
            .map_err(|e| format!("Cannot create directory: {e}"))?;
        let test = path.join(format!(
            ".argon-write-check-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&test)
            .await
            .map_err(|e| format!("Download directory is not writable: {e}"))?;
        drop(file);
        tokio::fs::remove_file(&test)
            .await
            .map_err(|e| e.to_string())?;
        Ok(path.to_string_lossy().into_owned())
    }
}

#[tauri::command]
async fn prepare_download_file(
    app: AppHandle,
    destination_path: String,
    total_bytes: u64,
    fresh: bool,
    reset: bool,
) -> Result<String, String> {
    let mut target = resolve_destination_path(&app, &destination_path);
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Cannot create download folder: {e}"))?;
    }
    let file = if fresh {
        let original = target.clone();
        let stem = original.file_stem().unwrap_or_default().to_string_lossy();
        let ext = original
            .extension()
            .map(|v| format!(".{}", v.to_string_lossy()))
            .unwrap_or_default();
        let mut attempt = 0;
        loop {
            match tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&target)
                .await
            {
                Ok(file) => break file,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    attempt += 1;
                    target = original.with_file_name(format!("{stem} ({attempt}){ext}"));
                }
                Err(e) => return Err(format!("Cannot create destination: {e}")),
            }
        }
    } else {
        tokio::fs::OpenOptions::new()
            .write(true)
            .open(&target)
            .await
            .map_err(|e| format!("Cannot resume destination (restart the download): {e}"))?
    };
    if fresh || reset {
        file.set_len(total_bytes)
            .await
            .map_err(|e| format!("Cannot allocate download file: {e}"))?;
    } else if file.metadata().await.map_err(|e| e.to_string())?.len() != total_bytes {
        return Err("Local file size changed; restart the download".into());
    }
    Ok(target.to_string_lossy().into_owned())
}

#[tauri::command]
async fn validate_download_file(file_path: String, expected_bytes: u64) -> Result<(), String> {
    let size = tokio::fs::metadata(&file_path)
        .await
        .map_err(|e| e.to_string())?
        .len();
    if size != expected_bytes {
        return Err(format!(
            "Incomplete file: expected {expected_bytes} bytes, saved {size}"
        ));
    }
    Ok(())
}

#[tauri::command]
async fn set_browser_bridge_ready(state: tauri::State<'_, Arc<TaskManager>>) -> Result<(), String> {
    state.frontend_ready.store(true, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
async fn cancel_task(
    state: tauri::State<'_, Arc<TaskManager>>,
    task_id: String,
) -> Result<(), String> {
    state.cancel(&task_id).await;
    Ok(())
}

#[tauri::command]
async fn cleanup_task(
    state: tauri::State<'_, Arc<TaskManager>>,
    task_id: String,
) -> Result<(), String> {
    state.remove(&task_id).await;
    Ok(())
}

#[tauri::command]
async fn download_segment(
    app: AppHandle,
    state: tauri::State<'_, Arc<TaskManager>>,
    task_id: String,
    url: String,
    destination_path: String,
    start_byte: u64,
    end_byte: u64,
    current_downloaded: u64,
    segment_index: u32,
    custom_headers: Option<HashMap<String, String>>,
    use_range: bool,
    expected_total: Option<u64>,
    validator: Option<String>,
) -> Result<transfer::TransferResult, String> {
    let token = state.get_or_create_token(&task_id).await;
    let path = resolve_destination_path(&app, &destination_path);
    transfer::download(
        transfer::Transfer {
            task_id: &task_id,
            url: &url,
            path: &path,
            start: start_byte,
            end: end_byte,
            downloaded: current_downloaded,
            segment_index,
            use_range,
            expected_total,
            validator: validator.as_deref(),
            custom_headers: &custom_headers,
        },
        &state,
        token,
        |payload| {
            let _ = app.emit("download-progress", payload);
        },
    )
    .await
}

#[tauri::command]
async fn verify_file_sha256(app: AppHandle, file_path: String) -> Result<String, String> {
    let resolved = resolve_destination_path(&app, &file_path);
    if !resolved.exists() || !resolved.is_file() {
        return Err("Target file does not exist or is not a regular file".to_string());
    }

    let mut file = tokio::fs::File::open(&resolved)
        .await
        .map_err(|e| format!("Cannot open file for hashing: {}", e))?;

    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65536];

    use tokio::io::AsyncReadExt;
    loop {
        let n = file
            .read(&mut buffer)
            .await
            .map_err(|e| format!("Read error: {}", e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }

    let result = hasher.finalize();
    Ok(format!("{:x}", result))
}

#[tauri::command]
async fn open_destination_folder(app: AppHandle, file_path: String) -> Result<(), String> {
    let target = resolve_destination_path(&app, &file_path);
    if !target.exists() {
        return Err("Destination path does not exist".to_string());
    }
    let target_str = target.to_string_lossy().to_string();
    app.opener()
        .reveal_item_in_dir(&target_str)
        .map_err(|e| e.to_string())
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
    if !target_path.exists() {
        return Ok(());
    }
    if !target_path.is_file() {
        return Err("Security violation: destination must be a file, not a directory".to_string());
    }

    let canonical_target = target_path.canonicalize().map_err(|e| e.to_string())?;

    // Block deletion of critical system root and directories
    let target_str = canonical_target.to_string_lossy().to_ascii_lowercase();
    let forbidden_prefixes = [
        "c:\\windows",
        "c:\\program files",
        "c:\\program files (x86)",
        "/etc",
        "/usr",
        "/bin",
        "/sbin",
        "/lib",
        "/system",
        "/library",
    ];
    for forbidden in forbidden_prefixes {
        if target_str.starts_with(forbidden) {
            return Err("Security violation: cannot delete system protected files".to_string());
        }
    }

    // Must be in download dir or user home directory
    let is_in_download = app
        .path()
        .download_dir()
        .ok()
        .and_then(|d| d.canonicalize().ok())
        .map(|d| canonical_target.starts_with(&d))
        .unwrap_or(false);

    let is_in_home = app
        .path()
        .home_dir()
        .ok()
        .and_then(|h| h.canonicalize().ok())
        .map(|h| canonical_target.starts_with(&h))
        .unwrap_or(false);

    if !is_in_download && !is_in_home {
        return Err(
            "Security violation: target file is outside user allowed directories".to_string(),
        );
    }

    tokio::fs::remove_file(canonical_target)
        .await
        .map_err(|e| format!("Failed to delete file from disk: {}", e))?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserDownloadPayload {
    pub url: String,
    pub filename: Option<String>,
    pub headers: Option<HashMap<String, String>>,
}

#[cfg(desktop)]
fn start_browser_bridge(app_handle: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let addr = "127.0.0.1:41523";
        let listener = match tokio::net::TcpListener::bind(addr).await {
            Ok(l) => {
                log::info!("Otium Browser Bridge listening on http://{}", addr);
                l
            }
            Err(e) => {
                log::warn!("Could not bind Browser Bridge port {}: {}", addr, e);
                return;
            }
        };

        loop {
            let (mut socket, _) = match listener.accept().await {
                Ok(conn) => conn,
                Err(_) => continue,
            };

            let app = app_handle.clone();
            tauri::async_runtime::spawn(async move {
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
                // Extract Origin and Client headers
                let origin_header = request
                    .lines()
                    .find(|line| line.to_ascii_lowercase().starts_with("origin:"))
                    .map(|line| line[7..].trim().to_string());

                let client_header = request
                    .lines()
                    .find(|line| line.to_ascii_lowercase().starts_with("x-otium-client:"))
                    .and_then(|line| {
                        line.split_once(':')
                            .map(|(_, value)| value.trim().to_owned())
                    });

                // Reject all web origins (Anti-CSRF / Drive-by download prevention)
                if let Some(ref orig) = origin_header {
                    let lower = orig.to_ascii_lowercase();
                    if lower.starts_with("http://") || lower.starts_with("https://") {
                        let body = r#"{"error":"forbidden_origin","message":"Web origins are blocked from communicating with Otium loopback bridge"}"#;
                        let response = format!(
                            "HTTP/1.1 403 Forbidden\r\n\
                            Content-Type: application/json\r\n\
                            Content-Length: {}\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = socket.write_all(response.as_bytes()).await;
                        return;
                    }
                }

                // Handle CORS preflight
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
                            let response = "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n";
                            let _ = socket.write_all(response.as_bytes()).await;
                            return;
                        }
                    };

                    let response = format!(
                        "HTTP/1.1 204 No Content\r\n\
                        Access-Control-Allow-Origin: {}\r\n\
                        Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
                        Access-Control-Allow-Headers: Content-Type, Authorization, X-Otium-Client\r\n\
                        Access-Control-Max-Age: 86400\r\n\
                        Content-Length: 0\r\n\r\n",
                        allowed_origin
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    return;
                }

                // Echo origin if it is an extension, or omit if none
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

                // Handle GET /ping or GET /status
                if request.starts_with("GET /ping") || request.starts_with("GET /status") {
                    if !app
                        .state::<Arc<TaskManager>>()
                        .frontend_ready
                        .load(Ordering::Relaxed)
                    {
                        let _ = socket
                            .write_all(
                                b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n",
                            )
                            .await;
                        return;
                    }
                    let body = serde_json::json!({"status": "ok", "app": "ARGON", "version": env!("CARGO_PKG_VERSION")}).to_string();
                    let response = format!(
                        "HTTP/1.1 200 OK\r\n\
                        {}Content-Type: application/json\r\n\
                        Content-Length: {}\r\n\r\n{}",
                        cors_origin_str,
                        body.len(),
                        body
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    return;
                }

                // Handle POST /add
                if request.starts_with("POST /add ") {
                    if !app
                        .state::<Arc<TaskManager>>()
                        .frontend_ready
                        .load(Ordering::Relaxed)
                    {
                        let _ = socket
                            .write_all(
                                b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n",
                            )
                            .await;
                        return;
                    }
                    let is_extension_origin = origin_header
                        .as_deref()
                        .map(|o| {
                            o.starts_with("chrome-extension://")
                                || o.starts_with("moz-extension://")
                                || o.starts_with("safari-web-extension://")
                        })
                        .unwrap_or(false);

                    let is_extension_client = client_header.as_deref() == Some("browser-extension");

                    if !is_extension_origin && !is_extension_client {
                        let body = r#"{"error":"unauthorized_client","message":"Missing valid X-Otium-Client header or extension origin"}"#;
                        let response = format!(
                            "HTTP/1.1 401 Unauthorized\r\n\
                            {}Content-Type: application/json\r\n\
                            Content-Length: {}\r\n\r\n{}",
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
                            let url_trim = payload.url.trim();
                            // Validate protocol to prevent file:// or internal protocol abuse
                            if !url_trim.starts_with("http://")
                                && !url_trim.starts_with("https://")
                                && !url_trim.starts_with("magnet:?")
                            {
                                let body = r#"{"error":"invalid_protocol","message":"Only http, https, and magnet URIs are permitted"}"#;
                                let response = format!(
                                    "HTTP/1.1 400 Bad Request\r\n\
                                    {}Content-Type: application/json\r\n\
                                    Content-Length: {}\r\n\r\n{}",
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

                            if app.emit("browser-add-download", payload).is_err() {
                                let _ = socket.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n").await;
                                return;
                            }
                            let body =
                                r#"{"status":"accepted","message":"Download queued in ARGON"}"#;
                            let response = format!(
                                "HTTP/1.1 200 OK\r\n\
                                {}Content-Type: application/json\r\n\
                                Content-Length: {}\r\n\r\n{}",
                                cors_origin_str,
                                body.len(),
                                body
                            );
                            let _ = socket.write_all(response.as_bytes()).await;
                            return;
                        }
                    }
                    let body = r#"{"error":"invalid_json"}"#;
                    let response = format!(
                        "HTTP/1.1 400 Bad Request\r\n\
                        {}Content-Type: application/json\r\n\
                        Content-Length: {}\r\n\r\n{}",
                        cors_origin_str,
                        body.len(),
                        body
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                    return;
                }

                // 404 for unknown endpoints
                let body = r#"{"error":"not_found"}"#;
                let response = format!(
                    "HTTP/1.1 404 Not Found\r\n\
                    {}Content-Type: application/json\r\n\
                    Content-Length: {}\r\n\r\n{}",
                    cors_origin_str,
                    body.len(),
                    body
                );
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
}

#[tauri::command]
async fn system_shutdown(_cancel: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::process::Command;
        if _cancel {
            let _ = Command::new("shutdown").args(["/a"]).status();
        } else {
            let _ = Command::new("shutdown")
                .args(["/s", "/t", "60", "/c", "ARGON: Scheduled shutdown in 60s"])
                .status();
        }
    }
    Ok(())
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

    tauri::Builder::default()
        .manage(task_manager)
        .manage(flow::FlowRuntime::default())
        .plugin(tauri_plugin_opener::init())
        .setup(|_app| {
            #[cfg(target_os = "android")]
            _app.handle().plugin(flow_mobile::init())?;
            #[cfg(desktop)]
            {
                start_browser_bridge(_app.handle().clone());
            }
            Ok(())
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
            probe_url,
            set_global_speed_limit,
            get_default_download_directory,
            validate_download_directory,
            prepare_download_file,
            validate_download_file,
            media::resolve_media,
            cancel_task,
            set_browser_bridge_ready,
            cleanup_task,
            download_segment,
            verify_file_sha256,
            open_destination_folder,
            open_file_item,
            delete_file_item,
            open_browser_extension_dir,
            system_shutdown
        ])
        .build(tauri::generate_context!())
        .expect("error while building ARGON")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Exit) {
                tauri::async_runtime::block_on(async {
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

    #[test]
    fn test_sanitize_filename_traversal() {
        assert_eq!(sanitize_filename("../../etc/passwd"), "passwd");
        assert_eq!(
            sanitize_filename("..\\..\\Windows\\System32\\cmd.exe"),
            "cmd.exe"
        );
        assert_eq!(sanitize_filename("/var/log/syslog"), "syslog");
        assert_eq!(sanitize_filename(".."), "download");
        assert_eq!(sanitize_filename("."), "download");
    }

    #[test]
    fn test_sanitize_filename_reserved_windows() {
        assert_eq!(sanitize_filename("CON.txt"), "_CON.txt");
        assert_eq!(sanitize_filename("prn.zip"), "_prn.zip");
        assert_eq!(sanitize_filename("aux.tar.gz"), "_aux.tar.gz");
        assert_eq!(sanitize_filename("nul.dat"), "_nul.dat");
        assert_eq!(sanitize_filename("com1.bin"), "_com1.bin");
        assert_eq!(sanitize_filename("lpt3.log"), "_lpt3.log");
        assert_eq!(sanitize_filename("normal_file.txt"), "normal_file.txt");
    }

    #[test]
    fn test_sanitize_filename_illegal_chars() {
        assert_eq!(
            sanitize_filename("report:2026*final?.pdf"),
            "report_2026_final_.pdf"
        );
        assert_eq!(
            sanitize_filename("bad\"pipe\"|redir>.txt"),
            "bad_pipe__redir_.txt"
        );
    }

    #[test]
    fn test_sanitize_filename_fallback() {
        assert_eq!(sanitize_filename(""), "download");
        assert_eq!(sanitize_filename("   "), "download");
        assert_eq!(sanitize_filename("...."), "download");
    }

    #[test]
    fn test_percent_decode() {
        assert_eq!(percent_decode("hello%20world"), "hello world");
        assert_eq!(
            percent_decode("%D8%A7%D9%84%D8%B9%D8%B1%D8%A8%D9%8A%D8%A9.zip"),
            "العربية.zip"
        );
        // Malformed inputs should not panic
        assert_eq!(percent_decode("broken%2"), "broken%2");
        assert_eq!(percent_decode("broken%"), "broken%");
        assert_eq!(percent_decode("broken%ZZ"), "broken%ZZ");
    }

    #[test]
    fn test_parse_content_disposition_filename() {
        let rfc5987 = "attachment; filename*=UTF-8''%D8%AA%D9%82%D8%B1%D9%8A%D8%B1.pdf";
        assert_eq!(
            parse_content_disposition_filename(rfc5987),
            Some("تقرير.pdf".to_string())
        );

        let quoted = "attachment; filename=\"my_archive.tar.gz\"; size=1024";
        assert_eq!(
            parse_content_disposition_filename(quoted),
            Some("my_archive.tar.gz".to_string())
        );

        let unquoted = "attachment; filename=setup.exe";
        assert_eq!(
            parse_content_disposition_filename(unquoted),
            Some("setup.exe".to_string())
        );

        let traversal = "attachment; filename=\"../../danger.sh\"";
        assert_eq!(
            parse_content_disposition_filename(traversal),
            Some("danger.sh".to_string())
        );

        let none = "inline; size=4096";
        assert_eq!(parse_content_disposition_filename(none), None);
    }

    #[tokio::test]
    async fn test_task_manager_lifecycle() {
        let tm = TaskManager::default();

        // Speed limit
        assert_eq!(tm.get_speed_limit(), 0);
        tm.set_speed_limit(1_048_576);
        assert_eq!(tm.get_speed_limit(), 1_048_576);

        // Tokens
        let t1 = tm.get_or_create_token("t1").await;
        assert!(!t1.load(Ordering::Relaxed));

        // Cancellation
        tm.cancel("t1").await;
        assert!(t1.load(Ordering::Relaxed));

        // A late segment in the same attempt must remain cancelled.
        let t1_fresh = tm.get_or_create_token("t1").await;
        assert!(t1_fresh.load(Ordering::Relaxed));

        // Cleanup
        tm.remove("t1").await;
        let map = tm.cancellations.read().await;
        assert!(!map.contains_key("t1"));
    }

    #[test]
    fn test_browser_payload_deserialization() {
        let json = r#"{"url":"https://example.com/app.exe","filename":"app.exe","headers":{"Referer":"https://example.com/"}}"#;
        let payload: Result<BrowserDownloadPayload, _> = serde_json::from_str(json);
        assert!(payload.is_ok());
        let p = payload.unwrap();
        assert_eq!(p.url, "https://example.com/app.exe");
        assert_eq!(p.filename, Some("app.exe".to_string()));
        assert_eq!(
            p.headers.unwrap().get("Referer").unwrap(),
            "https://example.com/"
        );
    }

    #[test]
    fn test_browser_payload_protocol_validation() {
        let valid_http = "http://example.com/file.zip";
        let valid_https = "https://example.com/file.zip";
        let valid_magnet = "magnet:?xt=urn:btih:c12fe1c06bba254a9dc9f519b335de7ec72523ad";
        let invalid_file = "file:///etc/passwd";
        let invalid_script = "javascript:alert(1)";

        let is_valid = |url: &str| {
            let u = url.trim();
            u.starts_with("http://") || u.starts_with("https://") || u.starts_with("magnet:?")
        };

        assert!(is_valid(valid_http));
        assert!(is_valid(valid_https));
        assert!(is_valid(valid_magnet));
        assert!(!is_valid(invalid_file));
        assert!(!is_valid(invalid_script));
    }
}
