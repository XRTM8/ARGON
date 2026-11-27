use super::{DownloaderService, MediaDownloadUpdate};
use crate::{
    disk::{self, CoalescedDiskWriter},
    model::*,
    resolver::{calculate_segments, detect_category, detect_platform, is_stream_manifest, same_resource_url},
    transfer::{
        download_segment, is_retryable_network_error, probe_resource, range_resource_key,
        strong_etag, verify_destination_size, SegmentOutcome, SegmentTransfer,
    },
};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::{sync::watch, task::JoinSet};

impl DownloaderService {
    pub(super) async fn execute_download(
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

        // 0. Optional Debrid unrestriction for locked hosts & torrents
        let mut _debrid_failure: Option<String> = None;
        if settings.debrid_enabled {
            if let Some(ref api_key) = settings.debrid_api_key {
                if !api_key.trim().is_empty()
                    && crate::debrid::is_debrid_supported_host(&task.original_url)
                {
                    let debrid_res = if crate::torrent::is_magnet_uri(&task.original_url) {
                        crate::debrid::unrestrict_magnet(
                            &self.http_client,
                            api_key,
                            &task.original_url,
                        )
                        .await
                    } else {
                        crate::debrid::unrestrict_link(
                            &self.http_client,
                            api_key,
                            &task.original_url,
                            None,
                        )
                        .await
                    };

                    match debrid_res {
                        Ok(unrestricted) => {
                            task.url = unrestricted.download_url;
                            if !task.custom_filename && !task.file_prepared {
                                task.filename =
                                    self.unique_filename(&task_id, &unrestricted.filename).await;
                                task.category = crate::resolver::detect_category(&task.filename);
                            }
                            if let Some(sz) = unrestricted.filesize {
                                if sz > 0 {
                                    task.total_bytes = sz;
                                }
                            }
                            self.sync_task_metadata(&task).await?;
                        }
                        Err(e) => {
                            _debrid_failure = Some(e);
                        }
                    }
                }
            }
        }

        // Catch unresolved torrent magnet URIs: fall back to native librqbit on desktop
        if crate::torrent::is_magnet_uri(&task.url) {
            #[cfg(not(target_os = "android"))]
            {
                let out_dir = self
                    .resolve_task_target_path(&task, &settings)
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| self.config.default_download_dir.clone());
                self.allow_root(out_dir.clone()).await;
                let session = self.torrent_manager.session().await?;
                let svc_ref = self.clone();
                let tid = task_id.clone();
                let (name, saved_path, total) = session
                    .download_magnet(
                        &task.url,
                        &out_dir,
                        cancel_flag.clone(),
                        cancel_rx.clone(),
                        move |downloaded, total, speed_bps| {
                            if let Ok(mut st) = svc_ref.state.try_write() {
                                if let Some(t) = st.tasks.get_mut(&tid) {
                                    t.downloaded_bytes = downloaded;
                                    if total > 0 {
                                        t.total_bytes = total;
                                    }
                                    t.speed_bytes_per_sec = speed_bps;
                                }
                            }
                        },
                    )
                    .await?;
                task.filename = name;
                task.save_path = Some(saved_path.to_string_lossy().into_owned());
                task.total_bytes = total;
                task.downloaded_bytes = total;
                task.status = TaskStatus::Completed;
                task.speed_bytes_per_sec = 0.0;
                task.eta_seconds = 0;
                task.completed_at = Some(now_ms());
                return Ok(task);
            }
            #[cfg(target_os = "android")]
            {
                let reason = _debrid_failure.unwrap_or_else(|| {
                    "تنزيل روابط التورنت على Android يتطلب تفعيل خدمة Real-Debrid وإدخال مفتاح API في الإعدادات".to_string()
                });
                return Err(format!("تعذر تنزيل التورنت: {reason}"));
            }
        }

