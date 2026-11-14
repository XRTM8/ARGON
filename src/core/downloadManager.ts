import type {
  AppSettings,
  DownloadStats,
  DownloadTask,
  DownloaderSnapshot,
  MediaMetadata,
  MediaToolsStatus,
  QueueId
} from '../types/downloader';
import {
  calculateSegments,
  deduplicateFilename,
  detectCategory,
  extractFilename,
  sanitizeFilename
} from './segmentation';
import { resolveUniversalLink } from './linkResolver';
import { isAndroidRuntime, syncSystemBarTheme } from './safeArea';
import { soundEngine } from './sound';
import { telemetryStore } from './telemetryStore';
import { isMagnetUri, parseMagnetUri, type TorrentMetadata } from './torrent';
import {
  getStorage,
  loadSettingsFromStorage,
  loadTasksFromStorage
} from './storage';

const MIGRATED_KEY = 'argon_migrated_to_rust_v1';

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

const DEFAULT_SETTINGS: AppSettings = {
  maxConcurrentDownloads: 3,
  maxConnectionsPerHost: 4,
  globalSpeedLimitBytes: 0,
  enableNotifications: true,
  enableSound: true,
  soundVolume: 0.5,
  defaultDirectory: 'Downloads',
  defaultSegmentCount: 8,
  autoShutdownOnComplete: false,
  nightSchedulerEnabled: false,
  nightSchedulerStartHour: 2,
  nightSchedulerEndHour: 8,
  theme: 'dark',
  dynamicWorkStealing: true,
  clipboardMonitorEnabled: false,
  autoVerifyIntegrity: false,
  autoRetryEnabled: true,
  maxAutoRetries: 3,
  minimizeToTray: true,
  startWithWindows: true
};

export class DownloadManager {
  private ready: Promise<void>;
  private runtimeError?: string;
  private runtimeReady = false;
  private destroyed = false;
  private tasks: Map<string, DownloadTask> = new Map();
  private listeners: Set<() => void> = new Set();
  private tauriUnlisten: (() => void) | null = null;
  private notifyTimeout: number | null = null;
  private refreshTimeout: number | null = null;
  private refreshing = false;
  private refreshQueued = false;
  private speedHistory: number[] = new Array(20).fill(0);
  private stats: DownloadStats = {
    activeCount: 0,
    queuedCount: 0,
    completedCount: 0,
    totalBytesDownloaded: 0,
    currentTotalSpeed: 0,
    speedLimitBytesPerSec: 0
  };
  private settings: AppSettings = { ...DEFAULT_SETTINGS };
  private boundActiveIds = new Set<string>();
  private lastAndroidNotifyAt = 0;
  private lastAndroidActiveCount = 0;

  constructor() {
    this.loadLocalPreviewFallback();
    this.ready = this.initNative()
      .then(() => {
        this.runtimeReady = true;
        this.notify(true);
      })
      .catch((error) => {
        this.runtimeError = String(error);
        this.notify(true);
      });
  }

  private loadLocalPreviewFallback() {
    const saved = loadSettingsFromStorage();
    if (saved) {
      this.settings = { ...this.settings, ...saved };
    }
    soundEngine.enabled = this.settings.enableSound;
    soundEngine.volume = this.settings.soundVolume ?? 0.5;
    syncSystemBarTheme(this.settings.theme ?? 'dark');

    if (!isTauri()) {
      for (const t of loadTasksFromStorage()) {
        this.tasks.set(t.id, t);
      }
      this.recomputePreviewStats();
    }
  }

