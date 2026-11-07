use crate::{SegmentProgressEvent, TaskManager, DEFAULT_USER_AGENT};
use futures_util::StreamExt;
use reqwest::{header, Client, StatusCode};
use serde::Serialize;
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

#[derive(Debug, Serialize)]
pub struct TransferResult {
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
}

pub fn client() -> Result<Client, String> {
    Client::builder()
        .user_agent(DEFAULT_USER_AGENT)
        .redirect(reqwest::redirect::Policy::limited(15))
        .connect_timeout(Duration::from_secs(15))
        .tcp_keepalive(Some(Duration::from_secs(15)))
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
    let client = client()?;
    let headers = headers(spec.custom_headers)?;
    let mut downloaded = if spec.use_range { spec.downloaded } else { 0 };
    let bounded = spec
        .use_range
        .then(|| {
            spec.end
                .checked_sub(spec.start)
                .and_then(|n| n.checked_add(1))
        })
        .flatten()
        .ok_or_else(|| "Invalid byte range".to_string());
    let bounded = if spec.use_range { Some(bounded?) } else { None };
    if bounded.is_some_and(|size| downloaded > size) {
        return Err("Invalid resume offset".into());
    }
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(spec.path)
        .await
        .map_err(|e| format!("Cannot open destination: {e}"))?;
    let mut known_total = spec.expected_total.unwrap_or(0);
    let mut retries = 0;
    let mut last_emit = std::time::Instant::now();
    let mut last_bytes = downloaded;
    loop {
        if token.load(Ordering::Relaxed) {
            return Err("CANCELLED".into());
        }
        if bounded.is_some_and(|size| downloaded == size) {
            break;
        }
        let mut request = client.get(parsed.clone()).headers(headers.clone());
        if spec.use_range {
            request = request.header(
                header::RANGE,
                format!("bytes={}-{}", spec.start + downloaded, spec.end),
            );
            if let Some(validator) = spec.validator {
                request = request.header(header::IF_RANGE, validator);
            }
        } else {
            downloaded = 0;
            last_bytes = 0;
            file.set_len(0)
                .await
                .map_err(|e| format!("Cannot reset destination: {e}"))?;
        }
        let response = tokio::time::timeout(Duration::from_secs(30), request.send()).await;
        let response = match response {
            Ok(Ok(response)) => response,
            result => {
                retries += 1;
                if retries > 3 {
                    return Err(format!(
                        "Connection failed: {}",
                        match result {
                            Ok(Err(e)) => e.to_string(),
                            _ => "response timeout".into(),
                        }
                    ));
                }
                tokio::time::sleep(Duration::from_millis(250 * (1 << retries))).await;
                continue;
            }
        };
        if spec.use_range
            && [StatusCode::OK, StatusCode::RANGE_NOT_SATISFIABLE].contains(&response.status())
        {
            return Err("RANGE_UNSUPPORTED: server rejected the requested byte range".into());
        }
        if !response.status().is_success() {
            return Err(format!("Server returned HTTP {}", response.status()));
        }
        if response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("text/html") || v.starts_with("application/xhtml+xml"))
        {
            return Err("The server returned a webpage instead of a downloadable file".into());
        }
        if spec.use_range {
            let range = response
                .headers()
                .get(header::CONTENT_RANGE)
                .and_then(|v| v.to_str().ok())
                .and_then(content_range);
            if response.status() != StatusCode::PARTIAL_CONTENT
                || range != Some((spec.start + downloaded, spec.end, known_total))
            {
                return Err("RANGE_UNSUPPORTED: invalid Content-Range".into());
            }
        } else {
            if response.status() == StatusCode::PARTIAL_CONTENT {
                return Err("Unexpected partial response for full download".into());
            }
            if let Some(length) = response.content_length() {
                if spec.expected_total.is_some_and(|n| n != length) {
                    return Err("Remote file size changed; retry the download".into());
                }
                known_total = length;
            }
        }
        file.seek(std::io::SeekFrom::Start(if spec.use_range {
            spec.start + downloaded
        } else {
            0
        }))
        .await
        .map_err(|e| e.to_string())?;
        let mut stream = response.bytes_stream();
        let mut failed = false;
        loop {
            if token.load(Ordering::Relaxed) {
                return Err("CANCELLED".into());
            }
            // Poll cancellation while waiting on a stalled server.
            let next = tokio::select! {
                value = tokio::time::timeout(Duration::from_secs(30), stream.next()) => value,
                _ = async { while !token.load(Ordering::Relaxed) { tokio::time::sleep(Duration::from_millis(50)).await; } } => return Err("CANCELLED".into()),
            };
            match next {
                Ok(Some(Ok(bytes))) => {
                    if bounded.is_some_and(|size| downloaded + bytes.len() as u64 > size) {
                        return Err("Server sent bytes outside the requested range".into());
                    }
                    file.write_all(&bytes)
                        .await
                        .map_err(|e| format!("Disk write failed: {e}"))?;
                    downloaded += bytes.len() as u64;
                    manager.throttle(bytes.len() as u64).await;
                    if last_emit.elapsed() >= Duration::from_millis(100) {
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
        file.flush()
            .await
            .map_err(|e| format!("Disk flush failed: {e}"))?;
        if bounded.is_some_and(|size| downloaded != size)
            || (!spec.use_range && known_total > 0 && downloaded != known_total)
        {
            failed = true;
        }
        if !failed {
            break;
        }
        retries += 1;
        if retries > 3 {
            return Err("Download interrupted before all bytes arrived".into());
        }
        tokio::time::sleep(Duration::from_millis(250 * (1 << retries))).await;
    }
    file.sync_all()
        .await
        .map_err(|e| format!("Disk sync failed: {e}"))?;
    if token.load(Ordering::Relaxed) {
        return Err("CANCELLED".into());
    }
    emit(SegmentProgressEvent {
        task_id: spec.task_id.into(),
        segment_index: spec.segment_index,
        delta_bytes: downloaded.saturating_sub(last_bytes),
        downloaded_bytes: downloaded,
        total_bytes: known_total,
        is_finished: true,
    });
    Ok(TransferResult {
        downloaded_bytes: downloaded,
        total_bytes: if known_total > 0 {
            known_total
        } else {
            downloaded
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use tokio::{io::AsyncReadExt, net::TcpListener};

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
