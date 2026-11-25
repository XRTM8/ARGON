use super::{ActiveRun, BoxFuture, DownloaderService};
use crate::{disk, model::*};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::watch;

impl DownloaderService {
    pub async fn process_queue(self: &Arc<Self>) {
        if self.destroyed.load(Ordering::Relaxed) || self.is_updating() {
            return;
        }
        let to_start: Vec<(DownloadTask, DownloaderSettings)> = {
            let mut state = self.state.write().await;
            if self.is_updating() {
                return;
            }
            let settings = state.settings.clone();
            let now_inst = Instant::now();
            let in_window = is_within_night_window(
                settings.night_scheduler_start_hour,
                settings.night_scheduler_end_hour,
            );

            let mut active_total = 0usize;
            let mut active_per_host: HashMap<String, usize> = HashMap::new();
            for t in state.tasks.values() {
                if t.status == TaskStatus::Downloading {
                    active_total += 1;
                    if let Some(h) = url_host(&t.url) {
                        *active_per_host.entry(h).or_insert(0) += 1;
                    }
                }
            }

            let mut queued: Vec<DownloadTask> = state
                .tasks
                .values()
                .filter(|t| t.status == TaskStatus::Queued)
                .cloned()
                .collect();
            queued.sort_by(|a, b| {
                b.queue_priority
                    .cmp(&a.queue_priority)
                    .then_with(|| a.created_at.cmp(&b.created_at))
            });

            let mut picked = Vec::new();
            for candidate in queued {
                if active_total >= settings.max_concurrent_downloads {
                    break;
                }
                if let Some(deadline) = state.retry_deadlines.get(&candidate.id) {
                    if *deadline > now_inst {
                        continue;
                    }
                }
                let forced = state.force_start_ids.contains(&candidate.id);
                if !forced
                    && (settings.night_scheduler_enabled || candidate.queue_id == QueueId::Night)
                    && !in_window
                {
                    continue;
                }
                let host = url_host(&candidate.url);
                if let Some(ref h) = host {
                    let count = active_per_host.get(h).copied().unwrap_or(0);
                    if count >= settings.max_connections_per_host {
                        continue;
                    }
                    *active_per_host.entry(h.clone()).or_insert(0) += 1;
                }
                if let Some(t) = state.tasks.get_mut(&candidate.id) {
                    t.status = TaskStatus::Downloading;
                    t.error_message = None;
                    t.media_finalizing = None;
                }
                state.retry_deadlines.remove(&candidate.id);
                state.force_start_ids.remove(&candidate.id);
                active_total += 1;
                picked.push((candidate, settings.clone()));
            }
            picked
        };

        if to_start.is_empty() {
            return;
        }
        let _ = self.save_tasks().await;
        (self.emit)(DownloadEvent::StateChanged);

        for (task, settings) in to_start {
            self.spawn_task_execution(task, settings).await;
        }
    }

    pub(super) fn spawn_task_execution<'a>(
        self: &'a Arc<Self>,
        task: DownloadTask,
        settings: DownloaderSettings,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let id = task.id.clone();
            self.cancel_run(&id).await;

            let run_id = uuid::Uuid::new_v4().to_string();
            let cancel_flag = Arc::new(AtomicBool::new(false));
            let (cancel_tx, cancel_rx) = watch::channel(false);
            let (finished_tx, finished) = watch::channel(());

            let task = {
                let state = self.state.read().await;
                let Some(task) = state.tasks.get(&id).filter(|task| task.status == TaskStatus::Downloading) else {
                    return;
                };
                let mut runs = self.active_runs.lock().await;
                runs.insert(
                    id.clone(),
                    ActiveRun {
                        run_id: run_id.clone(),
                        cancel_flag: cancel_flag.clone(),
                        cancel_tx,
                        finished,
                    },
                );
                task.clone()
            };

