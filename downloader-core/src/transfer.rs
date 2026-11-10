use crate::{
    disk::{CoalescedDiskWriter, SegmentWriter},
    model::{UrlProbeResult, DEFAULT_USER_AGENT},
    resolver::{percent_decode, parse_content_disposition_filename, sanitize_filename},
    throttle::{wait_cancelled, TaskPacer, ThrottleController},
};
use futures_util::StreamExt;
use reqwest::{header, Client, Method, StatusCode};
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, RwLock,
    },
    time::{Duration, Instant},
};
use tokio::sync::watch;

const THROTTLE_BATCH_BYTES: u64 = 65_536;

#[derive(Debug, Clone, Copy)]
pub struct SegmentBounds {
    pub written_end: u64,
    pub limit_end: u64,
}

#[derive(Debug, Clone)]
pub struct LiveTaskMetric {
    pub downloaded: u64,
    pub total: u64,
    pub last_sample_at: Instant,
    pub last_progress_at: Instant,
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

#[derive(Default)]
pub struct TelemetryTracker {
    metrics: RwLock<HashMap<String, LiveTaskMetric>>,
    bounds: tokio::sync::RwLock<HashMap<String, Arc<Mutex<SegmentBounds>>>>,
}

impl TelemetryTracker {
    pub fn init_task(&self, task_id: &str, total_bytes: u64, initial_segments: &[(u32, u64)]) {
        if let Ok(mut map) = self.metrics.write() {
            let now = Instant::now();
            let mut segments = HashMap::new();
            let mut sum = 0u64;
            for &(idx, dl) in initial_segments {
                segments.insert(idx, dl);
                sum = sum.saturating_add(dl);
            }
            map.insert(
                task_id.to_owned(),
                LiveTaskMetric {
                    downloaded: sum,
                    total: total_bytes,
                    last_sample_at: now,
                    last_progress_at: now,
                    last_sample_bytes: sum,
                    speed_bps: 0.0,
                    segments,
                },
            );
        }
    }

