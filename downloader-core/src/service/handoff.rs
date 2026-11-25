use super::{DownloaderService, EngineState};
use crate::{
    model::*,
    resolver::{
        calculate_segments, deduplicate_filename, detect_category, detect_platform,
        extract_filename_from_url, is_stream_manifest, resolve_universal_link, sanitize_filename,
    },
};
use std::sync::Arc;

impl DownloaderService {
    pub async fn add_task(self: &Arc<Self>, req: AddTaskRequest) -> Result<DownloadTask, String> {
        let task = Self::insert_task(&mut *self.state.write().await, req, false, None)?;
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
        media_info: Option<MediaMetadata>,
    ) -> Result<DownloadTask, String> {
        let id = payload.request_id.as_deref().map(browser_task_id).transpose()?;
        let queue_id = media_info.as_ref().map(|_| QueueId::Media);
        let req = AddTaskRequest {
            url: payload.url,
            filename: payload.filename,
            segment_count: None,
            expected_sha256: None,
            headers: payload.headers,
            post_body: payload.post_body,
            method: payload.method,
            media_info,
            queue_id,
            mirrors: None,
            queue_priority: None,
        };
        let task = {
            let _persist = self.persist_lock.lock().await;
            let mut state = self.state.write().await;
            if let Some(previous) = id.as_ref().and_then(|id| state.tasks.get(id)) {
                return Ok(previous.clone());
            }
            let task = Self::insert_task(&mut state, req, true, id)?;
            if let Err(error) = self.write_tasks(&state).await {
                state.tasks.remove(&task.id);
                return Err(error);
            }
            task
        };
        if !task.awaiting_browser_commit {
            (self.emit)(DownloadEvent::StateChanged);
            self.process_queue().await;
        }
        Ok(task)
    }

    pub async fn browser_handoff(
        self: &Arc<Self>,
        request_id: &str,
        action: &str,
    ) -> Result<Option<DownloadTask>, String> {
        let id = browser_task_id(request_id)?;
        if action == "status" {
            return Ok(self.state.read().await.tasks.get(&id).cloned());
        }
        let task = {
            let _persist = self.persist_lock.lock().await;
            let mut state = self.state.write().await;
            let Some(previous) = state.tasks.get(&id).cloned() else { return Ok(None); };
            if !previous.awaiting_browser_commit {
                return Ok(Some(previous));
            }
            let task = match action {
                "commit" => {
                    let mut task = previous.clone();
                    task.awaiting_browser_commit = false;
                    task.status = TaskStatus::Queued;
                    state.tasks.insert(id.clone(), task.clone());
                    Some(task)
                }
                "abort" => {
                    state.tasks.remove(&id);
                    None
                }
                _ => return Err("Invalid browser handoff action".into()),
            };
            if let Err(error) = self.write_tasks(&state).await {
                state.tasks.insert(id, previous);
                return Err(error);
            }
            task
        };
        if task.is_some() {
            (self.emit)(DownloadEvent::StateChanged);
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

    fn insert_task(
        state: &mut EngineState,
        req: AddTaskRequest,
        reject_invalid: bool,
        browser_id: Option<String>,
    ) -> Result<DownloadTask, String> {
        let original_url = req.url.trim().to_owned();
        let resolved = resolve_universal_link(&original_url);
        let is_magnet = crate::torrent::is_magnet_uri(&original_url);
        let url_valid = if is_magnet {
            true
        } else {
            reqwest::Url::parse(&resolved.resolved_url)
                .ok()
                .is_some_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
        };

        let mut error_message = if is_magnet {
            if !state.settings.debrid_enabled
                || state
                    .settings
                    .debrid_api_key
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or("")
                    .is_empty()
            {
                Some(
                    "تنزيل روابط التورنت وMagnet يتطلب تفعيل خدمة Real-Debrid في الإعدادات، أو استخدام رابط HTTP/HTTPS مباشر."
                        .to_owned(),
                )
            } else {
                None
            }
        } else if !url_valid {
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
            .unwrap_or_else(|| {
                if is_magnet {
                    crate::torrent::parse_magnet_uri(&original_url)
                        .map(|m| m.name)
                        .unwrap_or_else(|_| "magnet_download".to_owned())
                } else {
                    extract_filename_from_url(&resolved.resolved_url)
                }
            });
        let filename = deduplicate_filename(&sanitize_filename(&raw_name), &existing_names);
        let stream_manifest = is_stream_manifest(&original_url);

        let segments = if is_post {
            1
        } else {
            req.segment_count
                .unwrap_or(state.settings.default_segment_count)
                .clamp(1, 32)
        };

        let platform = detect_platform(&original_url);
        let queue_id = req.queue_id.unwrap_or_else(|| {
            if stream_manifest || !["direct", "other"].contains(&platform) {
                QueueId::Media
            } else {
                QueueId::Main
            }
        });

        let awaiting_browser_commit = browser_id.is_some();
        let id = browser_id.unwrap_or_else(|| new_id("task"));
        let task = DownloadTask {
            id: id.clone(),
            url: resolved.resolved_url,
            original_url,
            filename: filename.clone(),
            total_bytes: 0,
            downloaded_bytes: 0,
            status: if error_message.is_some() {
                TaskStatus::Error
            } else if awaiting_browser_commit {
                TaskStatus::Paused
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
            category: if is_magnet {
                TaskCategory::Torrents
            } else if stream_manifest || !["direct", "other"].contains(&platform) {
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
            range_resource_key: None,
            resume_notice: None,
            awaiting_browser_commit,
            media_finalizing: None,
            probed: false,
            queue_id,
            speed_limit_bytes_per_sec: 0,
            mirrors: req.mirrors.unwrap_or_default(),
            active_mirror_index: 0,
            retry_count: 0,
            queue_priority: req.queue_priority.unwrap_or(0),
        };
        state.tasks.insert(id, task.clone());
        Ok(task)
    }

}

fn browser_task_id(request_id: &str) -> Result<String, String> {
    uuid::Uuid::parse_str(request_id)
        .map(|id| format!("browser-{id}"))
        .map_err(|_| "Invalid browser handoff request ID".into())
}
