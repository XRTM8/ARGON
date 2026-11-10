use crate::{SegmentProgressEvent, TaskManager, DEFAULT_USER_AGENT};
use futures_util::StreamExt;
use reqwest::{header, Client, StatusCode};
use serde::Serialize;
use std::{
    collections::HashMap,
    io,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot};

const WRITE_BUFFER_CAP: usize = 512 * 1024;
const COALESCED_FLUSH_THRESHOLD: usize = 256 * 1024;
const THROTTLE_BATCH_BYTES: u64 = 65_536;

#[derive(Debug, Serialize)]
pub struct TransferResult {
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub end_byte: u64,
}

fn write_all_at(file: &std::fs::File, mut offset: u64, mut data: &[u8]) -> io::Result<()> {
    #[cfg(windows)]
    use std::os::windows::fs::FileExt;
    #[cfg(unix)]
    use std::os::unix::fs::FileExt;

    while !data.is_empty() {
        #[cfg(windows)]
        let written = file.seek_write(data, offset)?;
        #[cfg(unix)]
        let written = file.write_at(data, offset)?;

        if written == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "failed to write whole buffer",
            ));
        }
        data = &data[written..];
        offset += written as u64;
    }
    Ok(())
}

enum WriteCmd {
    Truncate {
        reply: oneshot::Sender<Result<(), String>>,
    },
    Write {
        offset: u64,
        buf: Vec<u8>,
    },
    Flush {
        reply: oneshot::Sender<Result<(), String>>,
    },
    Sync {
        reply: oneshot::Sender<Result<(), String>>,
    },
}

struct CoalescedDiskWriter {
    tx: mpsc::Sender<WriteCmd>,
    recycle_rx: mpsc::UnboundedReceiver<Vec<u8>>,
    active_buf: Vec<u8>,
    buf_start: u64,
    allocated_buffers: usize,
}

impl CoalescedDiskWriter {
    fn open(path: &Path) -> Result<Self, String> {
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(|e| format!("Cannot open destination: {e}"))?;
        let (tx, mut rx) = mpsc::channel::<WriteCmd>(4);
        let (recycle_tx, recycle_rx) = mpsc::unbounded_channel::<Vec<u8>>();

        std::thread::Builder::new()
            .name("argon-disk".into())
            .spawn(move || {
                let mut sticky_err: Option<String> = None;
                while let Some(cmd) = rx.blocking_recv() {
                    match cmd {
                        WriteCmd::Truncate { reply } => {
                            let res = match file.set_len(0) {
                                Ok(()) => Ok(()),
                                Err(e) => {
                                    let msg = format!("Cannot reset destination: {e}");
                                    sticky_err = Some(msg.clone());
                                    Err(msg)
                                }
                            };
                            let _ = reply.send(res);
                        }
                        WriteCmd::Write { offset, mut buf } => {
                            if sticky_err.is_none() && !buf.is_empty() {
                                if let Err(e) = write_all_at(&file, offset, &buf) {
                                    sticky_err = Some(format!("Disk write failed: {e}"));
                                }
                            }
                            buf.clear();
                            let _ = recycle_tx.send(buf);
                        }
                        WriteCmd::Flush { reply } => {
                            let res = if let Some(ref err) = sticky_err {
                                Err(err.clone())
                            } else {
                                use std::io::Write;
                                (&file)
                                    .flush()
                                    .map_err(|e| format!("Disk flush failed: {e}"))
                            };
                            let _ = reply.send(res);
                        }
                        WriteCmd::Sync { reply } => {
                            let res = if let Some(ref err) = sticky_err {
                                Err(err.clone())
                            } else {
                                use std::io::Write;
                                (&file)
                                    .flush()
                                    .map_err(|e| format!("Disk flush failed: {e}"))
                                    .and_then(|()| {
                                        file.sync_all()
                                            .map_err(|e| format!("Disk sync failed: {e}"))
                                    })
                            };
                            let _ = reply.send(res);
                        }
                    }
                }
            })
            .map_err(|e| format!("Cannot start disk writer: {e}"))?;

        Ok(Self {
            tx,
            recycle_rx,
            active_buf: Vec::with_capacity(WRITE_BUFFER_CAP),
            buf_start: 0,
            allocated_buffers: 1,
        })
    }