    pub fn record_segment_progress(
        &self,
        task_id: &str,
        segment_index: u32,
        downloaded_bytes: u64,
        total_bytes: u64,
    ) {
        if let Ok(mut map) = self.metrics.write() {
            let now = Instant::now();
            let entry = map
                .entry(task_id.to_owned())
                .or_insert_with(|| LiveTaskMetric {
                    downloaded: 0,
                    total: total_bytes,
                    last_sample_at: now,
                    last_progress_at: now,
                    last_sample_bytes: 0,
                    speed_bps: 0.0,
                    segments: HashMap::new(),
                });
            let prev = entry.segments.insert(segment_index, downloaded_bytes);
            if total_bytes > 0 {
                entry.total = total_bytes;
            }
            let sum: u64 = entry.segments.values().sum();
            if sum > entry.downloaded || prev != Some(downloaded_bytes) {
                entry.last_progress_at = now;
            }
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

    pub fn effective_task_speed(&self, task_id: &str) -> Option<(u64, u64, f64)> {
        let map = self.metrics.read().ok()?;
        let m = map.get(task_id)?;
        Some((m.downloaded, m.total, decayed_speed(m)))
    }

    pub fn segment_progress_map(&self, task_id: &str) -> HashMap<u32, u64> {
        self.metrics
            .read()
            .ok()
            .and_then(|map| map.get(task_id).map(|m| m.segments.clone()))
            .unwrap_or_default()
    }

    pub fn snapshot_telemetry_binary(&self) -> Vec<u8> {
        let map = match self.metrics.read() {
            Ok(g) => g,
            Err(_) => return vec![0x33, 0x47, 0x52, 0x41, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        };
        let count = map.len() as u32;
        let mut out = Vec::with_capacity(16 + map.len() * 32);
        // Magic "ARG3" (0x41524733 LE)
        out.extend_from_slice(&0x4152_4733u32.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        let speeds: Vec<f64> = map.values().map(decayed_speed).collect();
        let total_speed: f64 = speeds.iter().sum();
        out.extend_from_slice(&total_speed.to_le_bytes());

        for ((id, m), speed) in map.iter().zip(speeds.into_iter()) {
            let (h1, h2) = dual_hash_id(id);
            out.extend_from_slice(&h1.to_le_bytes());
            out.extend_from_slice(&h2.to_le_bytes());
            out.extend_from_slice(&(m.downloaded as f64).to_le_bytes());
            out.extend_from_slice(&(m.total as f64).to_le_bytes());
            out.extend_from_slice(&speed.to_le_bytes());
        }
        out
    }

    pub async fn register_segment(
        &self,
        task_id: &str,
        segment_index: u32,
        written_end: u64,
        limit_end: u64,
    ) -> Arc<Mutex<SegmentBounds>> {
        let key = format!("{task_id}:{segment_index}");
        let bounds = Arc::new(Mutex::new(SegmentBounds {
            written_end,
            limit_end,
        }));
        self.bounds.write().await.insert(key, bounds.clone());
        bounds
    }

    pub async fn unregister_segment(&self, task_id: &str, segment_index: u32) {
        let key = format!("{task_id}:{segment_index}");
        self.bounds.write().await.remove(&key);
    }

    /// Finds the active segment of `task_id` with the largest remaining byte span > `min_remaining`
    /// and steals its second half cleanly.
    pub async fn steal_largest_segment(
        &self,
        task_id: &str,
        exclude_index: u32,
        min_remaining: u64,
    ) -> Option<(u32, u64, u64, u64)> {
        let prefix = format!("{task_id}:");
        let candidates: Vec<(u32, Arc<Mutex<SegmentBounds>>)> = {
            let map = self.bounds.read().await;
            map.iter()
                .filter_map(|(k, v)| {
                    let idx_str = k.strip_prefix(&prefix)?;
                    let idx = idx_str.parse::<u32>().ok()?;
                    if idx == exclude_index {
                        return None;
                    }
                    Some((idx, v.clone()))
                })
                .collect()
        };

        let mut best: Option<(u32, Arc<Mutex<SegmentBounds>>, u64)> = None;
        for (idx, bounds) in candidates {
            if let Ok(guard) = bounds.lock() {
                if guard.limit_end > guard.written_end.saturating_add(min_remaining) {
                    let rem = guard.limit_end - guard.written_end;
                    if best.as_ref().is_none_or(|(_, _, b_rem)| rem > *b_rem) {
                        best = Some((idx, bounds.clone(), rem));
                    }
                }
            }
        }

        let (victim_idx, bounds, _) = best?;
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
        Some((victim_idx, split_point, split_point + 1, old_end))
    }

    pub async fn clear_task(&self, task_id: &str) {
        if let Ok(mut map) = self.metrics.write() {
            map.remove(task_id);
        }
        let prefix = format!("{task_id}:");
        self.bounds
            .write()
            .await
            .retain(|k, _| !k.starts_with(&prefix));
    }
}

fn decayed_speed(m: &LiveTaskMetric) -> f64 {
    let idle = m.last_progress_at.elapsed().as_secs_f64();
    if idle < 1.0 {
        m.speed_bps
    } else if idle >= 3.0 {
        0.0
    } else {
        m.speed_bps * (1.0 - (idle - 1.0) / 2.0).max(0.0)
    }
}

pub fn build_http_client() -> Result<Client, String> {
    Client::builder()
        .user_agent(DEFAULT_USER_AGENT)
        .redirect(reqwest::redirect::Policy::limited(15))
        .connect_timeout(Duration::from_secs(15))
        .tcp_keepalive(Some(Duration::from_secs(15)))
        .pool_max_idle_per_host(16)
        .tcp_nodelay(true)
        .build()
        .map_err(|e| e.to_string())
}

pub fn build_headers(custom: Option<&HashMap<String, String>>) -> Result<header::HeaderMap, String> {
    let mut result = header::HeaderMap::new();
    if let Some(map) = custom {
        for (key, value) in map {
            if [
                "range",
                "if-range",
                "host",
                "content-length",
                "accept-encoding",
                "connection",
            ]
            .contains(&key.to_ascii_lowercase().as_str())
            {
                continue;
            }
            result.insert(
                header::HeaderName::from_bytes(key.as_bytes())
                    .map_err(|_| format!("Invalid header: {key}"))?,
                header::HeaderValue::from_str(value)
                    .map_err(|_| format!("Invalid header value: {key}"))?,
            );
        }
    }
    result.insert(
        header::ACCEPT_ENCODING,
        header::HeaderValue::from_static("identity"),
    );
    Ok(result)
}

pub fn parse_content_range(value: &str) -> Option<(u64, u64, u64)> {
    let rest = value.strip_prefix("bytes ")?;
    let (range, total) = rest.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let (start, end, total) = (start.parse().ok()?, end.parse().ok()?, total.parse().ok()?);
    if start > end || end >= total {
        return None;
    }
    Some((start, end, total))
}

pub async fn probe_resource(
    client: &Client,
    url: &str,
    custom: Option<&HashMap<String, String>>,
) -> Result<UrlProbeResult, String> {
    let parsed = reqwest::Url::parse(url.trim()).map_err(|_| "Invalid URL")?;
    if !["http", "https"].contains(&parsed.scheme()) {
        return Err("Only HTTP and HTTPS URLs are allowed".into());
    }
    let headers = build_headers(custom)?;
    let response = tokio::time::timeout(
        Duration::from_secs(30),
        client
            .get(parsed)
            .headers(headers)
            .header(header::RANGE, "bytes=0-0")
            .send(),
    )
    .await
    .map_err(|_| "URL probe timed out")?
    .map_err(|e| format!("URL probe failed: {e}"))?;

    let empty = response.status() == StatusCode::RANGE_NOT_SATISFIABLE
        && response
            .headers()
            .get(header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            == Some("bytes */0");
    if !response.status().is_success() && !empty {
        return Err(format!("Server returned HTTP {}", response.status()));
    }
    let final_url = Some(response.url().to_string());
    let h = response.headers();
    let range = h
        .get(header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_content_range);
    let accept_ranges = response.status() == StatusCode::PARTIAL_CONTENT
        && range.is_some_and(|(start, end, _)| start == 0 && end == 0);
    if response.status() == StatusCode::PARTIAL_CONTENT && !accept_ranges {
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
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let suggested_filename = h
        .get(header::CONTENT_DISPOSITION)
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
            .get(header::ETAG)
            .and_then(|v| v.to_str().ok())
            .filter(|v| !v.starts_with("W/"))
            .map(str::to_owned),
        last_modified: h
            .get(header::LAST_MODIFIED)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned),
        final_url,
    })
}

#[derive(Debug, Clone)]
pub struct SegmentOutcome {
    pub segment_index: u32,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub end_byte: u64,
}

pub struct SegmentTransfer<'a> {
    pub task_id: &'a str,
    pub url: &'a str,
    pub start: u64,
    pub end: u64,
    pub downloaded: u64,
    pub segment_index: u32,
    pub use_range: bool,
    pub expected_total: Option<u64>,
    pub validator: Option<&'a str>,
    pub custom_headers: Option<&'a HashMap<String, String>>,
    pub method: Option<&'a str>,
    pub post_body: Option<&'a str>,
}

pub async fn download_segment(
    spec: SegmentTransfer<'_>,
    client: &Client,
    disk: Arc<CoalescedDiskWriter>,
    telemetry: &TelemetryTracker,
    throttle: &ThrottleController,
    task_pacer: Option<Arc<TaskPacer>>,
    cancel_flag: Arc<AtomicBool>,
    mut cancel_rx: watch::Receiver<bool>,
    on_progress: impl Fn(u32, u64, u64, bool),
) -> Result<SegmentOutcome, String> {
    let parsed = reqwest::Url::parse(spec.url).map_err(|_| "Invalid URL")?;
    if !["http", "https"].contains(&parsed.scheme()) {
        return Err("Only HTTP and HTTPS URLs are allowed".into());
    }
    let headers = build_headers(spec.custom_headers)?;
    let mut downloaded = if spec.use_range { spec.downloaded } else { 0 };
    let initial_end = spec.end;
    let initial_span = if spec.use_range {
        Some(
            initial_end
                .checked_sub(spec.start)
                .and_then(|n| n.checked_add(1))
                .ok_or_else(|| "Invalid byte range".to_string())?,
        )
    } else {
        None
    };
    if initial_span.is_some_and(|size| downloaded > size) {
        return Err("Invalid resume offset".into());
    }

    let bounds = if spec.use_range {
        Some(
            telemetry
                .register_segment(
                    spec.task_id,
                    spec.segment_index,
                    spec.start + downloaded,
                    initial_end,
                )
                .await,
        )
    } else {
        None
    };

    let current_limit_end = || -> u64 {
        bounds
            .as_ref()
            .and_then(|b| b.lock().ok().map(|g| g.limit_end))
            .unwrap_or(initial_end)
    };
    let current_span = || -> Option<u64> {
        if !spec.use_range {
            return None;
        }
        let limit = current_limit_end();
        limit.checked_sub(spec.start).and_then(|n| n.checked_add(1))
    };

    let mut writer = SegmentWriter::new(disk);
    let mut known_total = spec.expected_total.unwrap_or(0);
    let mut retries = 0u32;
    let max_retries = 5u32;
    let mut last_emit = Instant::now();
    let mut pending_throttle: u64 = 0;

    let result: Result<SegmentOutcome, String> = loop {
        if cancel_flag.load(Ordering::Acquire) {
            break Err("CANCELLED".into());
        }
        if current_span().is_some_and(|size| downloaded >= size) {
            break Ok(SegmentOutcome {
                segment_index: spec.segment_index,
                downloaded_bytes: downloaded,
                total_bytes: known_total,
                end_byte: current_limit_end(),
            });
        }

        let req_end = current_limit_end();
        let http_method = if !spec.use_range && spec.method.is_some_and(|m| m.eq_ignore_ascii_case("POST")) {
            Method::POST
        } else {
            Method::GET
        };
        let mut request = client.request(http_method, parsed.clone()).headers(headers.clone());
        if !spec.use_range {
            if let Some(body) = spec.post_body {
                request = request.body(body.to_owned());
            }
        }
        if spec.use_range {
            request = request.header(
                header::RANGE,
                format!("bytes={}-{}", spec.start + downloaded, req_end),
            );
            if let Some(validator) = spec.validator {
                request = request.header(header::IF_RANGE, validator);
            }
        } else {
            downloaded = 0;
            if let Err(e) = writer.truncate().await {
                break Err(e);
            }
        }

        let response = tokio::select! {
            res = tokio::time::timeout(Duration::from_secs(30), request.send()) => res,
            _ = wait_cancelled(&mut cancel_rx, &cancel_flag) => break Err("CANCELLED".into()),
        };

        let response = match response {
            Ok(Ok(response)) => response,
            res => {
                retries += 1;
                if retries > max_retries {
                    break Err(format!(
                        "Connection failed: {}",
                        match res {
                            Ok(Err(e)) => e.to_string(),
                            _ => "response timeout".into(),
                        }
                    ));
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(250 * (1 << retries.min(5)))) => {},
                    _ = wait_cancelled(&mut cancel_rx, &cancel_flag) => break Err("CANCELLED".into()),
                }
                continue;
            }
        };

        if spec.use_range
            && [StatusCode::OK, StatusCode::RANGE_NOT_SATISFIABLE].contains(&response.status())
        {
            break Err("RANGE_UNSUPPORTED: server rejected the requested byte range".into());
        }

        if matches!(
            response.status(),
            StatusCode::TOO_MANY_REQUESTS
                | StatusCode::INTERNAL_SERVER_ERROR
                | StatusCode::BAD_GATEWAY
                | StatusCode::SERVICE_UNAVAILABLE
                | StatusCode::GATEWAY_TIMEOUT
        ) {
            retries += 1;
            if retries > max_retries {
                break Err(format!("Server returned HTTP {}", response.status()));
            }
            let retry_after_secs = response
                .headers()
                .get(header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok())
                .map(|s| s.min(10));
            let wait_ms = retry_after_secs
                .map(|s| s * 1000)
                .unwrap_or_else(|| 350 * (1 << retries.min(5)));
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(wait_ms)) => {},
                _ = wait_cancelled(&mut cancel_rx, &cancel_flag) => break Err("CANCELLED".into()),
            }
            continue;
        }

