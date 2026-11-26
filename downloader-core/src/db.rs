use crate::model::DownloadTask;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum WalRecord {
    Upsert { task: Box<DownloadTask> },
    Delete { id: String },
}

/// Crash-safe Pure-Rust WAL store: appends incremental task updates to `tasks.wal`
/// and compacts to `tasks.json` periodically or on shutdown.
pub struct StateDb {
    state_dir: PathBuf,
    wal_lock: Mutex<()>,
}

impl StateDb {
    pub fn open(state_dir: &Path) -> Result<Self, String> {
        std::fs::create_dir_all(state_dir)
            .map_err(|e| format!("Cannot create state dir: {e}"))?;
        Ok(Self {
            state_dir: state_dir.to_path_buf(),
            wal_lock: Mutex::new(()),
        })
    }

    fn snapshot_path(&self) -> PathBuf {
        self.state_dir.join("tasks.json")
    }

    fn wal_path(&self) -> PathBuf {
        self.state_dir.join("tasks.wal")
    }

    /// Loads base `tasks.json` and replays any uncommitted `tasks.wal` records.
    pub fn load_tasks(&self) -> Result<Vec<DownloadTask>, String> {
        let _g = self.wal_lock.lock().map_err(|_| "WAL lock poisoned")?;
        let mut map: HashMap<String, DownloadTask> = HashMap::new();
        let snap = self.snapshot_path();
        if snap.is_file() {
            let raw = std::fs::read_to_string(&snap).map_err(|e| e.to_string())?;
            if !raw.trim().is_empty() {
                if let Ok(list) = serde_json::from_str::<Vec<DownloadTask>>(&raw) {
                    for t in list {
                        map.insert(t.id.clone(), t);
                    }
                }
            }
        }

        let wal = self.wal_path();
        if wal.is_file() {
            if let Ok(file) = std::fs::File::open(&wal) {
                let reader = BufReader::new(file);
                for line in reader.lines().map_while(Result::ok) {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if let Ok(rec) = serde_json::from_str::<WalRecord>(trimmed) {
                        match rec {
                            WalRecord::Upsert { task } => {
                                map.insert(task.id.clone(), *task);
                            }
                            WalRecord::Delete { id } => {
                                map.remove(&id);
                            }
                        }
                    }
                }
            }
        }

        let mut out: Vec<DownloadTask> = map.into_values().collect();
        out.sort_by_key(|a| std::cmp::Reverse(a.created_at));
        Ok(out)
    }

    /// Appends a single task upsert to `tasks.wal` without rewriting all tasks.
    pub fn append_upsert(&self, task: &DownloadTask) -> Result<(), String> {
        let _g = self.wal_lock.lock().map_err(|_| "WAL lock poisoned")?;
        let line = serde_json::to_string(&WalRecord::Upsert {
            task: Box::new(task.clone()),
        })
        .map_err(|e| e.to_string())?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.wal_path())
            .map_err(|e| e.to_string())?;
        writeln!(file, "{line}").map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Compacts all tasks atomically into `tasks.json` and truncates `tasks.wal`.
    pub fn compact(&self, tasks: &[&DownloadTask]) -> Result<(), String> {
        let _g = self.wal_lock.lock().map_err(|_| "WAL lock poisoned")?;
        crate::disk::save_json_atomic_blocking(&self.snapshot_path(), &tasks)?;
        let wal = self.wal_path();
        if wal.exists() {
            let _ = std::fs::remove_file(wal);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{QueueId, TaskCategory, TaskStatus};

    fn sample_task(id: &str) -> DownloadTask {
        DownloadTask {
            id: id.into(),
            url: "https://example.com/a.zip".into(),
            original_url: "https://example.com/a.zip".into(),
            filename: "a.zip".into(),
            total_bytes: 1024,
            downloaded_bytes: 0,
            status: TaskStatus::Queued,
            speed_bytes_per_sec: 0.0,
            eta_seconds: 0,
            chunks: Vec::new(),
            created_at: 100,
            completed_at: None,
            error_message: None,
            resume_notice: None,
            media_finalizing: None,
            checksum: None,
            save_path: None,
            headers: HashMap::new(),
            post_body: None,
            method: None,
            category: TaskCategory::Archives,
            media_info: None,
            requested_segments: 4,
            custom_filename: false,
            file_prepared: false,
            accept_ranges: false,
            etag: None,
            last_modified: None,
            range_resource_key: None,
            probed: false,
            queue_id: QueueId::Main,
            speed_limit_bytes_per_sec: 0,
            mirrors: Vec::new(),
            active_mirror_index: 0,
            retry_count: 0,
            awaiting_browser_commit: false,
            queue_priority: 0,
        }
    }

    #[test]
    fn pure_rust_wal_replays_and_compacts_tasks() {
        let dir = std::env::temp_dir().join(format!("argon_wal_test_{}", uuid::Uuid::new_v4()));
        let db = StateDb::open(&dir).unwrap();
        let t1 = sample_task("t1");
        let mut t2 = sample_task("t2");
        t2.downloaded_bytes = 512;
        db.append_upsert(&t1).unwrap();
        db.append_upsert(&t2).unwrap();

        let loaded = db.load_tasks().unwrap();
        assert_eq!(loaded.len(), 2);

        db.compact(&[&t1, &t2]).unwrap();
        assert!(!db.wal_path().exists());
        assert_eq!(db.load_tasks().unwrap().len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
