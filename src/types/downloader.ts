import type { TorrentMetadata } from '../core/torrent';

export type TaskStatus = 'queued' | 'downloading' | 'paused' | 'completed' | 'error';

export type TaskCategory = 'programs' | 'media' | 'documents' | 'archives' | 'torrents' | 'other';

export type QueueId = 'main' | 'night' | 'media';

export interface DownloadChunk {
  index: number;
  startByte: number;
  endByte: number;
  downloaded: number;
  status: 'pending' | 'downloading' | 'completed' | 'error';
  speedBytesPerSec?: number;
}

export interface ChecksumInfo {
  algorithm: 'sha256' | 'md5';
  expected?: string;
  actual?: string;
  verified?: boolean;
}

export interface MediaFormatInfo {
  id: string;
  ext: string;
  resolution: string;
  fps?: number;
  filesize?: number;
  vcodec: string;
  acodec: string;
  hasVideo: boolean;
  hasAudio: boolean;
  label: string;
}

export interface MediaMetadataInfo {
  title: string;
  uploader?: string;
  thumbnail?: string;
  duration?: number;
  hasFfmpeg: boolean;
  formats: MediaFormatInfo[];
}

export interface MediaToolsStatus {
  hasYtdlp: boolean;
  hasNode: boolean;
  hasFfmpeg: boolean;
  toolsDir: string;
}

export interface MediaMetadata {
  title?: string;
  author?: string;
  thumbnail?: string;
  platform?: 'youtube' | 'tiktok' | 'instagram' | 'twitter' | 'facebook' | 'reddit' | 'threads' | 'vimeo' | 'soundcloud' | 'direct' | 'other';
  quality?: string;
  duration?: number;
  isAudioOnly?: boolean;
  selectedFormatId?: string;
  useYtdlpMux?: boolean;
}

export interface DownloadTask {
  id: string;
  url: string;
  filename: string;
  totalBytes: number;
  downloadedBytes: number;
  status: TaskStatus;
  speedBytesPerSec: number;
  etaSeconds: number;
  chunks: DownloadChunk[];
  createdAt: number;
  completedAt?: number;
  errorMessage?: string;
  checksum?: ChecksumInfo;
  savePath?: string;
  headers?: Record<string, string>;
  postBody?: string;
  method?: string;
  category?: TaskCategory;
  torrentInfo?: TorrentMetadata;
  mediaInfo?: MediaMetadata;
  originalUrl?: string;
  requestedSegments?: number;
  customFilename?: boolean;
  filePrepared?: boolean;
  acceptRanges?: boolean;
  etag?: string;
  lastModified?: string;
  probed?: boolean;
  queueId?: QueueId;
  speedLimitBytesPerSec?: number;
  mirrors?: string[];
  activeMirrorIndex?: number;
  retryCount?: number;
}

export interface DownloadStats {
  activeCount: number;
  queuedCount: number;
  completedCount: number;
  totalBytesDownloaded: number;
  currentTotalSpeed: number;
  speedLimitBytesPerSec: number;
}

export interface AppSettings {
  maxConcurrentDownloads: number;
  maxConnectionsPerHost?: number;
  globalSpeedLimitBytes: number;
  enableNotifications: boolean;
  enableSound: boolean;
  soundVolume?: number;
  defaultDirectory: string;
  defaultSegmentCount?: number;
  autoShutdownOnComplete?: boolean;
  nightSchedulerEnabled?: boolean;
  nightSchedulerStartHour?: number;
  nightSchedulerEndHour?: number;
  theme?: 'dark' | 'light';
  dynamicWorkStealing?: boolean;
  clipboardMonitorEnabled?: boolean;
  autoVerifyIntegrity?: boolean;
  autoRetryEnabled?: boolean;
  maxAutoRetries?: number;
  minimizeToTray?: boolean;
  startWithWindows?: boolean;
}

export interface DownloaderSnapshot {
  tasks: DownloadTask[];
  stats: DownloadStats;
  settings: AppSettings;
  speedHistory: number[];
  updating: boolean;
}