        if !response.status().is_success() {
            break Err(format!("Server returned HTTP {}", response.status()));
        }

        if response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/html") || v.starts_with("application/xhtml+xml"))
        {
            break Err("The server returned a webpage instead of a downloadable file".into());
        }

        if spec.use_range {
            let range = response
                .headers()
                .get(header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(parse_content_range);
            if response.status() != StatusCode::PARTIAL_CONTENT
                || range != Some((spec.start + downloaded, req_end, known_total))
            {
                break Err("RANGE_UNSUPPORTED: invalid Content-Range".into());
            }
        } else {
            if response.status() == StatusCode::PARTIAL_CONTENT {
                break Err("Unexpected partial response for full download".into());
            }
            if let Some(length) = response.content_length() {
                if spec.expected_total.is_some_and(|n| n != length) {
                    break Err("Remote file size changed; retry the download".into());
                }
                known_total = length;
            }
        }

        let mut stream = response.bytes_stream();
        let mut failed = false;
        let mut disk_error: Option<String> = None;
        let mut range_violation = false;
        let mut cancelled_during_stream = false;

        loop {
            if cancel_flag.load(Ordering::Acquire) {
                cancelled_during_stream = true;
                break;
            }
            let next = tokio::select! {
                value = tokio::time::timeout(Duration::from_secs(30), stream.next()) => value,
                _ = wait_cancelled(&mut cancel_rx, &cancel_flag) => {
                    cancelled_during_stream = true;
                    break;
                }
            };
            if cancel_flag.load(Ordering::Acquire) {
                cancelled_during_stream = true;
                break;
            }

            match next {
                Ok(Some(Ok(bytes))) => {
                    let current_offset = if spec.use_range {
                        spec.start + downloaded
                    } else {
                        downloaded
                    };
                    let (write_len, hit_split_cap, exceeded_original) = if let Some(ref b) = bounds {
                        let mut guard = match b.lock() {
                            Ok(g) => g,
                            Err(_) => {
                                disk_error = Some("Segment lock poisoned".into());
                                break;
                            }
                        };
                        let max_allowed = (guard.limit_end + 1).saturating_sub(current_offset);
                        if (bytes.len() as u64) > max_allowed {
                            if guard.limit_end == initial_end {
                                (0usize, true, true)
                            } else {
                                let take = max_allowed as usize;
                                guard.written_end = current_offset + take as u64;
                                (take, true, false)
                            }
                        } else {
                            guard.written_end = current_offset + bytes.len() as u64;
                            (bytes.len(), false, false)
                        }
                    } else {
                        (bytes.len(), false, false)
                    };

                    if exceeded_original {
                        range_violation = true;
                        break;
                    }
                    if write_len > 0 {
                        if let Err(e) = writer.push(current_offset, &bytes[..write_len]).await {
                            disk_error = Some(e);
                            break;
                        }
                        downloaded += write_len as u64;
                        pending_throttle += write_len as u64;
                        telemetry.record_segment_progress(
                            spec.task_id,
                            spec.segment_index,
                            downloaded,
                            known_total,
                        );
                        if pending_throttle >= THROTTLE_BATCH_BYTES {
                            if let Err(e) = throttle
                                .wait_throttle(
                                    task_pacer.as_deref(),
                                    pending_throttle,
                                    &mut cancel_rx,
                                    &cancel_flag,
                                )
                                .await
                            {
                                if e == "CANCELLED" {
                                    cancelled_during_stream = true;
                                } else {
                                    disk_error = Some(e);
                                }
                                break;
                            }
                            pending_throttle = 0;
                        }
                    }
                    if hit_split_cap {
                        break;
                    }
                    if last_emit.elapsed() >= Duration::from_millis(200) {
                        on_progress(spec.segment_index, downloaded, known_total, false);
                        last_emit = Instant::now();
                    }
                }
                Ok(None) => break,
                _ => {
                    failed = true;
                    break;
                }
            }
        }

        if cancelled_during_stream || cancel_flag.load(Ordering::Acquire) {
            let _ = writer.flush().await;
            break Err("CANCELLED".into());
        }
        if pending_throttle > 0 {
            if let Err(e) = throttle
                .wait_throttle(
                    task_pacer.as_deref(),
                    pending_throttle,
                    &mut cancel_rx,
                    &cancel_flag,
                )
                .await
            {
                let _ = writer.flush().await;
                break Err(e);
            }
            pending_throttle = 0;
        }
        if let Some(err) = disk_error {
            break Err(err);
        }
        if range_violation {
            break Err("Server sent bytes outside the requested range".into());
        }
        if let Err(e) = writer.flush().await {
            break Err(e);
        }
        if !failed
            && (current_span().is_some_and(|size| downloaded != size)
                || (!spec.use_range && known_total > 0 && downloaded != known_total))
        {
            failed = true;
        }
        if !failed {
            break Ok(SegmentOutcome {
                segment_index: spec.segment_index,
                downloaded_bytes: downloaded,
                total_bytes: if known_total > 0 {
                    known_total
                } else {
                    downloaded
                },
                end_byte: current_limit_end(),
            });
        }

        retries += 1;
        if retries > max_retries {
            break Err("Download interrupted before all bytes arrived".into());
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(250 * (1 << retries.min(5)))) => {},
            _ = wait_cancelled(&mut cancel_rx, &cancel_flag) => break Err("CANCELLED".into()),
        }
    };

