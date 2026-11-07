import type { DownloadTask, DownloadStats, AppSettings } from '../types/downloader';
import {
  calculateSegments,
  extractFilename,
  sanitizeFilename,
  detectCategory,
  deduplicateFilename
} from './segmentation';
import { resolveUniversalLink } from './linkResolver';
import { detectPlatform, resolveMediaDownloadUrl } from './mediaResolver';
import { soundEngine } from './sound';
import { isMagnetUri, parseMagnetUri, type TorrentMetadata } from './torrent';
import {
  loadTasksFromStorage,
  saveTasksToStorage,
  loadSettingsFromStorage,
  saveSettingsToStorage
} from './storage';

// Check for Tauri native runtime
function isTauri(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

export interface ProbeResult {
  content_length?: number;
  accept_ranges: boolean;
  content_type?: string;
  suggested_filename?: string;
  etag?: string;
  last_modified?: string;
}

interface SpeedSample {
  lastBytes: number;
  lastTime: number;
}

interface NativeRun {
  nativeId: string;
  cancelled: boolean;
  promise: Promise<void>;
}

export class DownloadManager {
  private ready: Promise<void>;
  private runtimeError?: string;
  private runs = new Map<string, NativeRun>();
  private destroyed = false;
  private batchPausing = false;
  private tasks: Map<string, DownloadTask> = new Map();
  private listeners: Set<() => void> = new Set();
  private speedSamples: Map<string, SpeedSample> = new Map();
  private speedInterval: number | null = null;
  private tauriUnlisten: (() => void) | null = null;
  private defaultDownloadDir: string | null = null;
  private notifyTimeout: number | null = null;
  private storageSaveTimeout: number | null = null;
  private speedHistory: number[] = new Array(20).fill(0);

  // App Settings
  private maxConcurrentDownloads = 3;
  private speedLimitBytes = 0;
  private notificationsEnabled = true;
  private soundEnabled = true;
  private defaultSegmentCount = 8;
  private autoShutdownOnComplete = false;
  private nightSchedulerEnabled = false;
  private nightSchedulerStartHour = 2; // 02:00
  private nightSchedulerEndHour = 8;   // 08:00

  constructor() {
    this.loadSettings();
    this.loadFromStorage();
    this.startSpeedLoop();
    this.ready = Promise.all([this.initTauriListeners(), this.initDefaultDirectory()]).then(() => {}).catch((error) => {
      this.runtimeError = String(error);
    });
  }

  private loadSettings() {
    const parsed = loadSettingsFromStorage();
    if (!parsed) return;

    if (typeof parsed.maxConcurrentDownloads === 'number') {
      this.maxConcurrentDownloads = Math.max(1, parsed.maxConcurrentDownloads);
    }
    if (typeof parsed.globalSpeedLimitBytes === 'number') {
      this.speedLimitBytes = Math.max(0, parsed.globalSpeedLimitBytes);
    }
    if (typeof parsed.enableNotifications === 'boolean') {
      this.notificationsEnabled = parsed.enableNotifications;
    }
    if (typeof parsed.enableSound === 'boolean') {
      this.soundEnabled = parsed.enableSound;
      soundEngine.enabled = this.soundEnabled;
    }
    if (typeof parsed.soundVolume === 'number') {
      soundEngine.volume = Math.max(0, Math.min(1, parsed.soundVolume));
    }
    if (typeof parsed.defaultSegmentCount === 'number') {
      this.defaultSegmentCount = Math.max(1, Math.min(32, parsed.defaultSegmentCount));
    }
    if (typeof parsed.autoShutdownOnComplete === 'boolean') {
      this.autoShutdownOnComplete = parsed.autoShutdownOnComplete;
    }
    if (typeof parsed.nightSchedulerEnabled === 'boolean') {
      this.nightSchedulerEnabled = parsed.nightSchedulerEnabled;
    }
    if (typeof parsed.nightSchedulerStartHour === 'number') {
      this.nightSchedulerStartHour = Math.max(0, Math.min(23, parsed.nightSchedulerStartHour));
    }
    if (typeof parsed.nightSchedulerEndHour === 'number') {
      this.nightSchedulerEndHour = Math.max(0, Math.min(23, parsed.nightSchedulerEndHour));
    }
  }

  private saveSettingsToStorage() {
    const payload: AppSettings = {
      maxConcurrentDownloads: this.maxConcurrentDownloads,
      globalSpeedLimitBytes: this.speedLimitBytes,
      enableNotifications: this.notificationsEnabled,
      enableSound: this.soundEnabled,
      soundVolume: soundEngine.volume,
      defaultDirectory: this.defaultDownloadDir || 'Downloads',
      defaultSegmentCount: this.defaultSegmentCount,
      autoShutdownOnComplete: this.autoShutdownOnComplete,
      nightSchedulerEnabled: this.nightSchedulerEnabled,
      nightSchedulerStartHour: this.nightSchedulerStartHour,
      nightSchedulerEndHour: this.nightSchedulerEndHour
    };
    saveSettingsToStorage(payload);
  }

  private loadFromStorage() {
    const restored = loadTasksFromStorage();
    for (const task of restored) {
      this.tasks.set(task.id, task);
    }
  }

  private scheduleStorageSave() {
    if (this.storageSaveTimeout !== null) return;

    this.storageSaveTimeout = (typeof window !== 'undefined' ? globalThis.setTimeout : setTimeout)(() => {
      this.storageSaveTimeout = null;
      saveTasksToStorage(Array.from(this.tasks.values()));
    }, 400) as unknown as number;
  }

  private showNotification(title: string, body: string) {
    if (!this.notificationsEnabled || typeof window === 'undefined') return;
    try {
      if ('Notification' in window) {
        if (Notification.permission === 'granted') {
          new Notification(title, { body });
        }
      }
    } catch {
      // Notification unsupported
    }
  }

  private async initDefaultDirectory() {
    if (!isTauri()) return;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      this.defaultDownloadDir = await invoke<string>('get_default_download_directory');
      const savedDirectory = loadSettingsFromStorage()?.defaultDirectory;
      if (savedDirectory && savedDirectory !== 'Downloads') {
        try { this.defaultDownloadDir = await invoke<string>('validate_download_directory', { directory: savedDirectory }); }
        catch { /* An unavailable saved directory falls back to the native Downloads folder. */ }
      }
      if (this.speedLimitBytes > 0) {
        await invoke('set_global_speed_limit', { bytesPerSec: this.speedLimitBytes });
      }
      this.notify(true);
    } catch (error) {
      throw new Error(`تعذر تهيئة مجلد التنزيل: ${String(error)}`);
    }
  }

  private async initTauriListeners() {
    if (!isTauri()) return;
    try {
      const { listen } = await import('@tauri-apps/api/event');
      const unlistenProgress = await listen<{
        task_id: string;
        segment_index: number;
        delta_bytes: number;
        downloaded_bytes: number;
        total_bytes: number;
        is_finished: boolean;
      }>('download-progress', (event) => {
        const { task_id, segment_index, delta_bytes, downloaded_bytes, total_bytes, is_finished } = event.payload;
        this.handleTauriChunkProgress(task_id, segment_index, delta_bytes, downloaded_bytes, total_bytes, is_finished);
      });

      const unlistenBrowser = await listen<{
        url: string;
        filename?: string;
        headers?: Record<string, string>;
      }>('browser-add-download', (event) => {
        if (event.payload?.url) {
          this.addTask(
            event.payload.url,
            event.payload.filename,
            this.defaultSegmentCount,
            undefined,
            event.payload.headers
          );
          this.showNotification('تم استلام تنزيل من المتصفح ⚡', event.payload.filename || event.payload.url);
        }
      });
      this.tauriUnlisten = () => {
        unlistenProgress();
        unlistenBrowser();
      };
      if (this.destroyed) { this.tauriUnlisten(); this.tauriUnlisten = null; return; }
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('set_browser_bridge_ready');
    } catch (error) {
      throw new Error(`تعذر تهيئة أحداث التنزيل: ${String(error)}`);
    }
  }

  public subscribe(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private notify(immediate = false) {
    if (immediate) {
      if (this.notifyTimeout !== null) {
        globalThis.clearTimeout(this.notifyTimeout);
        this.notifyTimeout = null;
      }
      this.listeners.forEach((l) => l());
      return;
    }

    if (this.notifyTimeout !== null) return;
    this.notifyTimeout = globalThis.setTimeout(() => {
      this.notifyTimeout = null;
      this.listeners.forEach((l) => l());
    }, 60);
  }

  public async triggerShutdown(cancel: boolean = false): Promise<boolean> {
    if (!isTauri()) return false;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('system_shutdown', { cancel });
      return true;
    } catch {
      return false;
    }
  }

  public getSpeedHistory(): number[] {
    return [...this.speedHistory];
  }

  public restartTask(id: string) {
    const task = this.tasks.get(id);
    if (!task) return;
    this.cancelRun(id);
    task.downloadedBytes = 0;
    task.completedAt = undefined;
    task.errorMessage = undefined;
    task.probed = false;
    if (task.checksum) { task.checksum.actual = undefined; task.checksum.verified = false; }
    task.chunks = calculateSegments(0, 1);
    task.status = 'queued';
    this.scheduleStorageSave();
    this.processQueue();
    this.notify(true);
  }

  public getTasks(): DownloadTask[] {
    return Array.from(this.tasks.values()).sort((a, b) => b.createdAt - a.createdAt);
  }

  public getSettings(): AppSettings {
    return {
      maxConcurrentDownloads: this.maxConcurrentDownloads,
      globalSpeedLimitBytes: this.speedLimitBytes,
      enableNotifications: this.notificationsEnabled,
      enableSound: this.soundEnabled,
      soundVolume: soundEngine.volume,
      defaultDirectory: this.defaultDownloadDir || 'Downloads',
      defaultSegmentCount: this.defaultSegmentCount,
      autoShutdownOnComplete: this.autoShutdownOnComplete,
      nightSchedulerEnabled: this.nightSchedulerEnabled,
      nightSchedulerStartHour: this.nightSchedulerStartHour,
      nightSchedulerEndHour: this.nightSchedulerEndHour
    };
  }

  public async updateSettings(settings: Partial<AppSettings>) {
    if (settings.defaultDirectory !== undefined) {
      if (!isTauri()) throw new Error('تغيير مجلد الحفظ يتطلب تطبيق ARGON لسطح المكتب.');
      const { invoke } = await import('@tauri-apps/api/core');
      this.defaultDownloadDir = await invoke<string>('validate_download_directory', { directory: settings.defaultDirectory.trim() });
    }
    if (settings.maxConcurrentDownloads !== undefined) {
      this.maxConcurrentDownloads = Math.max(1, Math.min(32, Math.floor(settings.maxConcurrentDownloads) || 1));
    }
    if (settings.globalSpeedLimitBytes !== undefined) {
      this.speedLimitBytes = Math.max(0, settings.globalSpeedLimitBytes);
      if (isTauri()) {
        try {
          const { invoke } = await import('@tauri-apps/api/core');
          await invoke('set_global_speed_limit', { bytesPerSec: this.speedLimitBytes });
        } catch {
          // Fallback
        }
      }
    }
    if (settings.enableNotifications !== undefined) {
      this.notificationsEnabled = settings.enableNotifications;
      if (this.notificationsEnabled && typeof window !== 'undefined' && 'Notification' in window && Notification.permission === 'default') {
        Notification.requestPermission().catch(() => {});
      }
    }
    if (settings.enableSound !== undefined) {
      this.soundEnabled = settings.enableSound;
      soundEngine.enabled = this.soundEnabled;
    }
    if (settings.soundVolume !== undefined) {
      soundEngine.volume = Math.max(0, Math.min(1, settings.soundVolume));
    }
    if (settings.defaultSegmentCount !== undefined) {
      this.defaultSegmentCount = Math.max(1, Math.min(32, settings.defaultSegmentCount));
    }
    if (settings.autoShutdownOnComplete !== undefined) {
      this.autoShutdownOnComplete = Boolean(settings.autoShutdownOnComplete);
    }
    if (settings.nightSchedulerEnabled !== undefined) {
      this.nightSchedulerEnabled = Boolean(settings.nightSchedulerEnabled);
    }
    if (settings.nightSchedulerStartHour !== undefined) {
      this.nightSchedulerStartHour = Math.max(0, Math.min(23, settings.nightSchedulerStartHour));
    }
    if (settings.nightSchedulerEndHour !== undefined) {
      this.nightSchedulerEndHour = Math.max(0, Math.min(23, settings.nightSchedulerEndHour));
    }
    this.saveSettingsToStorage();
    this.notify(true);
    this.processQueue();
  }

  public isNightSchedulerActive(): boolean {
    if (!this.nightSchedulerEnabled) return true;
    const hour = new Date().getHours();
    if (this.nightSchedulerStartHour <= this.nightSchedulerEndHour) {
      return hour >= this.nightSchedulerStartHour && hour < this.nightSchedulerEndHour;
    } else {
      return hour >= this.nightSchedulerStartHour || hour < this.nightSchedulerEndHour;
    }
  }

  private checkAutoShutdown() {
    if (!this.autoShutdownOnComplete) return;
    const remainingActiveOrQueued = Array.from(this.tasks.values()).filter(
      (t) => t.status !== 'completed'
    );
    if (remainingActiveOrQueued.length === 0) {
      this.showNotification('إيقاف التشغيل المجدول', 'اكتملت جميع التحميلات، جاري إيقاف تشغيل الكمبيوتر...');
      this.triggerShutdown(false);
    }
  }

  public async openFile(filePath?: string) {
    if (!filePath || !isTauri()) return;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('open_file_item', { filePath });
    } catch (e) {
      console.warn('Failed to open file:', e);
    }
  }

  public async openBrowserExtensionDir() {
    if (!isTauri()) return;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('open_browser_extension_dir');
    } catch (e) {
      console.warn('Failed to open browser extension dir:', e);
    }
  }

  public addBatchTasks(urls: string[], segmentCount?: number): string[] {
    const ids: string[] = [];
    for (const raw of urls) {
      const trimmed = raw.trim();
      if (trimmed && /^https?:\/\/.+/i.test(trimmed)) {
        const id = this.addTask(trimmed, undefined, segmentCount);
        ids.push(id);
      }
    }
    return ids;
  }

  public exportUrls(filter?: 'all' | 'active' | 'completed'): string {
    const tasks = Array.from(this.tasks.values());
    const filtered = tasks.filter((t) => {
      if (filter === 'active') return t.status === 'downloading' || t.status === 'queued' || t.status === 'paused';
      if (filter === 'completed') return t.status === 'completed';
      return true;
    });
    return filtered.map((t) => t.url).join('\n');
  }

  public exportTasksJson(): string {
    const list = Array.from(this.tasks.values()).map((t) => ({
      url: t.url,
      filename: t.filename,
      category: t.category,
      headers: t.headers,
      checksum: t.checksum?.expected,
      createdAt: t.createdAt
    }));
    return JSON.stringify(list, null, 2);
  }

  public importTasks(rawText: string, segmentCount?: number): number {
    const trimmed = rawText.trim();
    if (!trimmed) return 0;

    let addedCount = 0;

    // 1. Try parsing as JSON array
    if (trimmed.startsWith('[') && trimmed.endsWith(']')) {
      try {
        const parsed = JSON.parse(trimmed);
        if (Array.isArray(parsed)) {
          for (const item of parsed) {
            if (item && typeof item.url === 'string' && /^https?:\/\/.+/i.test(item.url.trim())) {
              this.addTask(
                item.url.trim(),
                typeof item.filename === 'string' ? item.filename : undefined,
                segmentCount,
                typeof item.checksum === 'string' ? item.checksum : undefined,
                item.headers && typeof item.headers === 'object' ? item.headers : undefined
              );
              addedCount++;
            }
          }
          if (addedCount > 0) return addedCount;
        }
      } catch {
        // Fall back to plain text
      }
    }

    // 2. Parse as newline-separated plain text / URLs
    const lines = trimmed.split(/[\r\n]+/);
    for (const line of lines) {
      const match = line.match(/https?:\/\/[^\s"']+/i);
      if (match) {
        this.addTask(match[0].trim(), undefined, segmentCount);
        addedCount++;
      }
    }

    return addedCount;
  }

  public getStats(): DownloadStats {
    let activeCount = 0;
    let queuedCount = 0;
    let completedCount = 0;
    let totalBytesDownloaded = 0;
    let currentTotalSpeed = 0;

    for (const task of this.tasks.values()) {
      if (task.status === 'downloading') {
        activeCount++;
        currentTotalSpeed += task.speedBytesPerSec;
      } else if (task.status === 'queued') {
        queuedCount++;
      } else if (task.status === 'completed') {
        completedCount++;
      }
      totalBytesDownloaded += task.downloadedBytes;
    }

    return {
      activeCount,
      queuedCount,
      completedCount,
      totalBytesDownloaded,
      currentTotalSpeed,
      speedLimitBytesPerSec: this.speedLimitBytes
    };
  }

  public async probe(url: string, customHeaders?: Record<string, string>): Promise<ProbeResult> {
    if (!isTauri()) throw new Error('التنزيل يتطلب تطبيق ARGON لسطح المكتب؛ هذه نافذة معاينة فقط.');
    const { invoke } = await import('@tauri-apps/api/core');
    return invoke<ProbeResult>('probe_url', { url, customHeaders: customHeaders || null });
  }

  public processQueue() {
    if (this.destroyed || this.batchPausing || !this.isNightSchedulerActive()) return;
    let active = this.getTasks().filter((t) => t.status === 'downloading').length;
    for (const task of this.getTasks().filter((t) => t.status === 'queued').sort((a, b) => a.createdAt - b.createdAt)) {
      if (active >= this.maxConcurrentDownloads) break;
      task.status = 'downloading';
      task.errorMessage = undefined;
      active++;
      this.startNativeDownload(task);
    }
    this.scheduleStorageSave();
    this.notify(true);
  }

  public addTask(
    url: string, customFilename?: string, segmentCount: number = this.defaultSegmentCount,
    expectedSha256?: string, customHeaders?: Record<string, string>, torrentMeta?: TorrentMetadata,
    mediaInfo?: import('../types/downloader').MediaMetadata
  ): string {
    const originalUrl = url.trim();
    const resolved = resolveUniversalLink(originalUrl);
    const isMagnet = isMagnetUri(originalUrl);
    const meta = torrentMeta || (isMagnet ? parseMagnetUri(originalUrl) || undefined : undefined);
    const id = `task_${Date.now()}_${Math.random().toString(36).slice(2, 9)}`;
    const filename = deduplicateFilename(sanitizeFilename(customFilename?.trim() || meta?.name || mediaInfo?.title || extractFilename(resolved.resolvedUrl)), this.getTasks().map((t) => t.filename));
    let error: string | undefined;
    if (meta || isMagnet) error = 'روابط Magnet والتورنت تحتاج محرك P2P غير متاح في هذا الإصدار؛ استخدم رابط HTTP/HTTPS مباشر.';
    else {
      try { if (!['http:', 'https:'].includes(new URL(resolved.resolvedUrl).protocol)) throw new Error(); }
      catch { error = 'رابط غير صالح؛ أدخل رابط HTTP أو HTTPS كاملًا.'; }
    }
    if (!error && !isTauri()) error = 'التنزيل يتطلب تطبيق ARGON لسطح المكتب؛ هذه نافذة معاينة فقط.';
    if (expectedSha256?.trim() && !/^[a-f0-9]{64}$/i.test(expectedSha256.trim())) error = 'بصمة SHA-256 غير صالحة.';
    const task: DownloadTask = {
      id, originalUrl, url: resolved.resolvedUrl, filename,
      category: meta ? 'torrents' : detectCategory(filename), torrentInfo: meta, mediaInfo,
      headers: { ...resolved.suggestedHeaders, ...customHeaders },
      totalBytes: meta?.totalLength || 0, downloadedBytes: 0, status: error ? 'error' : 'queued',
      errorMessage: error, speedBytesPerSec: 0, etaSeconds: 0, chunks: calculateSegments(0, 1),
      createdAt: Date.now(), requestedSegments: Math.max(1, Math.min(32, Math.floor(segmentCount) || 1)),
      customFilename: !!customFilename?.trim(), filePrepared: false,
      checksum: expectedSha256?.trim() ? { algorithm: 'sha256', expected: expectedSha256.trim().toLowerCase(), verified: false } : undefined,
    };
    this.tasks.set(id, task);
    if (!error) { soundEngine.playStartSweep(); this.processQueue(); }
    this.scheduleStorageSave();
    this.notify(true);
    return id;
  }

  public addTorrentTask(meta: TorrentMetadata, segmentCount?: number): string {
    const rawUrl = meta.isMagnet
      ? `magnet:?xt=urn:btih:${meta.infoHash}&dn=${encodeURIComponent(meta.name)}`
      : `torrent://${meta.infoHash}`;
    return this.addTask(
      rawUrl,
      meta.name,
      segmentCount || this.defaultSegmentCount,
      undefined,
      undefined,
      meta
    );
  }

  private cancelRun(id: string) {
    const run = this.runs.get(id);
    if (!run) return;
    run.cancelled = true;
    if (isTauri()) void import('@tauri-apps/api/core').then(({ invoke }) => invoke('cancel_task', { taskId: run.nativeId })).catch(console.warn);
  }

  private startNativeDownload(task: DownloadTask) {
    const previous = this.runs.get(task.id);
    if (previous) this.cancelRun(task.id);
    const run: NativeRun = { nativeId: `${task.id}_${crypto.randomUUID()}`, cancelled: false, promise: Promise.resolve() };
    this.runs.set(task.id, run);
    const current = () => !this.destroyed && !run.cancelled && this.runs.get(task.id) === run && this.tasks.get(task.id) === task && task.status === 'downloading';
    run.promise = (async () => {
      if (previous) await previous.promise;
      await this.ready;
      if (!current()) return;
      if (!isTauri()) throw new Error('التنزيل يتطلب تطبيق ARGON لسطح المكتب؛ هذه نافذة معاينة فقط.');
      if (this.runtimeError) throw new Error(this.runtimeError);
      if (task.torrentInfo) throw new Error('محرك التورنت غير متاح في هذا الإصدار.');
      const { invoke } = await import('@tauri-apps/api/core');
      const platform = detectPlatform(task.originalUrl || task.url);
      if (!['direct', 'other'].includes(platform)) {
        const media = await resolveMediaDownloadUrl(task.originalUrl || task.url, { isAudioOnly: !!task.mediaInfo?.isAudioOnly });
        if (!current()) return;
        task.url = media.downloadUrl;
        task.headers = { ...task.headers, ...media.headers };
        if (task.mediaInfo) task.mediaInfo = { ...task.mediaInfo, quality: undefined, title: media.suggestedFilename || task.mediaInfo.title };
        if (!task.customFilename && !task.filePrepared && media.suggestedFilename) task.filename = this.uniqueFilename(task, media.suggestedFilename);
      }
      const probe = await this.probe(task.url, task.headers);
      if (!current()) return;
      if (/^(text\/html|application\/(xhtml\+xml|vnd\.apple\.mpegurl|dash\+xml)|audio\/x-mpegurl)/i.test(probe.content_type || '') || /\.(m3u8|mpd)(?:$|\?)/i.test(task.url)) {
        throw new Error('الرابط يعيد صفحة ويب أو قائمة بث، وليس ملفًا مباشرًا قابلًا للتنزيل.');
      }
      const sameResource = task.probed && task.acceptRanges && probe.accept_ranges &&
        task.totalBytes === (probe.content_length || 0) &&
        !!(task.etag || task.lastModified) &&
        (task.etag ? task.etag === probe.etag : task.lastModified === probe.last_modified);
      const resume = task.downloadedBytes > 0 && sameResource;
      if (!resume) {
        task.downloadedBytes = 0;
        task.chunks = calculateSegments(probe.content_length || 0, probe.accept_ranges ? task.requestedSegments || this.defaultSegmentCount : 1);
      }
      task.totalBytes = probe.content_length || 0;
      task.acceptRanges = probe.accept_ranges;
      task.etag = probe.etag;
      task.lastModified = probe.last_modified;
      task.probed = true;
      if (!task.customFilename && !task.filePrepared && probe.suggested_filename) task.filename = this.uniqueFilename(task, probe.suggested_filename);
      task.category = detectCategory(task.filename);
      const base = this.defaultDownloadDir || 'Downloads';
      const path = task.savePath || `${base}${base.includes('\\') ? '\\' : '/'}${task.filename}`;
      task.savePath = await invoke<string>('prepare_download_file', {
        destinationPath: path, totalBytes: task.totalBytes, fresh: !task.filePrepared, reset: !resume,
      });
      task.filePrepared = true;
      task.filename = task.savePath.split(/[\\/]/).pop() || task.filename;
      if (!current()) return;
      this.speedSamples.set(task.id, { lastBytes: task.downloadedBytes, lastTime: Date.now() });
      this.scheduleStorageSave();
      this.notify(true);
      const transfer = async (useRange: boolean) => {
        const results = await Promise.allSettled(task.chunks.filter((c) => c.status !== 'completed').map(async (chunk) => {
          try {
            const result = await invoke<{ downloaded_bytes: number; total_bytes: number }>('download_segment', {
              taskId: run.nativeId, url: task.url, destinationPath: task.savePath,
              startByte: chunk.startByte, endByte: chunk.endByte, currentDownloaded: chunk.downloaded,
              segmentIndex: chunk.index, customHeaders: task.headers || null, useRange,
              expectedTotal: probe.content_length ?? null, validator: task.etag || task.lastModified || null,
            });
            if (current()) { chunk.downloaded = result.downloaded_bytes; chunk.status = 'completed'; if (!useRange) task.totalBytes = result.total_bytes; }
          } catch (error) {
            await invoke('cancel_task', { taskId: run.nativeId });
            throw error;
          }
        }));
        const failures = results.filter((r): r is PromiseRejectedResult => r.status === 'rejected');
        const cause = failures.find((r) => String(r.reason).includes('RANGE_UNSUPPORTED')) || failures.find((r) => !String(r.reason).includes('CANCELLED')) || failures[0];
        if (cause) throw new Error(String(cause.reason));
      };
      try { await transfer(probe.accept_ranges && task.totalBytes > 0); }
      catch (error) {
        if (!current() || !String(error).includes('RANGE_UNSUPPORTED')) throw error;
        await invoke('cleanup_task', { taskId: run.nativeId });
        run.nativeId = `${task.id}_${crypto.randomUUID()}`;
        task.chunks = calculateSegments(0, 1);
        task.downloadedBytes = 0;
        task.acceptRanges = false;
        this.notify(true);
        await transfer(false);
      }
      if (!current()) return;
      task.downloadedBytes = task.chunks.reduce((sum, chunk) => sum + chunk.downloaded, 0);
      if (task.downloadedBytes !== task.totalBytes) throw new Error('الملف غير مكتمل: عدد البايتات المستلمة لا يطابق الحجم المتوقع.');
      await invoke('validate_download_file', { filePath: task.savePath, expectedBytes: task.totalBytes });
      if (task.checksum?.expected) {
        const hash = await invoke<string>('verify_file_sha256', { filePath: task.savePath });
        if (!current()) return;
        task.checksum.actual = hash.toLowerCase();
        task.checksum.verified = task.checksum.actual === task.checksum.expected;
        if (!task.checksum.verified) throw new Error('فشل التحقق من SHA-256؛ الملف لا يطابق البصمة المطلوبة.');
      }
      if (!current()) return;
      task.status = 'completed';
      task.completedAt = Date.now();
      soundEngine.playCompleteChime();
      this.showNotification('اكتمل التنزيل', `تم حفظ الملف: ${task.filename}`);
      this.checkAutoShutdown();
    })().catch((error) => {
      if (!current()) return;
      task.status = 'error';
      task.errorMessage = error instanceof Error ? error.message : String(error);
      soundEngine.playErrorTone();
    }).finally(async () => {
      if (isTauri()) {
        try { const { invoke } = await import('@tauri-apps/api/core'); await invoke('cleanup_task', { taskId: run.nativeId }); } catch { /* Window may be closing. */ }
      }
      if (this.runs.get(task.id) !== run) return;
      this.runs.delete(task.id);
      task.speedBytesPerSec = 0;
      task.etaSeconds = 0;
      this.speedSamples.delete(task.id);
      this.scheduleStorageSave();
      this.notify(true);
      this.processQueue();
    });
  }

  private uniqueFilename(task: DownloadTask, filename: string) {
    return deduplicateFilename(sanitizeFilename(filename), this.getTasks().filter((t) => t.id !== task.id).map((t) => t.filename));
  }

  private handleTauriChunkProgress(taskId: string, segmentIndex: number, _deltaBytes: number, downloadedBytes: number, totalBytes: number, isFinished: boolean) {
    const entry = [...this.runs.entries()].find(([, run]) => run.nativeId === taskId && !run.cancelled);
    if (!entry) return;
    const task = this.tasks.get(entry[0]);
    if (!task || task.status !== 'downloading') return;
    const chunk = task.chunks.find((c) => c.index === segmentIndex);
    if (!chunk) return;
    chunk.downloaded = downloadedBytes;
    chunk.status = isFinished ? 'completed' : 'downloading';
    if (totalBytes > 0) task.totalBytes = totalBytes;
    task.downloadedBytes = task.chunks.reduce((sum, c) => sum + c.downloaded, 0);
    this.scheduleStorageSave();
    this.notify(false);
  }

  public pauseTask(id: string) {
    const task = this.tasks.get(id);
    if (!task || !['downloading', 'queued'].includes(task.status)) return;
    task.status = 'paused';
    task.speedBytesPerSec = 0;
    task.etaSeconds = 0;
    this.cancelRun(id);
    this.scheduleStorageSave();
    this.notify(true);
    this.processQueue();
  }

  public startTaskImmediately(id: string) { this.resumeTask(id); }

  public resumeTask(id: string) {
    const task = this.tasks.get(id);
    if (!task || !['paused', 'error', 'queued'].includes(task.status)) return;
    task.status = 'queued';
    task.errorMessage = undefined;
    this.processQueue();
  }

  public pauseAll() {
    this.batchPausing = true;
    for (const task of this.tasks.values()) {
      if (task.status === 'downloading' || task.status === 'queued') {
        this.pauseTask(task.id);
      }
    }
    this.batchPausing = false;
    this.notify(true);
  }

  public resumeAll() {
    for (const task of this.tasks.values()) {
      if (task.status === 'paused' || task.status === 'error') {
        this.resumeTask(task.id);
      }
    }
    this.notify(true);
  }

  public removeTask(id: string, deleteFileFromDisk: boolean = false) {
    const task = this.tasks.get(id);
    this.cancelRun(id);
    const running = this.runs.get(id)?.promise || Promise.resolve();
    this.tasks.delete(id);
    this.speedSamples.delete(id);
    if (deleteFileFromDisk && task?.savePath && isTauri()) void running.then(async () => {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('delete_file_item', { filePath: task.savePath });
    }).catch(console.warn);
    this.scheduleStorageSave();
    this.notify(true);
    this.processQueue();
  }

  public pauseTasks(ids: string[]) {
    for (const id of ids) {
      this.pauseTask(id);
    }
    this.notify(true);
  }

  public resumeTasks(ids: string[]) {
    for (const id of ids) {
      this.resumeTask(id);
    }
    this.notify(true);
  }

  public removeTasks(ids: string[], deleteFiles: boolean = false) {
    for (const id of ids) {
      this.removeTask(id, deleteFiles);
    }
    this.notify(true);
  }

  public clearCompleted() {
    for (const [id, task] of this.tasks.entries()) {
      if (task.status === 'completed') {
        this.tasks.delete(id);
        this.speedSamples.delete(id);
      }
    }
    this.scheduleStorageSave();
    this.notify(true);
    this.processQueue();
  }

  public async openFolder(filePath?: string) {
    if (!filePath || !isTauri()) return;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('open_destination_folder', { filePath });
    } catch (e) {
      console.warn('Failed to open folder:', e);
    }
  }

  public async verifyHash(id: string): Promise<string | null> {
    const task = this.tasks.get(id);
    if (!task || !task.savePath) return null;
    if (isTauri()) {
      try {
        const { invoke } = await import('@tauri-apps/api/core');
        const hash = await invoke<string>('verify_file_sha256', { filePath: task.savePath });
        if (!task.checksum) {
          task.checksum = { algorithm: 'sha256' };
        }
        task.checksum.actual = hash.toLowerCase();
        if (task.checksum.expected) {
          task.checksum.verified = task.checksum.actual === task.checksum.expected;
        }
        this.scheduleStorageSave();
        this.notify(true);
        return hash;
      } catch {
        return null;
      }
    }
    return null;
  }

  private startSpeedLoop() {
    if (typeof window === 'undefined') return;

    this.speedInterval = window.setInterval(() => {
      this.processScheduledQueue();
      const now = Date.now();
      let hasChanges = false;

      for (const task of this.tasks.values()) {
        if (task.status === 'downloading') {
          // In native mode: compute accurate rolling throughput
          const sample = this.speedSamples.get(task.id);
          if (sample) {
            const elapsedSec = Math.max(0.1, (now - sample.lastTime) / 1000);
            const deltaBytes = Math.max(0, task.downloadedBytes - sample.lastBytes);
            task.speedBytesPerSec = Math.round(deltaBytes / elapsedSec);

            if (task.totalBytes > 0 && task.speedBytesPerSec > 0) {
              const remaining = Math.max(0, task.totalBytes - task.downloadedBytes);
              task.etaSeconds = Math.ceil(remaining / task.speedBytesPerSec);
            } else {
              task.etaSeconds = 0;
            }

            sample.lastBytes = task.downloadedBytes;
            sample.lastTime = now;
            hasChanges = true;
          } else {
            this.speedSamples.set(task.id, { lastBytes: task.downloadedBytes, lastTime: now });
          }
        }
      }

      let currentTotal = 0;
      for (const t of this.tasks.values()) {
        if (t.status === 'downloading') {
          currentTotal += t.speedBytesPerSec;
        }
      }
      this.speedHistory.push(currentTotal);
      if (this.speedHistory.length > 20) {
        this.speedHistory.shift();
      }

      if (hasChanges || currentTotal > 0) {
        this.notify(false);
      }
    }, 500);
  }

  private processScheduledQueue() {
    if (this.getTasks().some((t) => t.status === 'queued') && this.isNightSchedulerActive()) this.processQueue();
  }

  public destroy() {
    this.destroyed = true;
    for (const id of this.runs.keys()) this.cancelRun(id);
    saveTasksToStorage(this.getTasks());
    if (this.speedInterval !== null) {
      clearInterval(this.speedInterval);
      this.speedInterval = null;
    }
    if (this.notifyTimeout !== null) {
      clearTimeout(this.notifyTimeout);
      this.notifyTimeout = null;
    }
    if (this.storageSaveTimeout !== null) {
      clearTimeout(this.storageSaveTimeout);
      this.storageSaveTimeout = null;
    }
    if (this.tauriUnlisten) {
      this.tauriUnlisten();
      this.tauriUnlisten = null;
    }
    this.listeners.clear();
  }
}

export const downloadManager = new DownloadManager();