  private async initNative(): Promise<void> {
    if (!isTauri()) return;
    const [{ invoke }, { listen }] = await Promise.all([
      import('@tauri-apps/api/core'),
      import('@tauri-apps/api/event')
    ]);

    const unlistenState = await listen('download-state-changed', () => {
      this.scheduleRefresh();
    });

    const unlistenEvent = await listen<{
      type: string;
      data?: { taskId?: string; filename?: string; error?: string };
    }>('download-event', (event) => {
      const payload = event.payload;
      if (!payload) return;
      if (payload.type === 'taskCompleted') {
        soundEngine.playCompleteChime();
        this.showNotification('اكتمل التنزيل', `تم حفظ الملف: ${payload.data?.filename || ''}`);
      } else if (payload.type === 'taskFailed') {
        soundEngine.playErrorTone();
      } else if (payload.type === 'allIdleShutdownPending') {
        this.showNotification('إيقاف التشغيل المجدول', 'اكتملت جميع التحميلات، جاري إيقاف تشغيل الكمبيوتر...');
      }
      this.scheduleRefresh();
    });

    const unlistenClipboard = await listen<string>('argon-clipboard-url', (event) => {
      const url = event.payload;
      if (url && typeof window !== 'undefined') {
        window.dispatchEvent(new CustomEvent('argon-share-url', { detail: url }));
      }
    });

    this.tauriUnlisten = () => {
      unlistenState();
      unlistenEvent();
      unlistenClipboard();
    };
    if (this.destroyed) {
      this.tauriUnlisten();
      this.tauriUnlisten = null;
      return;
    }

    // One-time migration from legacy localStorage to Rust persistence
    const storage = getStorage();
    if (storage && !storage.getItem(MIGRATED_KEY)) {
      const legacyTasks = loadTasksFromStorage().filter((t) => !t.torrentInfo);
      if (legacyTasks.length > 0) {
        await invoke('dl_migrate_legacy', { tasks: legacyTasks }).catch(() => {});
      }
      const legacySettings = loadSettingsFromStorage();
      if (legacySettings) {
        await invoke('dl_update_settings', {
          patch: {
            maxConcurrentDownloads: legacySettings.maxConcurrentDownloads,
            globalSpeedLimitBytes: legacySettings.globalSpeedLimitBytes,
            enableNotifications: legacySettings.enableNotifications,
            enableSound: legacySettings.enableSound,
            soundVolume: legacySettings.soundVolume,
            defaultSegmentCount: legacySettings.defaultSegmentCount,
            autoShutdownOnComplete: legacySettings.autoShutdownOnComplete,
            nightSchedulerEnabled: legacySettings.nightSchedulerEnabled,
            nightSchedulerStartHour: legacySettings.nightSchedulerStartHour,
            nightSchedulerEndHour: legacySettings.nightSchedulerEndHour,
            theme: legacySettings.theme,
            dynamicWorkStealing: legacySettings.dynamicWorkStealing,
            clipboardMonitorEnabled: legacySettings.clipboardMonitorEnabled,
            autoVerifyIntegrity: legacySettings.autoVerifyIntegrity
          }
        }).catch(() => {});
      }
      try {
        storage.setItem(MIGRATED_KEY, '1');
      } catch {
        // Ignore storage errors
      }
    }

    await invoke('get_default_download_directory');
    await this.refreshFromNative();
    await invoke('set_browser_bridge_ready').catch(() => {});
  }

  private scheduleRefresh() {
    if (this.destroyed || this.refreshTimeout !== null) return;
    this.refreshTimeout = globalThis.setTimeout(() => {
      this.refreshTimeout = null;
      void this.refreshFromNative();
    }, 80) as unknown as number;
  }