    if spec.use_range {
        telemetry
            .unregister_segment(spec.task_id, spec.segment_index)
            .await;
    }

    let outcome = result?;
    writer.flush_buffer().await?;
    if cancel_flag.load(Ordering::Acquire) {
        return Err("CANCELLED".into());
    }
    telemetry.record_segment_progress(
        spec.task_id,
        spec.segment_index,
        outcome.downloaded_bytes,
        known_total,
    );
    on_progress(
        spec.segment_index,
        outcome.downloaded_bytes,
        outcome.total_bytes,
        true,
    );
    Ok(outcome)
}

pub fn is_retryable_network_error(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    if lower.contains("cancelled")
        || lower.contains("webpage")
        || lower.contains("sha-256")
        || lower.contains("disk")
        || lower.contains("allocate")
        || lower.contains("http 400")
        || lower.contains("http 401")
        || lower.contains("http 403")
        || lower.contains("http 404")
        || lower.contains("http 410")
    {
        return false;
    }
    lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("connection failed")
        || lower.contains("interrupted")
        || lower.contains("http 429")
        || lower.contains("http 500")
        || lower.contains("http 502")
        || lower.contains("http 503")
        || lower.contains("http 504")
}

/// Kept for compatibility with tests verifying individual transfer invariants directly.
pub async fn verify_destination_size(path: &Path, expected_bytes: u64) -> Result<(), String> {
    let size = tokio::fs::metadata(path)
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
