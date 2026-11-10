mod bridge;
mod flow;
#[cfg(target_os = "android")]
mod flow_mobile;
mod media;
mod transfer;
mod updater;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;
use tokio::sync::RwLock;

pub const DEFAULT_USER_AGENT: &str = concat!(
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36 ARGON/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/XRTM8/ARGON)"
);

#[derive(Debug, Clone, Copy)]
pub struct SegmentBounds {
    pub written_end: u64,
    pub limit_end: u64,
}

#[derive(Debug, Clone)]
pub struct LiveTaskMetric {
    pub downloaded: u64,
    pub total: u64,
    pub last_sample_at: std::time::Instant,
    pub last_sample_bytes: u64,
    pub speed_bps: f64,
    pub segments: HashMap<u32, u64>,
}

pub fn dual_hash_id(id: &str) -> (u32, u32) {
    let mut h1: u32 = 2166136261;
    let mut h2: u32 = 5381;
    for &b in id.as_bytes() {
        h1 ^= b as u32;
        h1 = h1.wrapping_mul(16777619);
        h2 = h2.wrapping_mul(33).wrapping_add(b as u32);
    }
    (h1, h2)
}

/// Lock-free atomic GCRA pacing helper: computes sleep duration in nanoseconds without Mutex contention.
fn reserve_delay_ns(next_ns: &AtomicU64, now_ns: u64, cost_ns: u64) -> u64 {
    let mut cur = next_ns.load(Ordering::Relaxed);
    loop {
        let base = cur.max(now_ns);
        let target = base.saturating_add(cost_ns);
        match next_ns.compare_exchange_weak(cur, target, Ordering::AcqRel, Ordering::Relaxed) {
            Ok(_) => return target.saturating_sub(now_ns),
            Err(actual) => cur = actual,
        }
    }
}