            let service = self.clone();
            tokio::spawn(async move {
                let outcome = service
                    .execute_download(task, settings, &run_id, cancel_flag.clone(), cancel_rx)
                    .await;
                drop(finished_tx);
                service.finalize_run(&id, &run_id, outcome).await;
            });
        })
    }

    pub(super) async fn cancel_run(&self, id: &str) {
        let (run_id, mut finished) = {
            let runs = self.active_runs.lock().await;
            let Some(run) = runs.get(id) else { return; };
            run.cancel_flag.store(true, Ordering::Release);
            let _ = run.cancel_tx.send(true);
            (run.run_id.clone(), run.finished.clone())
        };
        let _ = finished.changed().await;
        let mut runs = self.active_runs.lock().await;
        if runs.get(id).is_some_and(|run| run.run_id == run_id) {
            runs.remove(id);
        }
    }

    pub(super) async fn is_current_run(&self, id: &str, run_id: &str, cancel_flag: &AtomicBool) -> bool {
        if cancel_flag.load(Ordering::Acquire) || self.destroyed.load(Ordering::Relaxed) {
            return false;
        }
        self.active_runs
            .lock()
            .await
            .get(id)
            .is_some_and(|r| r.run_id == run_id)
    }

    pub(super) async fn background_ticker(self: &Arc<Self>) {
        let mut tick = tokio::time::interval(Duration::from_millis(500));
        loop {
            tick.tick().await;
            if self.destroyed.load(Ordering::Relaxed) {
                break;
            }
            let mut needs_queue = false;
            let mut active_changed = false;
            let mut structure_changed = false;
            let mut pause_outside_window = Vec::new();

            let should_update = {
                let state = self.state.read().await;
                state.tasks.values().any(|t| t.status == TaskStatus::Downloading || t.status == TaskStatus::Queued)
                    || state.speed_history.last().copied().unwrap_or(0.0) > 0.0
            };
            if !should_update {
                continue;
            }

            {
                let mut state = self.state.write().await;
                let now_inst = Instant::now();
                let settings = state.settings.clone();
                let has_ready_retry = state.tasks.values().any(|t| {
                    t.status == TaskStatus::Queued
                        && state
                            .retry_deadlines
                            .get(&t.id)
                            .is_none_or(|d| *d <= now_inst)
                });
                let in_night_window = is_within_night_window(
                    settings.night_scheduler_start_hour,
                    settings.night_scheduler_end_hour,
                );
                let has_queued = state.tasks.values().any(|t| t.status == TaskStatus::Queued);
                if has_queued
                    && (!settings.night_scheduler_enabled || in_night_window)
                    && (has_ready_retry || state.tasks.values().filter(|t| t.status == TaskStatus::Downloading).count() < settings.max_concurrent_downloads)
                {
                    needs_queue = true;
                }

                if settings.night_scheduler_enabled && !in_night_window {
                    let force_ids = state.force_start_ids.clone();
                    for task in state.tasks.values_mut() {
                        if task.status == TaskStatus::Downloading && !force_ids.contains(&task.id) {
                            task.status = TaskStatus::Queued;
                            task.speed_bytes_per_sec = 0.0;
                            task.eta_seconds = 0;
                            pause_outside_window.push(task.id.clone());
                            structure_changed = true;
                            active_changed = true;
                        }
                    }
                }

                let mut total_speed = 0.0f64;
                for task in state.tasks.values_mut() {
                    if task.status == TaskStatus::Downloading {
                        if let Some((dl, total, speed)) =
                            self.telemetry.effective_task_speed(&task.id)
                        {
                            task.downloaded_bytes = dl;
                            if total > 0 {
                                task.total_bytes = total;
                            }
                            task.speed_bytes_per_sec = speed;
                            task.eta_seconds = if task.total_bytes > 0 && speed > 0.0 {
                                ((task.total_bytes.saturating_sub(dl)) as f64 / speed).ceil()
                                    as u64
                            } else {
                                0
                            };
                            total_speed += speed;
                            active_changed = true;
                        }
                    }
                }
                if active_changed || state.speed_history.last().copied().unwrap_or(0.0) > 0.0 {
                    state.speed_history.push(total_speed);
                    if state.speed_history.len() > 20 {
                        state.speed_history.remove(0);
                    }
                }
            }

            for tid in pause_outside_window {
                self.cancel_run(&tid).await;
            }

            if structure_changed {
                (self.emit)(DownloadEvent::StateChanged);
            } else if active_changed {
                let (stats, speed_history) = {
                    let state = self.state.read().await;
                    let visible: Vec<DownloadTask> = state
                        .tasks
                        .values()
                        .filter(|t| !t.awaiting_browser_commit)
                        .cloned()
                        .collect();
                    (compute_stats(&visible, self.throttle.get_global_limit()), state.speed_history.clone())
                };
                (self.emit)(DownloadEvent::TelemetryTick { stats, speed_history });
            }
            if needs_queue {
                self.process_queue().await;
            }
        }
    }
}

pub(crate) fn url_host(raw: &str) -> Option<String> {
    reqwest::Url::parse(raw)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_ascii_lowercase()))
}

pub fn is_within_night_window(start_hour: u8, end_hour: u8) -> bool {
    let hour = disk::local_hour();
    if start_hour <= end_hour {
        hour >= start_hour && hour < end_hour
    } else {
        hour >= start_hour || hour < end_hour
    }
}

pub(crate) fn compute_stats(tasks: &[DownloadTask], speed_limit: u64) -> DownloadStats {
    let mut active_count = 0;
    let mut queued_count = 0;
    let mut completed_count = 0;
    let mut total_bytes_downloaded = 0;
    let mut current_total_speed = 0.0;

    for t in tasks {
        match t.status {
            TaskStatus::Downloading => {
                active_count += 1;
                current_total_speed += t.speed_bytes_per_sec;
            }
            TaskStatus::Queued => queued_count += 1,
            TaskStatus::Completed => completed_count += 1,
            _ => {}
        }
        total_bytes_downloaded += t.downloaded_bytes;
    }

    DownloadStats {
        active_count,
        queued_count,
        completed_count,
        total_bytes_downloaded,
        current_total_speed,
        speed_limit_bytes_per_sec: speed_limit,
    }
}
