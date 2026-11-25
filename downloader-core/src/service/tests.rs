use super::*;
use sha2::{Digest, Sha256};
use std::sync::atomic::AtomicUsize;
use std::time::Duration;
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
    async fn queue_waits_when_installation_starts_while_it_is_acquiring_the_state_lock() {
        let root = TempDir::new();
        let server = spawn_server(|_| http_resp("200 OK", "Content-Length: 4\r\n", b"done")).await;
        let svc = new_service(&root).await;
        svc.set_updating(true);
        let task = svc.add_task(AddTaskRequest {
            url: format!("{}/queued.bin", server.base_url),
            filename: None,
            segment_count: None,
            expected_sha256: None,
            headers: None,
            method: None,
            post_body: None,
            media_info: None,
            queue_id: None,
            mirrors: None,
            queue_priority: None,
        }).await.unwrap();

        let state = svc.state.write().await;
        svc.set_updating(false);
        let queue = svc.clone();
        let pending = tokio::spawn(async move { queue.process_queue().await });
        tokio::task::yield_now().await;
        svc.set_updating(true);
        drop(state);
        pending.await.unwrap();
        assert!(!svc.has_active_transfers().await);
        assert_eq!(svc.snapshot().await.tasks[0].status, TaskStatus::Queued);

        svc.set_updating(false);
        svc.process_queue().await;
        assert_eq!(wait_settled(&svc, &task.id).await.status, TaskStatus::Completed);
        svc.shutdown().await;
    }

    struct TestMediaExtractor;

    impl MediaExtractor for TestMediaExtractor {
        fn supports_hd_mux(&self) -> bool { true }

        fn resolve_media(
            &self,
            _url: String,
            _audio_only: bool,
            _format_id: Option<String>,
        ) -> BoxFuture<'static, Result<ResolvedMediaSpec, String>> {
            Box::pin(async { Err("Expected the media downloader".into()) })
        }

        #[allow(clippy::too_many_arguments)]
    fn download_muxed_media(
            &self,
            _task_id: String,
            url: String,
            destination_path: PathBuf,
            format_spec: String,
            _cancel_flag: Arc<AtomicBool>,
            _cancel_rx: watch::Receiver<bool>,
            on_progress: Arc<dyn Fn(MediaDownloadUpdate) + Send + Sync>,
        ) -> BoxFuture<'static, Result<PathBuf, String>> {
            Box::pin(async move {
                if url.ends_with("/stream") && format_spec != "selected-quality" {
                    return Err("The selected media quality was lost".into());
                }
                let target = destination_path.with_extension("mp4");
                let parent = target.parent().ok_or("Missing destination directory")?;
                tokio::fs::create_dir_all(parent).await.map_err(|error| error.to_string())?;
                on_progress(MediaDownloadUpdate::Finalizing);
                tokio::fs::write(&target, b"completed mux").await.map_err(|error| error.to_string())?;
                Ok(target)
            })
        }
    }

    #[tokio::test]
    async fn manifest_urls_and_mime_identified_streams_reach_the_media_downloader() {
        let root = TempDir::new();
        let config = new_service(&root).await;
        config.shutdown().await;
        let service = DownloaderService::new(config.config.clone(), Some(Arc::new(TestMediaExtractor)), |_| {})
            .await.unwrap();
        for (path, media_info) in [
            ("/film.m3u8", None),
            ("/stream", Some(MediaMetadata {
                selected_format_id: Some("selected-quality".into()),
                use_ytdlp_mux: Some(true),
                ..Default::default()
            })),
        ] {
            let task = service.add_browser_download(BrowserDownloadPayload {
                url: format!("http://127.0.0.1{path}"),
                filename: None,
                headers: None,
                method: None,
                post_body: None,
                request_id: None,
                stream_kind: None,
            }, media_info).await.unwrap();
            let done = wait_settled(&service, &task.id).await;
            assert_eq!(done.status, TaskStatus::Completed, "{:?}", done.error_message);
            assert_eq!(done.total_bytes, b"completed mux".len() as u64);
            assert_eq!(done.media_finalizing, None);
            let path = service.completed_download_path(&task.id).await.unwrap();
            assert_eq!(std::fs::read(path).unwrap(), b"completed mux");
        }
        service.shutdown().await;
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
                stream_kind: None,
            }, None)
            .await
            .unwrap();
        let done = wait_settled(&svc, &task.id).await;
        assert_eq!(done.status, TaskStatus::Completed, "{:?}", done.error_message);
        assert_eq!(done.downloaded_bytes, data.len() as u64);
        assert!(svc.completed_download_path(&task.id).await.unwrap().is_file());
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
    async fn staged_browser_handoff_survives_restart_and_commits_only_once() {
        let root = TempDir::new();
        let requests = Arc::new(AtomicUsize::new(0));
        let server_requests = requests.clone();
        let server = spawn_server(move |_| {
            server_requests.fetch_add(1, Ordering::SeqCst);
            http_resp("200 OK", "Content-Length: 5\r\n", b"argon")
        }).await;
        let request_id = uuid::Uuid::new_v4().to_string();
        let payload = BrowserDownloadPayload {
            url: format!("{}/file.bin", server.base_url),
            filename: None,
            headers: None,
            method: None,
            post_body: None,
            request_id: Some(request_id.clone()),
            stream_kind: None,
        };
        let svc = new_service(&root).await;
        let (first, retry) = tokio::join!(
            svc.add_browser_download(payload.clone(), None),
            svc.add_browser_download(payload.clone(), None),
        );
        let first = first.unwrap();
        assert_eq!(first.id, retry.unwrap().id);
        assert!(first.awaiting_browser_commit);
        svc.resume_all().await.unwrap();
        assert!(svc.snapshot().await.tasks.is_empty());
        svc.shutdown().await;
        assert_eq!(requests.load(Ordering::SeqCst), 0);

        let svc = new_service(&root).await;
        let prepared = svc.browser_handoff(&request_id, "status").await.unwrap().unwrap();
        assert!(prepared.awaiting_browser_commit);
        assert_eq!(first.id, prepared.id);
        let (commit, retry) = tokio::join!(
            svc.browser_handoff(&request_id, "commit"),
            svc.browser_handoff(&request_id, "commit"),
        );
        for task in [commit, retry] {
            let task = task.unwrap().unwrap();
            assert_eq!(task.id, first.id);
            assert!(!task.awaiting_browser_commit);
        }
        let done = wait_settled(&svc, &first.id).await;
        assert_eq!(done.status, TaskStatus::Completed, "{:?}", done.error_message);
        assert_eq!(svc.snapshot().await.tasks.len(), 1);
        assert_eq!(requests.load(Ordering::SeqCst), 2);

        let abandoned_id = uuid::Uuid::new_v4().to_string();
        svc.add_browser_download(BrowserDownloadPayload {
            request_id: Some(abandoned_id.clone()), ..payload
        }, None).await.unwrap();
        assert!(svc.browser_handoff(&abandoned_id, "abort").await.unwrap().is_none());
        assert!(svc.browser_handoff(&abandoned_id, "status").await.unwrap().is_none());
        svc.shutdown().await;
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
                queue_priority: None,
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
                queue_priority: None,
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
        let ignored_range = Arc::new(AtomicBool::new(false));
        let ignored_range_in_server = ignored_range.clone();
        let srv = spawn_server(move |req| {
            let req = req.to_ascii_lowercase();
            if req.contains("range: bytes=0-0") {
                // Probe says 206 supported, then real segment returns 200 OK ignoring Range!
                http_resp(
                    "206 Partial Content",
                    &format!("Content-Length: 1\r\nContent-Range: bytes 0-0/{}\r\nETag: \"probe\"\r\n", data.len()),
                    &data[0..1],
                )
            } else {
                if req.contains("range: bytes=") {
                    ignored_range_in_server.store(true, Ordering::SeqCst);
                }
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
                queue_priority: None,
            })
            .await
            .unwrap();
        let done = wait_settled(&svc, &task.id).await;
        assert_eq!(done.status, TaskStatus::Completed, "{:?}", done.error_message);
        assert!(ignored_range.load(Ordering::SeqCst));
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
                queue_priority: None,
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
                queue_priority: None,
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
                queue_priority: None,
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
        assert_safe_resume(ResumeScenario::Unchanged).await;
    }

    #[tokio::test]
    async fn changed_etag_restarts_even_when_the_size_matches() {
        assert_safe_resume(ResumeScenario::NewVersion).await;
    }

    #[tokio::test]
    async fn replacement_url_restarts_even_when_the_etag_and_size_match() {
        assert_safe_resume(ResumeScenario::Replacement).await;
    }

    #[tokio::test]
    async fn legacy_partial_without_identity_restarts_after_upgrade() {
        assert_safe_resume(ResumeScenario::Legacy).await;
    }

    #[tokio::test]
    async fn renewed_link_that_resolves_to_the_same_file_preserves_the_partial() {
        assert_safe_resume(ResumeScenario::RenewedLink).await;
    }

    #[derive(PartialEq, Eq)]
    enum ResumeScenario {
        Unchanged,
        NewVersion,
        Replacement,
        Legacy,
        RenewedLink,
    }

    async fn assert_safe_resume(scenario: ResumeScenario) {
        let change_etag = scenario == ResumeScenario::NewVersion;
        let replace_url = scenario == ResumeScenario::Replacement;
        let root = TempDir::new();
        let data: Vec<u8> = (0..(256 * 1024)).map(|i| ((i * 31) % 251) as u8).collect();
        let data_clone = data.clone();
        let version_changed = Arc::new(AtomicBool::new(false));
        let server_version_changed = version_changed.clone();
        let resumed_at_start = Arc::new(AtomicBool::new(false));
        let server_resumed_at_start = resumed_at_start.clone();
        let resuming = Arc::new(AtomicBool::new(false));
        let server_resuming = resuming.clone();
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
                let mut payload = data_clone.clone();
                let version_changed = server_version_changed.load(Ordering::SeqCst);
                let resuming = server_resuming.load(Ordering::SeqCst);
                let resumed_at_start = server_resumed_at_start.clone();
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
                    if req_str.starts_with("GET /renewed?token=fresh ") {
                        let _ = socket.write_all(&http_resp(
                            "302 Found",
                            "Content-Length: 0\r\nLocation: /partial.bin\r\n",
                            b"",
                        )).await;
                        return;
                    }
                    let replacement = req_str.starts_with("GET /partial.bin?file=other ");
                    let etag = if version_changed { "\"p2\"" } else { "\"p1\"" };
                    if version_changed || replacement {
                        payload.iter_mut().for_each(|byte| *byte = !*byte);
                    }
                    let range = req_str.lines().find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("range: bytes=")
                            .map(str::to_owned)
                    });
                    let Some(r) = range else {
                        let _ = socket
                            .write_all(&http_resp(
                                "200 OK",
                                &format!("Content-Length: {}\r\nETag: {etag}\r\n", payload.len()),
                                &payload,
                            ))
                            .await;
                        return;
                    };
                    let (s_str, e_str) = r.split_once('-').unwrap();
                    let s: usize = s_str.parse().unwrap();
                    let e: usize = e_str.parse().unwrap();
                    if resuming && s == 0 && e > 0 {
                        resumed_at_start.store(true, Ordering::SeqCst);
                    }
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
                                "Content-Length: {}\r\nContent-Range: bytes {s}-{e}/{}\r\nETag: {etag}\r\n",
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
                queue_priority: None,
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

        let svc = if scenario == ResumeScenario::Legacy {
            let tasks_path = svc.config.state_dir.join("tasks.json");
            svc.shutdown().await;
            let mut tasks: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(&tasks_path).unwrap()).unwrap();
            assert!(tasks[0].as_object_mut().unwrap().remove("rangeResourceKey").is_some());
            std::fs::write(&tasks_path, serde_json::to_vec(&tasks).unwrap()).unwrap();
            new_service(&root).await
        } else {
            svc
        };
        version_changed.store(change_etag, Ordering::SeqCst);
        resuming.store(true, Ordering::SeqCst);
        if replace_url {
            svc.refresh_task_url(&task.id, format!("{base_url}/partial.bin?file=other"))
                .await
                .unwrap();
        } else if scenario == ResumeScenario::RenewedLink {
            svc.refresh_task_url(&task.id, format!("{base_url}/renewed?token=fresh"))
                .await
                .unwrap();
        } else {
            svc.resume_task(&task.id, false).await.unwrap();
        }
        let done = wait_settled(&svc, &task.id).await;
        assert_eq!(done.status, TaskStatus::Completed, "{:?}", done.error_message);
        assert_eq!(done.downloaded_bytes, data.len() as u64);
        assert_eq!(resumed_at_start.load(Ordering::SeqCst), change_etag || replace_url || scenario == ResumeScenario::Legacy);
        let expected = if change_etag || replace_url {
            data.into_iter().map(|byte| !byte).collect()
        } else {
            data
        };
        let saved = std::fs::read(done.save_path.unwrap()).unwrap();
        assert_eq!(Sha256::digest(saved), Sha256::digest(expected));
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
                queue_priority: None,
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
            range_resource_key: None,
            resume_notice: None,
            awaiting_browser_commit: false,
            media_finalizing: None,
            probed: true,
            queue_id: QueueId::Main,
            speed_limit_bytes_per_sec: 0,
            mirrors: Vec::new(),
            active_mirror_index: 0,
            retry_count: 0,
            queue_priority: 0,
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

    #[tokio::test]
    async fn queue_priority_picks_higher_priority_task_before_older_tasks() {
        let root = TempDir::new();
        let server = spawn_server(|_| http_resp("200 OK", "Content-Length: 4\r\n", b"done")).await;
        let svc = new_service(&root).await;

        svc.update_settings(DownloaderSettingsPatch {
            max_concurrent_downloads: Some(1),
            ..Default::default()
        }).await.unwrap();

        svc.set_updating(true);

        let _t1 = svc.add_task(AddTaskRequest {
            url: format!("{}/file1.bin", server.base_url),
            filename: Some("file1.bin".to_string()),
            segment_count: None,
            expected_sha256: None,
            headers: None,
            post_body: None,
            method: None,
            media_info: None,
            queue_id: Some(QueueId::Main),
            mirrors: None,
            queue_priority: Some(0),
        }).await.unwrap();

        let t2 = svc.add_task(AddTaskRequest {
            url: format!("{}/file2.bin", server.base_url),
            filename: Some("file2.bin".to_string()),
            segment_count: None,
            expected_sha256: None,
            headers: None,
            post_body: None,
            method: None,
            media_info: None,
            queue_id: Some(QueueId::Main),
            mirrors: None,
            queue_priority: Some(10),
        }).await.unwrap();

        svc.set_updating(false);
        svc.process_queue().await;

        let snap = svc.snapshot().await;
        let running = snap.tasks.iter().find(|t| t.status == TaskStatus::Downloading);
        assert!(running.is_some(), "One task should be downloading");
        assert_eq!(running.unwrap().id, t2.id, "Higher priority task should be picked first");

        svc.shutdown().await;
    }