  public async refreshFromNative(): Promise<void> {
    if (!isTauri() || this.destroyed) return;
    if (this.refreshing) {
      this.refreshQueued = true;
      return;
    }
    this.refreshing = true;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const snap = await invoke<DownloaderSnapshot>('dl_snapshot');
      if (this.destroyed) return;
      this.applySnapshot(snap);
    } catch {
      // Transient IPC read error
    } finally {
      this.refreshing = false;
      if (this.refreshQueued && !this.destroyed) {
        this.refreshQueued = false;
        void this.refreshFromNative();
      }
    }
  }

  private applySnapshot(snap: DownloaderSnapshot) {
    // Keep any in-flight optimistic pending tasks that haven't resolved yet
    const next = new Map<string, DownloadTask>();
    for (const t of snap.tasks) {
      next.set(t.id, t);
    }
    for (const [id, existing] of this.tasks.entries()) {
      if (id.startsWith('pending_') && !next.has(id)) {
        next.set(id, existing);
      }
    }
    this.tasks = next;
    this.stats = snap.stats;
    this.settings = { ...DEFAULT_SETTINGS, ...snap.settings };
    this.speedHistory = snap.speedHistory?.length ? snap.speedHistory : new Array(20).fill(0);

    soundEngine.enabled = this.settings.enableSound;
    soundEngine.volume = this.settings.soundVolume ?? 0.5;
    syncSystemBarTheme(this.settings.theme ?? 'dark');

    const activeIds = new Set<string>();
    for (const t of snap.tasks) {
      if (t.status === 'downloading') {
        activeIds.add(t.id);
        if (!this.boundActiveIds.has(t.id)) {
          this.boundActiveIds.add(t.id);
          telemetryStore.bindNativeRun(t.id, t.id);
        }
      }
    }
    for (const prev of Array.from(this.boundActiveIds)) {
      if (!activeIds.has(prev)) {
        this.boundActiveIds.delete(prev);
        telemetryStore.bindNativeRun(prev, null);
      }
    }

    this.syncAndroidNotification(Date.now());
    this.notify(false);
  }

  private recomputePreviewStats() {
    let activeCount = 0;
    let queuedCount = 0;
    let completedCount = 0;
    let totalBytesDownloaded = 0;
    let currentTotalSpeed = 0;
    for (const t of this.tasks.values()) {
      if (t.status === 'downloading') {
        activeCount++;
        currentTotalSpeed += t.speedBytesPerSec;
      } else if (t.status === 'queued') queuedCount++;
      else if (t.status === 'completed') completedCount++;
      totalBytesDownloaded += t.downloadedBytes;
    }
    this.stats = {
      activeCount,
      queuedCount,
      completedCount,
      totalBytesDownloaded,
      currentTotalSpeed,
      speedLimitBytesPerSec: this.settings.globalSpeedLimitBytes
    };
  }

  private showNotification(title: string, body: string) {
    if (!this.settings.enableNotifications || typeof window === 'undefined') return;
    try {
      if ('Notification' in window && Notification.permission === 'granted') {
        new Notification(title, { body });
      }
    } catch {
      // Notification unsupported
    }
  }

  public getRuntimeStatus(): 'preview' | 'starting' | 'ready' | 'error' {
    return !isTauri() ? 'preview' : this.runtimeError ? 'error' : this.runtimeReady ? 'ready' : 'starting';
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
    }, 120) as unknown as number;
  }

  public async triggerShutdown(cancel = false): Promise<boolean> {
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

  public getTasks(): DownloadTask[] {
    return Array.from(this.tasks.values()).sort((a, b) => b.createdAt - a.createdAt);
  }

  public getStats(): DownloadStats {
    return { ...this.stats };
  }

  public getSettings(): AppSettings {
    return { ...this.settings };
  }

  public async updateSettings(patch: Partial<AppSettings>) {
    if (!isTauri()) {
      this.settings = { ...this.settings, ...patch };
      if (patch.theme === 'light' || patch.theme === 'dark') {
        syncSystemBarTheme(patch.theme);
      }
      this.notify(true);
      if (patch.defaultDirectory !== undefined) {
        throw new Error('تغيير مجلد الحفظ يتطلب تطبيق ARGON لسطح المكتب.');
      }
      return;
    }
    if (
      patch.enableNotifications &&
      typeof window !== 'undefined' &&
      'Notification' in window &&
      Notification.permission === 'default'
    ) {
      Notification.requestPermission().catch(() => {});
    }
    await this.ready;
    const { invoke } = await import('@tauri-apps/api/core');
    const updated = await invoke<AppSettings>('dl_update_settings', { patch });
    this.settings = { ...DEFAULT_SETTINGS, ...updated };
    soundEngine.enabled = this.settings.enableSound;
    soundEngine.volume = this.settings.soundVolume ?? 0.5;
    syncSystemBarTheme(this.settings.theme ?? 'dark');
    this.notify(true);
  }

  public addTask(
    url: string,
    customFilename?: string,
    segmentCount: number = this.settings.defaultSegmentCount || 8,
    expectedSha256?: string,
    customHeaders?: Record<string, string>,
    torrentMeta?: TorrentMetadata,
    mediaInfo?: MediaMetadata,
    _deferStart = false,
    queueId: QueueId = 'main',
    mirrors?: string[]
  ): string {
    const originalUrl = url.trim();
    const resolved = resolveUniversalLink(originalUrl);
    const isMagnet = isMagnetUri(originalUrl);
    const meta = torrentMeta || (isMagnet ? parseMagnetUri(originalUrl) || undefined : undefined);
    const tempId = `pending_${Date.now()}_${Math.random().toString(36).slice(2, 8)}`;
    const filename = deduplicateFilename(
      sanitizeFilename(customFilename?.trim() || meta?.name || mediaInfo?.title || extractFilename(resolved.resolvedUrl)),
      this.getTasks().map((t) => t.filename)
    );

    let localError: string | undefined;
    if (meta || isMagnet) {
      localError = 'روابط Magnet والتورنت تحتاج محرك P2P غير متاح في هذا الإصدار؛ استخدم رابط HTTP/HTTPS مباشر.';
    } else if (!/^https?:\/\/.+/i.test(resolved.resolvedUrl)) {
      localError = 'يُدعم فقط روابط HTTP و HTTPS الصالحة.';
    } else if (!isTauri()) {
      localError = 'التنزيل يتطلب تطبيق ARGON لسطح المكتب؛ هذه نافذة معاينة فقط.';
    }

    if (localError) {
      const errTask: DownloadTask = {
        id: tempId,
        originalUrl,
        url: resolved.resolvedUrl,
        filename,
        category: meta ? 'torrents' : detectCategory(filename),
        torrentInfo: meta,
        mediaInfo,
        headers: { ...resolved.suggestedHeaders, ...customHeaders },
        totalBytes: meta?.totalLength || 0,
        downloadedBytes: 0,
        status: 'error',
        errorMessage: localError,
        speedBytesPerSec: 0,
        etaSeconds: 0,
        chunks: calculateSegments(0, 1),
        createdAt: Date.now(),
        requestedSegments: Math.max(1, Math.min(32, Math.floor(segmentCount) || 1)),
        customFilename: !!customFilename?.trim(),
        filePrepared: false,
        queueId,
        mirrors
      };
      this.tasks.set(tempId, errTask);
      this.recomputePreviewStats();
      this.notify(true);
      return tempId;
    }

    soundEngine.playStartSweep();
    const optimistic: DownloadTask = {
      id: tempId,
      originalUrl,
      url: resolved.resolvedUrl,
      filename,
      category: detectCategory(filename),
      mediaInfo,
      headers: { ...resolved.suggestedHeaders, ...customHeaders },
      totalBytes: 0,
      downloadedBytes: 0,
      status: 'queued',
      speedBytesPerSec: 0,
      etaSeconds: 0,
      chunks: calculateSegments(0, 1),
      createdAt: Date.now(),
      requestedSegments: Math.max(1, Math.min(32, Math.floor(segmentCount) || 1)),
      customFilename: !!customFilename?.trim(),
      filePrepared: false,
      queueId,
      mirrors
    };
    this.tasks.set(tempId, optimistic);
    this.notify(true);

    void this.ready
      .then(async () => {
        if (this.destroyed) return;
        const { invoke } = await import('@tauri-apps/api/core');
        if (this.destroyed) return;
        const created = await invoke<DownloadTask>('dl_add_task', {
          request: {
            url: originalUrl,
            filename: customFilename?.trim() || null,
            segmentCount,
            expectedSha256: expectedSha256?.trim() || null,
            headers: customHeaders || null,
            mediaInfo: mediaInfo || null,
            queueId,
            mirrors: mirrors?.length ? mirrors : null
          }
        });
        if (this.destroyed) return;
        this.tasks.delete(tempId);
        this.tasks.set(created.id, created);
        this.notify(true);
        await this.refreshFromNative();
      })
      .catch((err) => {
        if (this.destroyed) return;
        const current = this.tasks.get(tempId);
        if (current) {
          current.status = 'error';
          current.errorMessage = String(err);
          this.notify(true);
        }
      });

    return tempId;
  }

  public async addTaskAsync(
    url: string,
    customFilename?: string,
    segmentCount?: number,
    expectedSha256?: string,
    customHeaders?: Record<string, string>,
    mediaInfo?: MediaMetadata,
    queueId: QueueId = 'main',
    mirrors?: string[]
  ): Promise<string> {
    const resolved = resolveUniversalLink(url.trim());
    if (!isTauri() || isMagnetUri(url) || !/^https?:\/\/.+/i.test(resolved.resolvedUrl)) {
      return this.addTask(url, customFilename, segmentCount, expectedSha256, customHeaders, undefined, mediaInfo, false, queueId, mirrors);
    }
    await this.ready;
    soundEngine.playStartSweep();
    const { invoke } = await import('@tauri-apps/api/core');
    const created = await invoke<DownloadTask>('dl_add_task', {
      request: {
        url: url.trim(),
        filename: customFilename?.trim() || null,
        segmentCount: segmentCount ?? this.settings.defaultSegmentCount ?? 8,
        expectedSha256: expectedSha256?.trim() || null,
        headers: customHeaders || null,
        mediaInfo: mediaInfo || null,
        queueId,
        mirrors: mirrors?.length ? mirrors : null
      }
    });
    this.tasks.set(created.id, created);
    this.notify(true);
    void this.refreshFromNative();
    return created.id;
  }

  public addTorrentTask(meta: TorrentMetadata, segmentCount?: number): string {
    const rawUrl = meta.isMagnet
      ? `magnet:?xt=urn:btih:${meta.infoHash}&dn=${encodeURIComponent(meta.name)}`
      : `torrent://${meta.infoHash}`;
    return this.addTask(rawUrl, meta.name, segmentCount || this.settings.defaultSegmentCount, undefined, undefined, meta);
  }

  public addBatchTasks(urls: string[], segmentCount?: number): string[] {
    const valid = urls.map((u) => u.trim()).filter((u) => /^https?:\/\/.+/i.test(u));
    if (!valid.length) return [];
    if (!isTauri()) {
      return valid.map((u) => this.addTask(u, undefined, segmentCount));
    }
    void this.ready
      .then(async () => {
        if (this.destroyed) return;
        const { invoke } = await import('@tauri-apps/api/core');
        if (this.destroyed) return;
        await invoke('dl_add_batch', { urls: valid, segmentCount: segmentCount ?? null, queueId: null });
        await this.refreshFromNative();
      })
      .catch(() => {});
    return [];
  }

  private fireNativeCommand(command: string, args?: Record<string, unknown>) {
    if (!isTauri() || this.destroyed) return;
    void import('@tauri-apps/api/core')
      .then(({ invoke }) => (this.destroyed ? undefined : invoke(command, args)))
      .then(() => {
        if (!this.destroyed) return this.refreshFromNative();
      })
      .catch(() => {});
  }

  public pauseTask(id: string) {
    this.fireNativeCommand('dl_pause_task', { id });
  }

  public startTaskImmediately(id: string) {
    this.resumeTask(id, true);
  }

  public resumeTask(id: string, force = false) {
    this.fireNativeCommand('dl_resume_task', { id, force });
  }

  public restartTask(id: string) {
    this.fireNativeCommand('dl_restart_task', { id });
  }

  public refreshTaskUrl(id: string, value: string): Promise<void> {
    const task = this.tasks.get(id);
    if (!task || !['paused', 'error'].includes(task.status)) {
      throw new Error('Pause the download before replacing its URL.');
    }
    let next: URL;
    try {
      next = new URL(value.trim());
      if (!['http:', 'https:'].includes(next.protocol)) throw new Error();
    } catch {
      throw new Error('Enter a complete HTTP or HTTPS URL.');
    }
    if (!isTauri()) return Promise.resolve();
    return import('@tauri-apps/api/core')
      .then(({ invoke }) => invoke('dl_refresh_url', { id, url: value.trim() }))
      .then(() => this.refreshFromNative());
  }

  public pauseAll() {
    this.fireNativeCommand('dl_pause_all');
  }

  public resumeAll() {
    this.fireNativeCommand('dl_resume_all');
  }

  public async removeTask(id: string, deleteFileFromDisk = false) {
    if (!isTauri()) {
      this.tasks.delete(id);
      this.recomputePreviewStats();
      this.notify(true);
      return;
    }
    const { invoke } = await import('@tauri-apps/api/core');
    try {
      await invoke('dl_remove_task', { id, deleteFileFromDisk });
      this.tasks.delete(id);
      telemetryStore.bindNativeRun(id, null);
      this.boundActiveIds.delete(id);
      this.notify(true);
      await this.refreshFromNative();
    } catch (error) {
      const task = this.tasks.get(id);
      if (task) {
        task.errorMessage = `Could not delete file: ${String(error)}`;
        this.notify(true);
      }
    }
  }

  public pauseTasks(ids: string[]) {
    for (const id of ids) this.pauseTask(id);
  }

  public resumeTasks(ids: string[]) {
    for (const id of ids) this.resumeTask(id);
  }

  public removeTasks(ids: string[], deleteFiles = false) {
    for (const id of ids) void this.removeTask(id, deleteFiles);
  }

  public clearCompleted() {
    if (!isTauri()) {
      for (const [id, task] of this.tasks.entries()) {
        if (task.status === 'completed') this.tasks.delete(id);
      }
      this.recomputePreviewStats();
      this.notify(true);
      return;
    }
    this.fireNativeCommand('dl_clear_completed');
  }

  public setTaskQueue(id: string, queueId: QueueId) {
    this.fireNativeCommand('dl_set_task_queue', { id, queueId });
  }

  public async setTaskSpeedLimit(id: string, bytesPerSec: number) {
    if (!isTauri()) return;
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('dl_set_task_speed_limit', { id, bytesPerSec: Math.max(0, Math.floor(bytesPerSec)) });
    await this.refreshFromNative();
  }

  public async verifyHash(id: string): Promise<string | null> {
    if (!isTauri()) return null;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const hash = await invoke<string | null>('dl_verify_hash', { id });
      await this.refreshFromNative();
      return hash;
    } catch {
      return null;
    }
  }

  public async probe(url: string, customHeaders?: Record<string, string>): Promise<ProbeResult> {
    if (!isTauri()) throw new Error('التنزيل يتطلب تطبيق ARGON لسطح المكتب؛ هذه نافذة معاينة فقط.');
    const { invoke } = await import('@tauri-apps/api/core');
    return invoke<ProbeResult>('probe_url', { url, customHeaders: customHeaders || null });
  }

  public async mediaToolsStatus(): Promise<MediaToolsStatus> {
    if (!isTauri()) return { hasYtdlp: false, hasNode: false, hasFfmpeg: false, toolsDir: '' };
    const { invoke } = await import('@tauri-apps/api/core');
    return invoke<MediaToolsStatus>('media_tools_status');
  }

  public async installFfmpegTool(): Promise<MediaToolsStatus> {
    if (!isTauri()) throw new Error('يتطلب تطبيق سطح المكتب.');
    const { invoke } = await import('@tauri-apps/api/core');
    return invoke<MediaToolsStatus>('install_ffmpeg_tool');
  }

  public async updateYtdlpTool(): Promise<MediaToolsStatus> {
    if (!isTauri()) throw new Error('يتطلب تطبيق سطح المكتب.');
    const { invoke } = await import('@tauri-apps/api/core');
    return invoke<MediaToolsStatus>('update_ytdlp_tool');
  }

  public beginUpdateInstallation() {
    if (this.stats.activeCount > 0) throw new Error('transfers_active');
    if (isTauri()) {
      void import('@tauri-apps/api/core').then(({ invoke }) =>
        invoke('dl_set_updating', { active: true }).catch(() => {})
      );
    }
  }

  public endUpdateInstallation() {
    if (isTauri()) {
      void import('@tauri-apps/api/core').then(({ invoke }) =>
        invoke('dl_set_updating', { active: false }).catch(() => {})
      );
    }
  }

  public async shareFile(filePath?: string) {
    if (!filePath || !isTauri()) throw new Error('Open the ARGON app to share saved files.');
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('share_file_item', { filePath });
  }

  public async openFile(filePath?: string) {
    if (!filePath || !isTauri()) throw new Error('Open the ARGON app to access saved files.');
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('open_file_item', { filePath });
  }

  public async openFolder(filePath?: string) {
    if (!filePath || !isTauri()) throw new Error('Open the ARGON app to access saved folders.');
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('open_destination_folder', { filePath });
  }

  public async openBrowserExtensionDir() {
    if (!isTauri()) throw new Error('Open the ARGON desktop app to find the extension folder.');
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('open_browser_extension_dir');
  }

  public exportUrls(filter?: 'all' | 'active' | 'completed'): string {
    const tasks = this.getTasks();
    const filtered = tasks.filter((t) => {
      if (filter === 'active') return t.status === 'downloading' || t.status === 'queued' || t.status === 'paused';
      if (filter === 'completed') return t.status === 'completed';
      return true;
    });
    return filtered.map((t) => t.url).join('\n');
  }

  public exportTasksJson(): string {
    const list = this.getTasks().map((t) => ({
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

    for (const line of trimmed.split(/[\r\n]+/)) {
      const match = line.match(/https?:\/\/[^\s"']+/i);
      if (match) {
        this.addTask(match[0].trim(), undefined, segmentCount);
        addedCount++;
      }
    }
    return addedCount;
  }

  private syncAndroidNotification(now: number) {
    if (!isTauri() || !isAndroidRuntime() || this.destroyed) return;
    const active = this.getTasks().filter((t) => t.status === 'downloading');
    const activeCount = active.length;
    if (activeCount === 0 && this.lastAndroidActiveCount === 0) return;
    if (activeCount > 0 && now - this.lastAndroidNotifyAt < 1200) return;
    this.lastAndroidNotifyAt = now;
    this.lastAndroidActiveCount = activeCount;
    const totalKnown = active.reduce((s, t) => s + (t.totalBytes > 0 ? t.totalBytes : 0), 0);
    const totalDl = active.reduce((s, t) => s + t.downloadedBytes, 0);
    const totalSpeed = active.reduce((s, t) => s + t.speedBytesPerSec, 0);
    const pct = totalKnown > 0 ? Math.min(100, Math.round((totalDl / totalKnown) * 100)) : 0;
    const title = activeCount === 0 ? 'ARGON' : activeCount === 1 ? active[0].filename : `ARGON · ${activeCount} downloads`;
    const speedMb = totalSpeed > 0 ? `${(totalSpeed / (1024 * 1024)).toFixed(1)} MB/s` : 'Connecting…';
    const text = activeCount === 0 ? '' : totalKnown > 0 ? `${pct}% · ${speedMb}` : speedMb;
    void import('@tauri-apps/api/core')
      .then(({ invoke }) =>
        invoke('sync_android_transfer_notification', {
          activeCount,
          title,
          text,
          progress: pct,
          indeterminate: totalKnown <= 0
        })
      )
      .catch(() => {});
  }

  public destroy() {
    this.destroyed = true;
    if (this.notifyTimeout !== null) {
      clearTimeout(this.notifyTimeout);
      this.notifyTimeout = null;
    }
    if (this.refreshTimeout !== null) {
      clearTimeout(this.refreshTimeout);
      this.refreshTimeout = null;
    }
    if (this.tauriUnlisten) {
      this.tauriUnlisten();
      this.tauriUnlisten = null;
    }
    this.listeners.clear();
  }
}

export const downloadManager = new DownloadManager();