        // 1. Optional platform media resolution / muxing
        let platform = detect_platform(&task.original_url);
        let stream_manifest = is_stream_manifest(&task.original_url);
        let selected_mux = task.media_info.as_ref()
            .filter(|media| media.use_ytdlp_mux == Some(true))
            .and_then(|media| media.selected_format_id.clone());
        let explicit_mux = stream_manifest || selected_mux.is_some();
        if explicit_mux && !self.media.as_ref().is_some_and(|media| media.supports_hd_mux()) {
            return Err("media_tools_required".into());
        }
        let has_media_info = task.media_info.is_some();
        let is_media_target = explicit_mux || has_media_info || !["direct", "other"].contains(&platform);
        if is_media_target {
            if let Some(ref media_hook) = self.media {
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
                    && is_media_target
                    && media_hook.supports_hd_mux();

                if explicit_mux || auto_hd_mux {
                    let fmt = selected_mux.unwrap_or_else(|| {
                        "bestvideo[ext=mp4]+bestaudio[ext=m4a]/bestvideo+bestaudio/best[ext=mp4]/best"
                            .to_owned()
                    });
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
                            self.sync_task_metadata(&task).await?;
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
                            Arc::new(move |update| match update {
                                MediaDownloadUpdate::Bytes(dl, total) => {
                                    svc_prog.telemetry.record_segment_progress(&tid, 0, dl, total);
                                }
                                MediaDownloadUpdate::Finalizing => {
                                    if let Ok(mut state) = svc_prog.state.try_write() {
                                        if let Some(task) = state.tasks.get_mut(&tid) {
                                            task.media_finalizing = Some(true);
                                        }
                                    }
                                    (svc_prog.emit)(DownloadEvent::StateChanged);
                                }
                            }),
                        )
                        .await?;
                    if !self.is_current_run(&task_id, run_id, &cancel_flag).await {
                        return Err("CANCELLED".into());
                    }
                    let size = tokio::fs::metadata(&saved).await
                        .map_err(|error| format!("Cannot read finished media file: {error}"))?.len();
                    if size == 0 {
                        return Err("Media tool produced an empty file".into());
                    }
                    if let Some(parent) = saved.parent() {
                        self.allow_root(parent.to_path_buf()).await;
                    }
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
        let resource_url = probe.final_url.as_deref();
        let resource_key = resource_url.map(range_resource_key);
        let use_range = !is_post
            && probe.accept_ranges
            && probe_len > 0
            && resource_url.is_some()
            && (probe.etag.as_deref().and_then(strong_etag).is_some()
                || probe.last_modified.is_some()
                || is_media_target);
        let resume = use_range
            && task.file_prepared
            && task.accept_ranges
            && task.downloaded_bytes > 0
            && task.total_bytes == probe_len
            && task.range_resource_key == resource_key
            && (task.etag == probe.etag
                || (task.etag.is_none() && probe.etag.is_none() && task.last_modified == probe.last_modified));
        if !resume {
            if task.downloaded_bytes > 0 {
                task.resume_notice = Some("resource-unverified".into());
            }
            task.downloaded_bytes = 0;
            let segs = if use_range {
                let host_cap = crate::host_limits::HostLimits::host_key(
                    resource_url.unwrap_or(&effective_url),
                )
                .map(|h| self.host_limits.effective_limit(&h, settings.max_connections_per_host))
                .unwrap_or(settings.max_connections_per_host);
                task.requested_segments.min(host_cap.max(1) as u32)
            } else {
                1
            };
            task.chunks = calculate_segments(probe_len, segs);
        }

        task.total_bytes = probe_len;
        task.accept_ranges = use_range;
        task.etag = probe.etag.clone();
        task.last_modified = probe.last_modified.clone();
        task.range_resource_key = resource_key.filter(|_| use_range);
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

        self.sync_task_metadata(&task).await?;

        let initial_pairs: Vec<(u32, u64)> =
            task.chunks.iter().map(|c| (c.index, c.downloaded)).collect();
        self.telemetry
            .init_task(&task_id, task.total_bytes, &initial_pairs);

        // 4. Execute transfer (Ranged parallel with work-stealing, falling back to single stream on RANGE_UNSUPPORTED)
        let transfer_url = probe
            .final_url
            .as_ref()
            .filter(|u| same_resource_url(u, &effective_url))
            .map_or(effective_url.as_str(), String::as_str);

        if use_range {
            match self
                .run_chunk_pool(
                    &mut task,
                    transfer_url,
                    &final_path,
                    resource_url,
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
                    task.range_resource_key = None;
                    task.resume_notice = Some("range-rejected".into());
                    task.downloaded_bytes = 0;
                    task.chunks = calculate_segments(0, 1);
                    self.sync_task_metadata(&task).await?;
                    self.telemetry.init_task(&task_id, task.total_bytes, &[(0, 0)]);
                    self.run_chunk_pool(
                        &mut task,
                        transfer_url,
                        &final_path,
                        None,
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
                transfer_url,
                &final_path,
                None,
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

        if settings.antivirus_enabled
            && crate::antivirus::should_scan(&settings.antivirus_extensions, &final_path)
        {
            match crate::antivirus::scan_file(&settings.antivirus_kind, None, &final_path).await {
                crate::antivirus::ScanResult::Infected { detail } => {
                    return Err(format!("Antivirus threat detected: {detail}"));
                }
                crate::antivirus::ScanResult::Error { detail } => {
                    log::warn!("Antivirus scan skipped/error for {}: {detail}", final_path.display());
                }
                crate::antivirus::ScanResult::Clean | crate::antivirus::ScanResult::Skipped => {}
            }
        }

        Ok(task)
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_chunk_pool(
        self: &Arc<Self>,
        task: &mut DownloadTask,
        url: &str,
        final_path: &Path,
        resource_url: Option<&str>,
        expected_total: Option<u64>,
        dynamic_work_stealing: bool,
        pacer: Arc<crate::throttle::TaskPacer>,
        cancel_flag: Arc<AtomicBool>,
        cancel_rx: watch::Receiver<bool>,
    ) -> Result<(), String> {
        let disk = CoalescedDiskWriter::open_with_budget(
            final_path,
            Some(self.memory_budget.clone()),
        )?;
        let use_range = resource_url.is_some();
        let validator = task.etag.clone();

        // If this is a fresh download with known size, preallocate the file on disk to eliminate fragmentation
        if use_range && task.total_bytes > 0 && task.downloaded_bytes == 0 {
            let _ = disk.preallocate(task.total_bytes).await;
        }

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
            let resource_url = resource_url.map(str::to_owned);
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
                        resource_url: resource_url.as_deref(),
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
                            let _ = self.sync_task_metadata(task).await;
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
        let disk_result = disk.sync().await;
        if errors.is_empty() && disk_result.is_ok() {
            disk.finish_sparse().await;
            return Ok(());
        }
        if !errors.is_empty() && task.chunks.len() > 1 {
            let throttled = errors.iter().any(|e| {
                e.contains("HTTP 429") || e.contains("HTTP 503") || e.contains("HTTP 403")
            });
            if throttled {
                if let Some(host) = crate::host_limits::HostLimits::host_key(url) {
                    self.host_limits.record_refusal(&host, task.chunks.len());
                }
            }
        }
        if disk_result.is_ok() {
            self.sync_chunks_from_telemetry(&mut task.chunks, &task.id);
        } else {
            task.range_resource_key = None;
        }
        {
            let mut state = self.state.write().await;
            if let Some(t) = state.tasks.get_mut(&task.id) {
                t.range_resource_key = task.range_resource_key.clone();
                t.chunks = task.chunks.clone();
                t.downloaded_bytes = t.chunks.iter().map(|c| c.downloaded).sum();
            }
        }
        disk_result?;
        let cause = errors
            .iter()
            .find(|e| e.contains("RANGE_UNSUPPORTED"))
            .or_else(|| errors.iter().find(|e| !e.contains("CANCELLED")))
            .unwrap_or(&errors[0])
            .clone();
        Err(cause)
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

    async fn sync_task_metadata(&self, snapshot_task: &DownloadTask) -> Result<(), String> {
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
                t.range_resource_key = snapshot_task.range_resource_key.clone();
                t.resume_notice = snapshot_task.resume_notice.clone();
                t.probed = snapshot_task.probed;
                t.file_prepared = snapshot_task.file_prepared;
                t.category = snapshot_task.category.clone();
                t.headers = snapshot_task.headers.clone();
            }
        }
        self.save_tasks().await?;
        (self.emit)(DownloadEvent::StateChanged);
        Ok(())
    }

    pub(super) async fn finalize_run(
        self: &Arc<Self>,
        id: &str,
        run_id: &str,
        outcome: Result<DownloadTask, String>,
    ) {
        let still_current = {
            let mut runs = self.active_runs.lock().await;
            if runs.get(id).is_some_and(|r| {
                r.run_id == run_id && !r.cancel_flag.load(Ordering::Acquire)
            }) {
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
                task.media_finalizing = None;
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

}
