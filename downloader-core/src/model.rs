use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub const DEFAULT_USER_AGENT: &str = concat!(
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/133.0.0.0 Safari/537.36 ARGON/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/XRTM8/ARGON)"
);

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", uuid::Uuid::new_v4().simple())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Queued,
    Downloading,
    Paused,
    Completed,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskCategory {
    Programs,
    Media,
    Documents,
    Archives,
    Torrents,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum QueueId {
    #[default]
    Main,
    Night,
    Media,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChunkStatus {
    Pending,
    Downloading,
    Completed,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadChunk {
    pub index: u32,
    pub start_byte: u64,
    pub end_byte: u64,
    pub downloaded: u64,
    pub status: ChunkStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed_bytes_per_sec: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChecksumInfo {
    pub algorithm: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verified: Option<bool>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaMetadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_audio_only: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_format_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_ytdlp_mux: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadTask {
    pub id: String,
    pub url: String,
    pub original_url: String,
    pub filename: String,
    pub total_bytes: u64,
    pub downloaded_bytes: u64,
    pub status: TaskStatus,
    pub speed_bytes_per_sec: f64,
    pub eta_seconds: u64,
    pub chunks: Vec<DownloadChunk>,
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum: Option<ChecksumInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub save_path: Option<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_body: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    pub category: TaskCategory,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_info: Option<MediaMetadata>,
    pub requested_segments: u32,
    pub custom_filename: bool,
    pub file_prepared: bool,
    pub accept_ranges: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub etag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_modified: Option<String>,
    pub probed: bool,
    pub queue_id: QueueId,
    #[serde(default)]
    pub speed_limit_bytes_per_sec: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mirrors: Vec<String>,
    #[serde(default)]
    pub active_mirror_index: usize,
    #[serde(default)]
    pub retry_count: u32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadStats {
    pub active_count: usize,
    pub queued_count: usize,
    pub completed_count: usize,
    pub total_bytes_downloaded: u64,
    pub current_total_speed: f64,
    pub speed_limit_bytes_per_sec: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct DownloaderSettings {
    pub max_concurrent_downloads: usize,
    pub max_connections_per_host: usize,
    pub global_speed_limit_bytes: u64,
    pub enable_notifications: bool,
    pub enable_sound: bool,
    pub sound_volume: f64,
    pub default_directory: String,
    pub default_segment_count: u32,
    pub auto_shutdown_on_complete: bool,
    pub night_scheduler_enabled: bool,
    pub night_scheduler_start_hour: u8,
    pub night_scheduler_end_hour: u8,
    pub theme: String,
    pub dynamic_work_stealing: bool,
    pub clipboard_monitor_enabled: bool,
    pub auto_verify_integrity: bool,
    pub auto_retry_enabled: bool,
    pub max_auto_retries: u32,
    pub minimize_to_tray: bool,
    pub start_with_windows: bool,
}

impl Default for DownloaderSettings {
    fn default() -> Self {
        Self {
            max_concurrent_downloads: 3,
            max_connections_per_host: 4,
            global_speed_limit_bytes: 0,
            enable_notifications: true,
            enable_sound: true,
            sound_volume: 0.5,
            default_directory: String::new(),
            default_segment_count: 8,
            auto_shutdown_on_complete: false,
            night_scheduler_enabled: false,
            night_scheduler_start_hour: 2,
            night_scheduler_end_hour: 8,
            theme: "dark".to_owned(),
            dynamic_work_stealing: true,
            clipboard_monitor_enabled: false,
            auto_verify_integrity: false,
            auto_retry_enabled: true,
            max_auto_retries: 3,
            minimize_to_tray: true,
            start_with_windows: false,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloaderSettingsPatch {
    pub max_concurrent_downloads: Option<usize>,
    pub max_connections_per_host: Option<usize>,
    pub global_speed_limit_bytes: Option<u64>,
    pub enable_notifications: Option<bool>,
    pub enable_sound: Option<bool>,
    pub sound_volume: Option<f64>,
    pub default_directory: Option<String>,
    pub default_segment_count: Option<u32>,
    pub auto_shutdown_on_complete: Option<bool>,
    pub night_scheduler_enabled: Option<bool>,
    pub night_scheduler_start_hour: Option<u8>,
    pub night_scheduler_end_hour: Option<u8>,
    pub theme: Option<String>,
    pub dynamic_work_stealing: Option<bool>,
    pub clipboard_monitor_enabled: Option<bool>,
    pub auto_verify_integrity: Option<bool>,
    pub auto_retry_enabled: Option<bool>,
    pub max_auto_retries: Option<u32>,
    pub minimize_to_tray: Option<bool>,
    pub start_with_windows: Option<bool>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddTaskRequest {
    pub url: String,
    pub filename: Option<String>,
    pub segment_count: Option<u32>,
    pub expected_sha256: Option<String>,
    pub headers: Option<HashMap<String, String>>,
    pub post_body: Option<String>,
    pub method: Option<String>,
    pub media_info: Option<MediaMetadata>,
    pub queue_id: Option<QueueId>,
    pub mirrors: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserDownloadPayload {
    pub url: String,
    pub filename: Option<String>,
    pub headers: Option<HashMap<String, String>>,
    pub method: Option<String>,
    pub post_body: Option<String>,
    #[serde(default)]
    pub request_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UrlProbeResult {
    pub content_length: Option<u64>,
    pub accept_ranges: bool,
    pub content_type: Option<String>,
    pub suggested_filename: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub final_url: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloaderSnapshot {
    pub tasks: Vec<DownloadTask>,
    pub stats: DownloadStats,
    pub settings: DownloaderSettings,
    pub speed_history: Vec<f64>,
    pub updating: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", content = "data", rename_all = "camelCase")]
pub enum DownloadEvent {
    StateChanged,
    TaskCompleted {
        task_id: String,
        filename: String,
        save_path: String,
    },
    TaskFailed {
        task_id: String,
        filename: String,
        error: String,
    },
    AllIdleShutdownPending,
    MediaDownloadRequested {
        task_id: String,
        url: String,
        destination_path: String,
        format_spec: String,
    },
}

#[derive(Clone, Debug)]
pub struct Config {
    pub state_dir: PathBuf,
    pub default_download_dir: PathBuf,
    pub is_android: bool,
}
