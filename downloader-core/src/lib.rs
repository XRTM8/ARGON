pub mod debrid;
pub mod disk;
pub mod model;
pub mod resolver;
pub mod service;
pub mod social;
pub mod throttle;
pub mod torrent;
pub mod transfer;

pub use model::{
    AddTaskRequest, BrowserDownloadPayload, ChecksumInfo, ChunkStatus, Config, DownloadChunk,
    DownloadEvent, DownloadStats, DownloadTask, DownloaderSettings, DownloaderSettingsPatch,
    DownloaderSnapshot, MediaMetadata, QueueId, TaskCategory, TaskStatus, UrlProbeResult,
    DEFAULT_USER_AGENT,
};
pub use resolver::recommended_segment_count;
pub use service::{BoxFuture, DownloaderService, MediaDownloadUpdate, MediaExtractor, ResolvedMediaSpec};
pub use torrent::{is_magnet_uri, parse_magnet_uri, parse_torrent_bytes, TorrentFileItem, TorrentMetadata};
pub use transfer::{
    build_headers, build_http_client, dual_hash_id, parse_content_range, probe_resource,
    SegmentOutcome, SegmentTransfer, TelemetryTracker,
};