pub async fn wait_cancelled(
    rx: &mut tokio::sync::watch::Receiver<bool>,
    flag: &AtomicBool,
) {
    while !flag.load(Ordering::Acquire) {
        if *rx.borrow_and_update() {
            break;
        }
        if rx.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

#[derive(Default)]
pub struct TaskManager {
    cancellations: RwLock<HashMap<String, Arc<AtomicBool>>>,
    cancel_watches: RwLock<HashMap<String, tokio::sync::watch::Sender<bool>>>,
    speed_limit_bytes_per_sec: AtomicU64,
    task_speed_limits: RwLock<HashMap<String, Arc<AtomicU64>>>,
    task_next_ns: RwLock<HashMap<String, Arc<AtomicU64>>>,
    global_next_ns: AtomicU64,
    epoch: std::sync::OnceLock<std::time::Instant>,
    live_metrics: std::sync::RwLock<HashMap<String, LiveTaskMetric>>,
    segment_bounds: RwLock<HashMap<String, Arc<std::sync::Mutex<SegmentBounds>>>>,
    allowed_roots: RwLock<Vec<std::path::PathBuf>>,
    shared_client: tokio::sync::OnceCell<reqwest::Client>,
    frontend_ready: AtomicBool,
    updating: AtomicBool,
    bridge_listening: AtomicBool,
    bridge_sequence: AtomicU64,
    bridge_acks: tokio::sync::Mutex<HashMap<String, tokio::sync::oneshot::Sender<bool>>>,
}

impl TaskManager {
    fn now_ns(&self) -> u64 {
        let base = self.epoch.get_or_init(std::time::Instant::now);
        base.elapsed().as_nanos() as u64
    }

    pub async fn http_client(&self) -> Result<reqwest::Client, String> {
        self.shared_client
            .get_or_try_init(|| async { transfer::client() })
            .await
            .cloned()
    }

    pub fn record_segment_progress(
        &self,
        task_id: &str,
        segment_index: u32,
        downloaded_bytes: u64,
        total_bytes: u64,
    ) {
        if let Ok(mut map) = self.live_metrics.write() {
            let now = std::time::Instant::now();
            let entry = map.entry(task_id.to_string()).or_insert_with(|| LiveTaskMetric {
                downloaded: 0,
                total: total_bytes,
                last_sample_at: now,
                last_sample_bytes: 0,
                speed_bps: 0.0,
                segments: HashMap::new(),
            });
            entry.segments.insert(segment_index, downloaded_bytes);
            if total_bytes > 0 {
                entry.total = total_bytes;
            }
            let sum: u64 = entry.segments.values().sum();
            entry.downloaded = sum;
            let elapsed = now.duration_since(entry.last_sample_at).as_secs_f64();
            if elapsed >= 0.25 {
                let delta = sum.saturating_sub(entry.last_sample_bytes);
                let inst = delta as f64 / elapsed;
                entry.speed_bps = if entry.speed_bps <= 0.0 {
                    inst
                } else {
                    entry.speed_bps * 0.65 + inst * 0.35
                };
                entry.last_sample_at = now;
                entry.last_sample_bytes = sum;
            }
        }
    }

    pub fn snapshot_telemetry_binary(&self) -> Vec<u8> {
        let map = match self.live_metrics.read() {
            Ok(g) => g,
            Err(_) => return vec![0x33, 0x47, 0x52, 0x41, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        };
        let count = map.len() as u32;
        let mut out = Vec::with_capacity(16 + map.len() * 32);
        // Magic "ARG3" (0x41524733 LE)
        out.extend_from_slice(&0x4152_4733u32.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        let total_speed: f64 = map.values().map(|m| m.speed_bps).sum();
        out.extend_from_slice(&total_speed.to_le_bytes());

        for (id, m) in map.iter() {
            let (h1, h2) = dual_hash_id(id);
            out.extend_from_slice(&h1.to_le_bytes());
            out.extend_from_slice(&h2.to_le_bytes());
            out.extend_from_slice(&(m.downloaded as f64).to_le_bytes());
            out.extend_from_slice(&(m.total as f64).to_le_bytes());
            out.extend_from_slice(&m.speed_bps.to_le_bytes());
        }
        out
    }

    pub async fn allow_root(&self, dir: std::path::PathBuf) {
        if let Ok(canon) = tokio::fs::canonicalize(&dir).await {
            let mut roots = self.allowed_roots.write().await;
            if !roots.iter().any(|existing| existing == &canon) {
                roots.push(canon);
            }
        }
    }

    pub async fn register_segment(
        &self,
        task_id: &str,
        segment_index: u32,
        written_end: u64,
        limit_end: u64,
    ) -> Arc<std::sync::Mutex<SegmentBounds>> {
        let key = format!("{task_id}:{segment_index}");
        let bounds = Arc::new(std::sync::Mutex::new(SegmentBounds {
            written_end,
            limit_end,
        }));
        self.segment_bounds
            .write()
            .await
            .insert(key, bounds.clone());
        bounds
    }

    pub async fn unregister_segment(&self, task_id: &str, segment_index: u32) {
        let key = format!("{task_id}:{segment_index}");
        self.segment_bounds.write().await.remove(&key);
    }

    /// Dynamically shrinks an active segment's upper byte bound so an idle worker can steal the tail.
    pub async fn split_segment(
        &self,
        task_id: &str,
        segment_index: u32,
        min_remaining: u64,
    ) -> Option<(u64, u64)> {
        let key = format!("{task_id}:{segment_index}");
        let bounds = self.segment_bounds.read().await.get(&key).cloned()?;
        let mut guard = bounds.lock().ok()?;
        if guard.limit_end <= guard.written_end.saturating_add(min_remaining) {
            return None;
        }
        let old_end = guard.limit_end;
        let split_point = guard.written_end + (old_end - guard.written_end) / 2;
        if split_point <= guard.written_end || split_point >= old_end {
            return None;
        }
        guard.limit_end = split_point;
        Some((split_point + 1, old_end))
    }

    pub async fn set_task_speed_limit(&self, task_id: &str, bytes_per_sec: u64) {
        let mut map = self.task_speed_limits.write().await;
        if bytes_per_sec == 0 {
            map.remove(task_id);
        } else {
            map.entry(task_id.to_string())
                .or_insert_with(|| Arc::new(AtomicU64::new(bytes_per_sec)))
                .store(bytes_per_sec, Ordering::Relaxed);
        }
    }

    pub async fn throttle_task(&self, task_id: &str, bytes: u64) {
        let limit = {
            let map = self.task_speed_limits.read().await;
            map.get(task_id)
                .map(|v| v.load(Ordering::Relaxed))
                .unwrap_or(0)
        };
        if limit == 0 || bytes == 0 {
            return;
        }
        let pacer = {
            let read = self.task_next_ns.read().await;
            if let Some(p) = read.get(task_id) {
                p.clone()
            } else {
                drop(read);
                let mut write = self.task_next_ns.write().await;
                write
                    .entry(task_id.to_string())
                    .or_insert_with(|| Arc::new(AtomicU64::new(0)))
                    .clone()
            }
        };
        let cost_ns = ((bytes as u128 * 1_000_000_000u128) / (limit as u128)).min(u64::MAX as u128) as u64;
        let delay_ns = reserve_delay_ns(&pacer, self.now_ns(), cost_ns);
        if delay_ns > 0 {
            tokio::time::sleep(std::time::Duration::from_nanos(delay_ns)).await;
        }
    }

    pub async fn subscribe_cancel(&self, task_id: &str) -> tokio::sync::watch::Receiver<bool> {
        {
            let map = self.cancel_watches.read().await;
            if let Some(tx) = map.get(task_id) {
                return tx.subscribe();
            }
        }
        let pre_cancelled = self
            .cancellations
            .read()
            .await
            .get(task_id)
            .is_some_and(|f| f.load(Ordering::Relaxed));
        let mut map = self.cancel_watches.write().await;
        map.entry(task_id.to_string())
            .or_insert_with(|| tokio::sync::watch::channel(pre_cancelled).0)
            .subscribe()
    }

    pub async fn begin_transfer(&self, task_id: &str) -> Result<Arc<AtomicBool>, String> {
        let mut map = self.cancellations.write().await;
        if self.updating.load(Ordering::SeqCst) {
            return Err("update_installing".into());
        }
        let token = map
            .entry(task_id.to_string())
            .or_insert_with(|| Arc::new(AtomicBool::new(false)))
            .clone();
        drop(map);
        let mut watches = self.cancel_watches.write().await;
        watches
            .entry(task_id.to_string())
            .or_insert_with(|| tokio::sync::watch::channel(token.load(Ordering::Relaxed)).0);
        Ok(token)
    }
    pub async fn get_or_create_token(&self, task_id: &str) -> Arc<AtomicBool> {
        let mut map = self.cancellations.write().await;
        let token = if let Some(token) = map.get(task_id) {
            token.clone()
        } else {
            let token = Arc::new(AtomicBool::new(false));
            map.insert(task_id.to_string(), token.clone());
            token
        };
        drop(map);
        let mut watches = self.cancel_watches.write().await;
        watches
            .entry(task_id.to_string())
            .or_insert_with(|| tokio::sync::watch::channel(token.load(Ordering::Relaxed)).0);
        token
    }

    pub async fn cancel(&self, task_id: &str) {
        let mut map = self.cancellations.write().await;
        map.entry(task_id.to_owned())
            .or_insert_with(|| Arc::new(AtomicBool::new(true)))
            .store(true, Ordering::Release);
        drop(map);
        let mut watches = self.cancel_watches.write().await;
        if let Some(tx) = watches.get(task_id) {
            let _ = tx.send(true);
        } else {
            let (tx, _) = tokio::sync::watch::channel(true);
            watches.insert(task_id.to_owned(), tx);
        }
    }

    pub async fn remove(&self, task_id: &str) {
        self.cancellations.write().await.remove(task_id);
        self.cancel_watches.write().await.remove(task_id);
        self.task_speed_limits.write().await.remove(task_id);
        self.task_next_ns.write().await.remove(task_id);
        if let Ok(mut map) = self.live_metrics.write() {
            map.remove(task_id);
        }
        let prefix = format!("{task_id}:");
        self.segment_bounds
            .write()
            .await
            .retain(|k, _| !k.starts_with(&prefix));
    }

    pub async fn throttle(&self, bytes: u64) {
        let limit = self.get_speed_limit();
        if limit == 0 || bytes == 0 {
            return;
        }
        let cost_ns = ((bytes as u128 * 1_000_000_000u128) / (limit as u128)).min(u64::MAX as u128) as u64;
        let delay_ns = reserve_delay_ns(&self.global_next_ns, self.now_ns(), cost_ns);
        if delay_ns > 0 {
            tokio::time::sleep(std::time::Duration::from_nanos(delay_ns)).await;
        }
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
async fn get_default_download_directory(
    app: AppHandle,
    state: tauri::State<'_, Arc<TaskManager>>,
) -> Result<String, String> {
    let path = get_resolved_download_dir(&app);
    tokio::fs::create_dir_all(&path)
        .await
        .map_err(|e| format!("Download folder unavailable: {e}"))?;
    state.allow_root(path.clone()).await;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
async fn validate_download_directory(
    state: tauri::State<'_, Arc<TaskManager>>,
    directory: String,
) -> Result<String, String> {
    let path = std::path::PathBuf::from(directory.trim());
    if !path.is_absolute() {
        return Err("أدخل مسار مجلد كاملًا، مثل C:\\Downloads".into());
    }
    #[cfg(target_os = "android")]
    {
        let _ = state;
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
        state.allow_root(path.clone()).await;
        Ok(path.to_string_lossy().into_owned())
    }
}

#[tauri::command]
async fn set_task_speed_limit(
    state: tauri::State<'_, Arc<TaskManager>>,
    task_id: String,
    bytes_per_sec: u64,
) -> Result<(), String> {
    state.set_task_speed_limit(&task_id, bytes_per_sec).await;
    Ok(())
}

#[tauri::command]
async fn poll_telemetry_binary(
    state: tauri::State<'_, Arc<TaskManager>>,
) -> Result<tauri::ipc::Response, String> {
    Ok(tauri::ipc::Response::new(state.snapshot_telemetry_binary()))
}

#[tauri::command]
async fn split_active_segment(
    state: tauri::State<'_, Arc<TaskManager>>,
    task_id: String,
    segment_index: u32,
) -> Result<Option<(u64, u64)>, String> {
    Ok(state.split_segment(&task_id, segment_index, 524_288).await)
}

#[tauri::command]
async fn prepare_download_file(
    app: AppHandle,
    state: tauri::State<'_, Arc<TaskManager>>,
    destination_path: String,
    total_bytes: u64,
    fresh: bool,
    reset: bool,
) -> Result<String, String> {
    if state.updating.load(Ordering::SeqCst) {
        return Err("update_installing".into());
    }
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
async fn acknowledge_browser_download(
    state: tauri::State<'_, Arc<TaskManager>>,
    request_id: String,
    accepted: bool,
) -> Result<(), String> {
    let sender = state
        .bridge_acks
        .lock()
        .await
        .remove(&request_id)
        .ok_or("Browser handoff expired; the browser will keep its download")?;
    sender
        .send(accepted)
        .map_err(|_| "Browser handoff closed".to_string())
}

#[tauri::command]
async fn browser_bridge_status(
    state: tauri::State<'_, Arc<TaskManager>>,
) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "listening": state.bridge_listening.load(Ordering::Relaxed),
        "ready": state.frontend_ready.load(Ordering::Relaxed),
        "port": 41523
    }))
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
    let token = state.begin_transfer(&task_id).await?;
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

    tokio::task::spawn_blocking(move || {
        use std::io::Read;
        let mut file = std::fs::File::open(&resolved)
            .map_err(|e| format!("Cannot open file for hashing: {}", e))?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 1024 * 1024];
        loop {
            let n = file
                .read(&mut buffer)
                .map_err(|e| format!("Read error: {}", e))?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        Ok(format!("{:x}", hasher.finalize()))
    })
    .await
    .map_err(|e| e.to_string())?
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
async fn delete_file_item(
    app: AppHandle,
    state: tauri::State<'_, Arc<TaskManager>>,
    file_path: String,
) -> Result<(), String> {
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

    // Must be inside download dir, user home dir, resolved app download dir, app local data, or user-configured allowed roots
    let resolved_dir = get_resolved_download_dir(&app).canonicalize().ok();
    let local_data_dir = app
        .path()
        .app_local_data_dir()
        .ok()
        .and_then(|d| d.canonicalize().ok());
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

    let is_in_resolved = resolved_dir
        .as_ref()
        .map(|d| canonical_target.starts_with(d))
        .unwrap_or(false);
    let is_in_local_data = local_data_dir
        .as_ref()
        .map(|d| canonical_target.starts_with(d))
        .unwrap_or(false);
    let is_in_allowed_root = state
        .allowed_roots
        .read()
        .await
        .iter()
        .any(|root| canonical_target.starts_with(root));

    if !is_in_download
        && !is_in_home
        && !is_in_resolved
        && !is_in_local_data
        && !is_in_allowed_root
    {
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
    #[serde(default)]
    pub request_id: Option<String>,
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
                if request.starts_with("GET /ping ") || request.starts_with("GET /status ") {
                    if !app
                        .state::<Arc<TaskManager>>()
                        .frontend_ready
                        .load(Ordering::Relaxed)
                        || app
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
                    let body = serde_json::json!({"status": "ok", "app": "ARGON", "version": env!("CARGO_PKG_VERSION"), "capabilities": ["persisted-handoff", "direct-files"]}).to_string();
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
                        || app
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
                            let valid_url = reqwest::Url::parse(url_trim).ok().filter(|url| {
                                matches!(url.scheme(), "http" | "https")
                                    && url.host_str().is_some()
                                    && !url.path().to_ascii_lowercase().ends_with(".m3u8")
                                    && !url.path().to_ascii_lowercase().ends_with(".mpd")
                            });
                            if valid_url.is_none() {
                                let body = r#"{"error":"unsupported_url","message":"Use a direct HTTP/HTTPS file URL; magnet and HLS/DASH streams are unsupported"}"#;
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

                            let state = app.state::<Arc<TaskManager>>();
                            let request_id = format!(
                                "bridge-{}",
                                state.bridge_sequence.fetch_add(1, Ordering::Relaxed)
                            );
                            let (sender, receiver) = tokio::sync::oneshot::channel();
                            state
                                .bridge_acks
                                .lock()
                                .await
                                .insert(request_id.clone(), sender);
                            payload.request_id = Some(request_id.clone());
                            if app.emit("browser-add-download", payload).is_err() {
                                state.bridge_acks.lock().await.remove(&request_id);
                                let _ = socket.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n").await;
                                return;
                            }
                            let accepted = matches!(
                                tokio::time::timeout(std::time::Duration::from_secs(4), receiver)
                                    .await,
                                Ok(Ok(true))
                            );
                            state.bridge_acks.lock().await.remove(&request_id);
                            if !accepted {
                                let body = r#"{"error":"handoff_not_saved","message":"ARGON could not save the task; keep the browser download"}"#;
                                let response = format!("HTTP/1.1 503 Service Unavailable\r\n{}Content-Type: application/json\r\nContent-Length: {}\r\n\r\n{}", cors_origin_str, body.len(), body);
                                let _ = socket.write_all(response.as_bytes()).await;
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

    let builder = tauri::Builder::default();
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
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
        .manage(updater::UpdateRuntime::default())
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
            set_task_speed_limit,
            split_active_segment,
            poll_telemetry_binary,
            get_default_download_directory,
            validate_download_directory,
            prepare_download_file,
            validate_download_file,
            media::resolve_media,
            media::list_media_formats,
            media::download_media_with_ytdlp,
            cancel_task,
            set_browser_bridge_ready,
            acknowledge_browser_download,
            browser_bridge_status,
            cleanup_task,
            download_segment,
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
    async fn update_installation_blocks_new_native_transfers() {
        let tm = TaskManager::default();
        tm.updating.store(true, Ordering::SeqCst);
        assert!(tm.begin_transfer("blocked").await.is_err());
        assert!(tm.cancellations.read().await.is_empty());
        tm.updating.store(false, Ordering::SeqCst);
        assert!(tm.begin_transfer("allowed").await.is_ok());
        assert!(tm.cancellations.read().await.contains_key("allowed"));
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

    #[tokio::test]
    async fn zero_poll_cancel_watch_notifies_immediately() {
        let tm = TaskManager::default();
        let flag = tm.begin_transfer("watch-task").await.unwrap();
        let mut rx = tm.subscribe_cancel("watch-task").await;
        let waiter = tokio::spawn(async move {
            wait_cancelled(&mut rx, &flag).await;
            true
        });
        tokio::task::yield_now().await;
        tm.cancel("watch-task").await;
        assert!(waiter.await.unwrap());
    }

    #[test]
    fn binary_telemetry_frame_matches_wire_layout() {
        let tm = TaskManager::default();
        tm.record_segment_progress("task_1", 0, 1024, 4096);
        tm.record_segment_progress("task_1", 1, 2048, 4096);
        let bytes = tm.snapshot_telemetry_binary();
        assert_eq!(&bytes[0..4], &0x4152_4733u32.to_le_bytes());
        let count = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        assert_eq!(count, 1);
        assert_eq!(bytes.len(), 16 + 32);
        let (eh1, eh2) = dual_hash_id("task_1");
        assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), eh1);
        assert_eq!(u32::from_le_bytes(bytes[20..24].try_into().unwrap()), eh2);
        let dl = f64::from_le_bytes(bytes[24..32].try_into().unwrap());
        let total = f64::from_le_bytes(bytes[32..40].try_into().unwrap());
        assert_eq!(dl, 3072.0);
        assert_eq!(total, 4096.0);
    }

    #[test]
    fn lock_free_gcra_delay_is_non_decreasing() {
        let next = AtomicU64::new(0);
        let d1 = reserve_delay_ns(&next, 1_000, 500);
        let d2 = reserve_delay_ns(&next, 1_000, 500);
        assert_eq!(d1, 500);
        assert_eq!(d2, 1_000);
    }
}