    async fn take_buffer(&mut self) -> Vec<u8> {
        if let Ok(mut buf) = self.recycle_rx.try_recv() {
            buf.clear();
            return buf;
        }
        if self.allocated_buffers < 4 {
            self.allocated_buffers += 1;
            return Vec::with_capacity(WRITE_BUFFER_CAP);
        }
        // Pool saturated: asynchronously await a recycled buffer without blocking Tokio workers
        self.recycle_rx
            .recv()
            .await
            .map(|mut b| {
                b.clear();
                b
            })
            .unwrap_or_else(|| Vec::with_capacity(WRITE_BUFFER_CAP))
    }

    async fn truncate(&mut self) -> Result<(), String> {
        self.active_buf.clear();
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(WriteCmd::Truncate { reply })
            .await
            .map_err(|_| "Disk writer stopped".to_string())?;
        rx.await.map_err(|_| "Disk writer stopped".to_string())?
    }

    async fn push(&mut self, offset: u64, slice: &[u8]) -> Result<(), String> {
        if slice.is_empty() {
            return Ok(());
        }
        if !self.active_buf.is_empty() && self.buf_start + self.active_buf.len() as u64 != offset {
            self.flush_buffer().await?;
        }
        if self.active_buf.is_empty() {
            self.buf_start = offset;
        }
        self.active_buf.extend_from_slice(slice);
        if self.active_buf.len() >= COALESCED_FLUSH_THRESHOLD {
            self.flush_buffer().await?;
        }
        Ok(())
    }

    async fn flush_buffer(&mut self) -> Result<(), String> {
        if self.active_buf.is_empty() {
            return Ok(());
        }
        let next = self.take_buffer().await;
        let full = std::mem::replace(&mut self.active_buf, next);
        self.tx
            .send(WriteCmd::Write {
                offset: self.buf_start,
                buf: full,
            })
            .await
            .map_err(|_| "Disk writer stopped".to_string())
    }

    async fn flush(&mut self) -> Result<(), String> {
        self.flush_buffer().await?;
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(WriteCmd::Flush { reply })
            .await
            .map_err(|_| "Disk writer stopped".to_string())?;
        rx.await.map_err(|_| "Disk writer stopped".to_string())?
    }

    async fn sync(&mut self) -> Result<(), String> {
        self.flush_buffer().await?;
        let (reply, rx) = oneshot::channel();
        self.tx
            .send(WriteCmd::Sync { reply })
            .await
            .map_err(|_| "Disk writer stopped".to_string())?;
        rx.await.map_err(|_| "Disk writer stopped".to_string())?
    }
}

pub fn client() -> Result<Client, String> {
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

pub fn headers(custom: &Option<HashMap<String, String>>) -> Result<header::HeaderMap, String> {
    let mut result = header::HeaderMap::new();
    if let Some(map) = custom {
        for (key, value) in map {
            // Transfer framing belongs to the downloader, never to browser headers.
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

pub fn content_range(value: &str) -> Option<(u64, u64, u64)> {
    let rest = value.strip_prefix("bytes ")?;
    let (range, total) = rest.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let (start, end, total) = (start.parse().ok()?, end.parse().ok()?, total.parse().ok()?);
    if start > end || end >= total {
        return None;
    }
    Some((start, end, total))
}

pub struct Transfer<'a> {
    pub task_id: &'a str,
    pub url: &'a str,
    pub path: &'a Path,
    pub start: u64,
    pub end: u64,
    pub downloaded: u64,
    pub segment_index: u32,
    pub use_range: bool,
    pub expected_total: Option<u64>,
    pub validator: Option<&'a str>,
    pub custom_headers: &'a Option<HashMap<String, String>>,
}

pub async fn download(
    spec: Transfer<'_>,
    manager: &TaskManager,
    token: Arc<AtomicBool>,
    emit: impl Fn(SegmentProgressEvent),
) -> Result<TransferResult, String> {
    let parsed = reqwest::Url::parse(spec.url).map_err(|_| "Invalid URL")?;
    if !["http", "https"].contains(&parsed.scheme()) {
        return Err("Only HTTP and HTTPS URLs are allowed".into());
    }
    let client = manager.http_client().await?;
    let headers = headers(spec.custom_headers)?;
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
            manager
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

    let mut writer = CoalescedDiskWriter::open(spec.path)?;
    let mut cancel_rx = manager.subscribe_cancel(spec.task_id).await;
    let mut known_total = spec.expected_total.unwrap_or(0);
    let mut retries = 0u32;
    let max_retries = 5u32;
    let mut last_metric = std::time::Instant::now();
    let mut last_emit = std::time::Instant::now();
    let mut last_bytes = downloaded;
    let mut pending_throttle: u64 = 0;
    let result: Result<TransferResult, String> = loop {
        if token.load(Ordering::Relaxed) {
            break Err("CANCELLED".into());
        }
        if current_span().is_some_and(|size| downloaded >= size) {
            break Ok(TransferResult {
                downloaded_bytes: downloaded,
                total_bytes: known_total,
                end_byte: current_limit_end(),
            });
        }
        let req_end = current_limit_end();
        let mut request = client.get(parsed.clone()).headers(headers.clone());
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
            last_bytes = 0;
            if let Err(e) = writer.truncate().await {
                break Err(e);
            }
        }
        let response = tokio::select! {
            res = tokio::time::timeout(Duration::from_secs(30), request.send()) => res,
            _ = crate::wait_cancelled(&mut cancel_rx, &token) => break Err("CANCELLED".into()),
        };
        let response = match response {
            Ok(Ok(response)) => response,
            result => {
                retries += 1;
                if retries > max_retries {
                    break Err(format!(
                        "Connection failed: {}",
                        match result {
                            Ok(Err(e)) => e.to_string(),
                            _ => "response timeout".into(),
                        }
                    ));
                }
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_millis(250 * (1 << retries.min(5)))) => {},
                    _ = crate::wait_cancelled(&mut cancel_rx, &token) => break Err("CANCELLED".into()),
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
                _ = crate::wait_cancelled(&mut cancel_rx, &token) => break Err("CANCELLED".into()),
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
                .and_then(content_range);
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
        loop {
            if token.load(Ordering::Relaxed) {
                failed = true;
                break;
            }
            let next = tokio::select! {
                value = tokio::time::timeout(Duration::from_secs(30), stream.next()) => value,
                _ = crate::wait_cancelled(&mut cancel_rx, &token) => {
                    failed = true;
                    break;
                },
            };
            if token.load(Ordering::Relaxed) {
                failed = true;
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
                        if pending_throttle >= THROTTLE_BATCH_BYTES {
                            manager.throttle(pending_throttle).await;
                            manager.throttle_task(spec.task_id, pending_throttle).await;
                            pending_throttle = 0;
                        }
                    }
                    if hit_split_cap {
                        break;
                    }
                    if last_metric.elapsed() >= Duration::from_millis(33) {
                        manager.record_segment_progress(
                            spec.task_id,
                            spec.segment_index,
                            downloaded,
                            known_total,
                        );
                        last_metric = std::time::Instant::now();
                    }
                    if last_emit.elapsed() >= Duration::from_millis(200) {
                        emit(SegmentProgressEvent {
                            task_id: spec.task_id.into(),
                            segment_index: spec.segment_index,
                            delta_bytes: downloaded.saturating_sub(last_bytes),
                            downloaded_bytes: downloaded,
                            total_bytes: known_total,
                            is_finished: false,
                        });
                        last_bytes = downloaded;
                        last_emit = std::time::Instant::now();
                    }
                }
                Ok(None) => break,
                _ => {
                    failed = true;
                    break;
                }
            }
        }
        if pending_throttle > 0 {
            manager.throttle(pending_throttle).await;
            manager.throttle_task(spec.task_id, pending_throttle).await;
            pending_throttle = 0;
        }
        if token.load(Ordering::Relaxed) {
            break Err("CANCELLED".into());
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
            break Ok(TransferResult {
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
            _ = crate::wait_cancelled(&mut cancel_rx, &token) => break Err("CANCELLED".into()),
        }
    };
    if spec.use_range {
        manager.unregister_segment(spec.task_id, spec.segment_index).await;
    }
    let result = result?;
    writer.sync().await?;
    if token.load(Ordering::Relaxed) {
        return Err("CANCELLED".into());
    }
    manager.record_segment_progress(
        spec.task_id,
        spec.segment_index,
        downloaded,
        known_total,
    );
    emit(SegmentProgressEvent {
        task_id: spec.task_id.into(),
        segment_index: spec.segment_index,
        delta_bytes: downloaded.saturating_sub(last_bytes),
        downloaded_bytes: downloaded,
        total_bytes: known_total,
        is_finished: true,
    });
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    static NEXT: AtomicUsize = AtomicUsize::new(0);
    struct File(std::path::PathBuf);
    impl File {
        async fn new(size: u64) -> Self {
            let path = std::env::temp_dir().join(format!(
                "argon-transfer-{}-{}.bin",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            let file = tokio::fs::File::create(&path).await.unwrap();
            file.set_len(size).await.unwrap();
            Self(path)
        }
    }
    impl Drop for File {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }
    struct Server {
        url: String,
        task: tokio::task::JoinHandle<()>,
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.task.abort();
        }
    }
    async fn server(respond: impl Fn(&str) -> Vec<u8> + Send + Sync + 'static) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/file.bin", listener.local_addr().unwrap());
        let respond = Arc::new(respond);
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let respond = respond.clone();
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buffer = [0; 4096];
                    while !request.windows(4).any(|v| v == b"\r\n\r\n") {
                        let n = socket.read(&mut buffer).await.unwrap();
                        if n == 0 {
                            return;
                        }
                        request.extend_from_slice(&buffer[..n]);
                    }
                    let response = respond(&String::from_utf8_lossy(&request));
                    let _ = socket.write_all(&response).await;
                });
            }
        });
        Server { url, task }
    }
    fn response(status: &str, headers: &str, bytes: &[u8]) -> Vec<u8> {
        let mut out =
            format!("HTTP/1.1 {status}\r\nConnection: close\r\n{headers}\r\n").into_bytes();
        out.extend_from_slice(bytes);
        out
    }
    fn spec<'a>(server: &'a Server, file: &'a File) -> Transfer<'a> {
        Transfer {
            task_id: "test",
            url: &server.url,
            path: &file.0,
            start: 0,
            end: 0,
            downloaded: 0,
            segment_index: 0,
            use_range: false,
            expected_total: None,
            validator: None,
            custom_headers: &None,
        }
    }
    async fn run(spec: Transfer<'_>) -> Result<TransferResult, String> {
        download(
            spec,
            &TaskManager::default(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .await
    }
    #[tokio::test]
    async fn full_file_and_empty_file_are_written_exactly() {
        for body in [b"hello world".to_vec(), vec![]] {
            let expected = body.clone();
            let server = server(move |_| {
                response(
                    "200 OK",
                    &format!("Content-Length: {}\r\n", body.len()),
                    &body,
                )
            })
            .await;
            let file = File::new(128).await;
            let result = run(spec(&server, &file)).await.unwrap();
            assert_eq!(tokio::fs::read(&file.0).await.unwrap(), expected);
            assert_eq!(result.downloaded_bytes, expected.len() as u64);
        }
    }
    #[tokio::test]
    async fn chunked_unknown_size_downloads_real_bytes() {
        let server = server(|_| {
            response(
                "200 OK",
                "Transfer-Encoding: chunked\r\n",
                b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n",
            )
        })
        .await;
        let file = File::new(0).await;
        let result = run(spec(&server, &file)).await.unwrap();
        assert_eq!(result.total_bytes, 11);
        assert_eq!(tokio::fs::read(&file.0).await.unwrap(), b"hello world");
    }
    #[tokio::test]
    async fn concurrent_ranges_and_resume_preserve_file_hash() {
        let data: Vec<u8> = (0..4096).map(|i| (i % 251) as u8).collect();
        let sent = data.clone();
        let server = server(move |request| {
            let range = request
                .lines()
                .find_map(|line| {
                    line.to_lowercase()
                        .strip_prefix("range: bytes=")
                        .map(str::to_owned)
                })
                .unwrap();
            let (start, end) = range.split_once('-').unwrap();
            let (start, end): (usize, usize) = (start.parse().unwrap(), end.parse().unwrap());
            response(
                "206 Partial Content",
                &format!(
                    "Content-Length: {}\r\nContent-Range: bytes {start}-{end}/4096\r\n",
                    end - start + 1
                ),
                &sent[start..=end],
            )
        })
        .await;
        let file = File::new(4096).await;
        tokio::fs::write(&file.0, &data).await.unwrap();
        let mut first = spec(&server, &file);
        first.use_range = true;
        first.end = 2047;
        first.downloaded = 500;
        first.expected_total = Some(4096);
        let mut second = spec(&server, &file);
        second.use_range = true;
        second.start = 2048;
        second.end = 4095;
        second.expected_total = Some(4096);
        second.segment_index = 1;
        let (a, b) = tokio::join!(run(first), run(second));
        assert_eq!(
            a.unwrap().downloaded_bytes + b.unwrap().downloaded_bytes,
            4096
        );
        assert_eq!(
            Sha256::digest(tokio::fs::read(&file.0).await.unwrap()),
            Sha256::digest(data)
        );
    }
    #[tokio::test]
    async fn one_byte_range_is_not_an_open_stream() {
        let server = server(|_| {
            response(
                "206 Partial Content",
                "Content-Length: 1\r\nContent-Range: bytes 0-0/1\r\n",
                b"x",
            )
        })
        .await;
        let file = File::new(1).await;
        let mut s = spec(&server, &file);
        s.use_range = true;
        s.expected_total = Some(1);
        assert_eq!(run(s).await.unwrap().downloaded_bytes, 1);
    }
    #[tokio::test]
    async fn ignored_or_incorrect_ranges_fail_before_writing() {
        for headers in [
            "Content-Length: 3\r\n",
            "Content-Length: 3\r\nContent-Range: bytes 1-3/4\r\n",
        ] {
            let server = server(move |_| {
                response(
                    if headers.contains("Content-Range") {
                        "206 Partial Content"
                    } else {
                        "200 OK"
                    },
                    headers,
                    b"abc",
                )
            })
            .await;
            let file = File::new(3).await;
            let mut s = spec(&server, &file);
            s.use_range = true;
            s.end = 2;
            s.expected_total = Some(3);
            assert!(run(s).await.unwrap_err().contains("RANGE_UNSUPPORTED"));
            assert_eq!(tokio::fs::read(&file.0).await.unwrap(), vec![0; 3]);
        }
    }
    #[tokio::test]
    async fn http_error_and_html_are_never_completed() {
        for status in ["403 Forbidden", "404 Not Found", "200 OK"] {
            let server = server(move |_| {
                response(
                    status,
                    "Content-Length: 3\r\nContent-Type: text/html\r\n",
                    b"abc",
                )
            })
            .await;
            let file = File::new(0).await;
            assert!(run(spec(&server, &file)).await.is_err());
        }
    }
    #[tokio::test]
    async fn truncated_body_is_not_reported_as_complete() {
        let server = server(|_| response("200 OK", "Content-Length: 100\r\n", b"short")).await;
        let file = File::new(0).await;
        assert!(run(spec(&server, &file))
            .await
            .unwrap_err()
            .contains("interrupted"));
    }
    #[tokio::test]
    async fn cancellation_survives_late_segments() {
        let manager = TaskManager::default();
        manager.cancel("test").await;
        let token = manager.get_or_create_token("test").await;
        let server = server(|_| panic!("Cancelled transfer must not connect")).await;
        let file = File::new(0).await;
        assert_eq!(
            download(spec(&server, &file), &manager, token, |_| {})
                .await
                .unwrap_err(),
            "CANCELLED"
        );
    }
    #[tokio::test]
    async fn probe_checks_get_range_and_rejects_http_errors() {
        let server = server(|_| response("206 Partial Content", "Content-Length: 1\r\nContent-Range: bytes 0-0/2048\r\nETag: \"v1\"\r\nContent-Disposition: attachment; filename=real.bin\r\n", b"x")).await;
        let probe = crate::probe_resource(&server.url, &None).await.unwrap();
        assert!(probe.accept_ranges);
        assert_eq!(probe.content_length, Some(2048));
        assert_eq!(probe.etag.as_deref(), Some("\"v1\""));
        assert_eq!(probe.suggested_filename.as_deref(), Some("real.bin"));
        let denied = server_forbidden().await;
        assert!(crate::probe_resource(&denied.url, &None)
            .await
            .unwrap_err()
            .contains("403"));
    }
    async fn server_forbidden() -> Server {
        server(|_| response("403 Forbidden", "Content-Length: 0\r\n", b"")).await
    }
    use sha2::{Digest, Sha256};

    #[tokio::test]
    async fn retries_transient_503_and_completes_cleanly() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let counter = attempts.clone();
        let server = server(move |_| {
            let n = counter.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                response("503 Service Unavailable", "Retry-After: 0\r\nContent-Length: 0\r\n", b"")
            } else {
                response("200 OK", "Content-Length: 5\r\n", b"argon")
            }
        })
        .await;
        let file = File::new(0).await;
        let result = run(spec(&server, &file)).await.unwrap();
        assert_eq!(result.downloaded_bytes, 5);
        assert_eq!(tokio::fs::read(&file.0).await.unwrap(), b"argon");
    }

    #[tokio::test]
    async fn dynamic_segment_split_stops_cleanly_at_new_boundary() {
        let manager = Arc::new(TaskManager::default());
        let bounds = manager.register_segment("split-task", 0, 0, 1_048_575).await;
        {
            let mut guard = bounds.lock().unwrap();
            guard.written_end = 100_000;
        }
        let stolen = manager.split_segment("split-task", 0, 524_288).await.unwrap();
        let expected_split = 100_000 + (1_048_575 - 100_000) / 2;
        assert_eq!(stolen, (expected_split + 1, 1_048_575));
        assert_eq!(bounds.lock().unwrap().limit_end, expected_split);
        manager.unregister_segment("split-task", 0).await;
    }

    #[tokio::test]
    #[ignore = "Requires internet access"]
    async fn public_https_download_saves_valid_json() {
        let url = "https://raw.githubusercontent.com/XRTM8/ARGON/main/package.json";
        let probe = crate::probe_resource(url, &None).await.unwrap();
        let file = File::new(0).await;
        let result = download(
            Transfer {
                task_id: "https-test",
                url,
                path: &file.0,
                start: 0,
                end: 0,
                downloaded: 0,
                segment_index: 0,
                use_range: false,
                expected_total: probe.content_length,
                validator: None,
                custom_headers: &None,
            },
            &TaskManager::default(),
            Arc::new(AtomicBool::new(false)),
            |_| {},
        )
        .await
        .unwrap();
        let bytes = tokio::fs::read(&file.0).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(parsed["name"].is_string());
        assert_eq!(result.downloaded_bytes, bytes.len() as u64);
        assert_eq!(result.total_bytes, bytes.len() as u64);
    }
}
