import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DownloadManager } from './downloadManager';
import { STORAGE_SETTINGS_KEY, STORAGE_TASKS_KEY } from './storage';
import type { AppSettings, DownloadTask, DownloaderSnapshot, MediaToolsStatus } from '../types/downloader';

const { invoke, events } = vi.hoisted(() => ({
  invoke: vi.fn(),
  events: new Map<string, (event: { payload?: unknown }) => void>()
}));

vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (event: string, handler: (event: { payload?: unknown }) => void) => {
    events.set(event, handler);
    return () => {
      events.delete(event);
    };
  })
}));

function baseSettings(overrides: Partial<AppSettings> = {}): AppSettings {
  return {
    maxConcurrentDownloads: 3,
    maxConnectionsPerHost: 4,
    globalSpeedLimitBytes: 0,
    enableNotifications: true,
    enableSound: false,
    soundVolume: 0.5,
    defaultDirectory: 'C:\\Downloads',
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
    startWithWindows: false,
    ...overrides
  };
}

function makeTask(id: string, overrides: Partial<DownloadTask> = {}): DownloadTask {
  return {
    id,
    url: 'https://example.com/file.zip',
    originalUrl: 'https://example.com/file.zip',
    filename: 'file.zip',
    totalBytes: 2048,
    downloadedBytes: 0,
    status: 'queued',
    speedBytesPerSec: 0,
    etaSeconds: 0,
    chunks: [{ index: 0, startByte: 0, endByte: 2047, downloaded: 0, status: 'pending' }],
    createdAt: Date.now(),
    category: 'archives',
    queueId: 'main',
    probed: true,
    acceptRanges: true,
    ...overrides
  };
}

describe('DownloadManager Rust facade lifecycle', () => {
  let manager: DownloadManager | null = null;
  let tasks: DownloadTask[] = [];
  let settings: AppSettings = baseSettings();
  let updating = false;
  let nextId = 1;
  let overrideInvoke: ((name: string, args: Record<string, unknown>) => unknown) | null = null;

  const calls = (name: string) => invoke.mock.calls.filter(([cmd]) => cmd === name);

  function snapshot(): DownloaderSnapshot {
    const activeCount = tasks.filter((t) => t.status === 'downloading').length;
    const queuedCount = tasks.filter((t) => t.status === 'queued').length;
    const completedCount = tasks.filter((t) => t.status === 'completed').length;
    const totalBytesDownloaded = tasks.reduce((s, t) => s + t.downloadedBytes, 0);
    const currentTotalSpeed = tasks.reduce((s, t) => s + (t.status === 'downloading' ? t.speedBytesPerSec : 0), 0);
    return {
      tasks: tasks.map((t) => ({ ...t, chunks: t.chunks.map((c) => ({ ...c })) })),
      stats: {
        activeCount,
        queuedCount,
        completedCount,
        totalBytesDownloaded,
        currentTotalSpeed,
        speedLimitBytesPerSec: settings.globalSpeedLimitBytes
      },
      settings: { ...settings },
      speedHistory: new Array(20).fill(0),
      updating
    };
  }

  function defaultInvoke(name: string, args: Record<string, unknown> = {}): unknown {
    if (overrideInvoke) {
      const result = overrideInvoke(name, args);
      if (result !== undefined) return result;
    }
    if (name === 'get_default_download_directory') return settings.defaultDirectory;
    if (name === 'set_browser_bridge_ready') return null;
    if (name === 'dl_snapshot') return snapshot();
    if (name === 'dl_migrate_legacy') return null;
    if (name === 'dl_update_settings') {
      settings = { ...settings, ...(args.patch as Partial<AppSettings>) };
      return { ...settings };
    }
    if (name === 'dl_add_task') {
      const req = args.request as Record<string, unknown>;
      const task = makeTask(`task_${nextId++}`, {
        url: String(req.url),
        originalUrl: String(req.url),
        filename: String(req.filename || 'file.zip'),
        requestedSegments: Number(req.segmentCount || settings.defaultSegmentCount || 8),
        queueId: (req.queueId as DownloadTask['queueId']) || 'main',
        mirrors: (req.mirrors as string[] | null) || undefined,
        headers: (req.headers as Record<string, string> | null) || undefined
      });
      tasks.push(task);
      return task;
    }
    if (name === 'dl_add_batch') {
      const urls = args.urls as string[];
      for (const u of urls) {
        tasks.push(makeTask(`task_${nextId++}`, { url: u, originalUrl: u }));
      }
      return tasks;
    }
    if (name === 'dl_pause_task') {
      const found = tasks.find((t) => t.id === args.id);
      if (found) found.status = 'paused';
      return null;
    }
    if (name === 'dl_resume_task') {
      const found = tasks.find((t) => t.id === args.id);
      if (found) found.status = args.force ? 'downloading' : 'queued';
      return null;
    }
    if (name === 'dl_restart_task') {
      const found = tasks.find((t) => t.id === args.id);
      if (found) {
        found.downloadedBytes = 0;
        found.status = 'queued';
        found.checksum = found.checksum?.expected
          ? { algorithm: 'sha256', expected: found.checksum.expected }
          : undefined;
      }
      return null;
    }
    if (name === 'dl_refresh_url') {
      const found = tasks.find((t) => t.id === args.id);
      if (found) {
        found.url = String(args.url);
        found.status = 'queued';
      }
      return null;
    }
    if (name === 'dl_pause_all') {
      for (const t of tasks) if (t.status === 'downloading' || t.status === 'queued') t.status = 'paused';
      return null;
    }
    if (name === 'dl_resume_all') {
      for (const t of tasks) if (t.status === 'paused' || t.status === 'error') t.status = 'queued';
      return null;
    }
    if (name === 'dl_remove_task') {
      tasks = tasks.filter((t) => t.id !== args.id);
      return null;
    }
    if (name === 'dl_clear_completed') {
      tasks = tasks.filter((t) => t.status !== 'completed');
      return null;
    }
    if (name === 'dl_set_task_queue') {
      const found = tasks.find((t) => t.id === args.id);
      if (found) found.queueId = args.queueId as DownloadTask['queueId'];
      return null;
    }
    if (name === 'dl_set_task_speed_limit') {
      const found = tasks.find((t) => t.id === args.id);
      if (found) found.speedLimitBytesPerSec = Number(args.bytesPerSec);
      return null;
    }
    if (name === 'dl_verify_hash') {
      const hash = 'a'.repeat(64);
      const found = tasks.find((t) => t.id === args.id);
      if (found) found.checksum = { algorithm: 'sha256', expected: found.checksum?.expected, actual: hash, verified: true };
      return hash;
    }
    if (name === 'dl_set_updating') {
      updating = Boolean(args.active);
      return null;
    }
    if (name === 'probe_url') {
      return { content_length: 2048, accept_ranges: true, content_type: 'application/zip' };
    }
    if (name === 'media_tools_status' || name === 'install_ffmpeg_tool' || name === 'update_ytdlp_tool') {
      const st: MediaToolsStatus = {
        hasYtdlp: true,
        hasNode: true,
        hasFfmpeg: name === 'install_ffmpeg_tool',
        toolsDir: 'C:\\Tools'
      };
      return st;
    }
    return null;
  }

  let memoryStorage: Record<string, string> = {};
  let windowObj: Record<string, unknown> = {};

  function installGlobals(withTauri = true) {
    memoryStorage = {};
    const mockStorage: Storage = {
      get length() {
        return Object.keys(memoryStorage).length;
      },
      key: (i: number) => Object.keys(memoryStorage)[i] ?? null,
      getItem: (k: string) => memoryStorage[k] ?? null,
      setItem: (k: string, v: string) => {
        memoryStorage[k] = v;
      },
      removeItem: (k: string) => {
        delete memoryStorage[k];
      },
      clear: () => {
        memoryStorage = {};
      }
    };
    windowObj = {
      localStorage: mockStorage,
      ...(withTauri ? { __TAURI_INTERNALS__: { invoke } } : {})
    };
    vi.stubGlobal('localStorage', mockStorage);
    vi.stubGlobal('window', windowObj);
  }

  beforeEach(() => {
    events.clear();
    tasks = [];
    settings = baseSettings();
    updating = false;
    nextId = 1;
    overrideInvoke = null;
    invoke.mockReset();
    invoke.mockImplementation((name: string, args: Record<string, unknown>) => Promise.resolve(defaultInvoke(name, args)));
    installGlobals(true);
  });

  afterEach(() => {
    manager?.destroy();
    manager = null;
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('migrates legacy localStorage state once and marks browser bridge ready', async () => {
    localStorage.setItem(
      STORAGE_TASKS_KEY,
      JSON.stringify([makeTask('legacy_1', { url: 'https://example.com/old.zip', filename: 'old.zip' })])
    );
    localStorage.setItem(STORAGE_SETTINGS_KEY, JSON.stringify({ theme: 'light', maxConcurrentDownloads: 5 }));

    manager = new DownloadManager();
    await vi.waitFor(() => expect(manager?.getRuntimeStatus()).toBe('ready'));

    expect(calls('dl_migrate_legacy')).toHaveLength(1);
    expect(calls('dl_update_settings')[0][1]).toMatchObject({
      patch: expect.objectContaining({ theme: 'light', maxConcurrentDownloads: 5 })
    });
    expect(calls('set_browser_bridge_ready')).toHaveLength(1);
    expect(localStorage.getItem('argon_migrated_to_rust_v1')).toBe('1');

    // Second startup must skip duplicate migration
    manager.destroy();
    invoke.mockClear();
    manager = new DownloadManager();
    await vi.waitFor(() => expect(manager?.getRuntimeStatus()).toBe('ready'));
    expect(calls('dl_migrate_legacy')).toHaveLength(0);
  });

  it('operates safely in browser preview mode without simulating native downloads', async () => {
    delete windowObj.__TAURI_INTERNALS__;
    manager = new DownloadManager();
    expect(manager.getRuntimeStatus()).toBe('preview');

    const previewId = manager.addTask('https://example.com/setup.exe');
    expect(manager.getTasks().find((t) => t.id === previewId)?.status).toBe('error');
    await expect(manager.updateSettings({ defaultDirectory: 'D:\\Other' })).rejects.toThrow();

    const imported = manager.importTasks('https://example.com/a.zip\nhttps://example.com/b.zip');
    expect(imported).toBe(2);
    expect(JSON.parse(manager.exportTasksJson())).toHaveLength(3);
    expect(manager.getTasks().every((t) => t.totalBytes === 0)).toBe(true);
  });

  it('rejects magnets and non-HTTP(S) URLs explicitly without invoking backend transfer', async () => {
    manager = new DownloadManager();
    await vi.waitFor(() => expect(manager?.getRuntimeStatus()).toBe('ready'));

    for (const url of [
      'magnet:?xt=urn:btih:da39a3ee5e6b4b0d3255bfef95601890afd80709',
      'file:///secret',
      'nonsense'
    ]) {
      const id = await manager.addTaskAsync(url);
      const t = manager.getTasks().find((item) => item.id === id);
      expect(t?.status).toBe('error');
      expect(t?.errorMessage).toBeTruthy();
    }
    expect(calls('dl_add_task')).toHaveLength(0);
  });

  it('creates tasks via dl_add_task with mirrors, custom headers, and checksum, and reconciles optimistic tasks', async () => {
    manager = new DownloadManager();
    await vi.waitFor(() => expect(manager?.getRuntimeStatus()).toBe('ready'));

    const createdId = await manager.addTaskAsync(
      'https://primary.example/app.exe',
      'custom-app.exe',
      16,
      'a'.repeat(64),
      { Authorization: 'Bearer tok' },
      undefined,
      'night',
      ['https://mirror.example/app.exe']
    );

    expect(createdId).toBe('task_1');
    expect(calls('dl_add_task')[0][1]).toEqual({
      request: {
        url: 'https://primary.example/app.exe',
        filename: 'custom-app.exe',
        segmentCount: 16,
        expectedSha256: 'a'.repeat(64),
        headers: { Authorization: 'Bearer tok' },
        mediaInfo: null,
        queueId: 'night',
        mirrors: ['https://mirror.example/app.exe']
      }
    });

    // Synchronous addTask creates optimistic entry and swaps cleanly with native task
    const optId = manager.addTask('https://example.com/two.bin');
    expect(optId.startsWith('pending_')).toBe(true);
    await vi.waitFor(() => expect(manager?.getTasks().some((t) => t.id === 'task_2')).toBe(true));
    expect(manager.getTasks().some((t) => t.id === optId)).toBe(false);
  });

  it('delegates pause, resume, force-start, restart, speed limit, hash verify, and batch actions to Rust', async () => {
    manager = new DownloadManager();
    await vi.waitFor(() => expect(manager?.getRuntimeStatus()).toBe('ready'));

    const id = await manager.addTaskAsync('https://example.com/archive.zip');
    manager.pauseTask(id);
    await vi.waitFor(() => expect(manager?.getTasks()[0].status).toBe('paused'));

    manager.startTaskImmediately(id);
    await vi.waitFor(() => expect(calls('dl_resume_task')[0][1]).toEqual({ id, force: true }));

    manager.restartTask(id);
    await vi.waitFor(() => expect(calls('dl_restart_task')).toHaveLength(1));

    await manager.setTaskSpeedLimit(id, 512_000);
    expect(calls('dl_set_task_speed_limit')[0][1]).toEqual({ id, bytesPerSec: 512000 });
    expect(manager.getTasks()[0].speedLimitBytesPerSec).toBe(512_000);

    manager.setTaskQueue(id, 'media');
    await vi.waitFor(() => expect(calls('dl_set_task_queue')[0][1]).toEqual({ id, queueId: 'media' }));

    const digest = await manager.verifyHash(id);
    expect(digest).toBe('a'.repeat(64));

    manager.pauseAll();
    manager.resumeAll();
    await vi.waitFor(() => {
      expect(calls('dl_pause_all')).toHaveLength(1);
      expect(calls('dl_resume_all')).toHaveLength(1);
    });
  });

  it('keeps the task and reports an error if disk deletion rejects', async () => {
    overrideInvoke = (name) => {
      if (name === 'dl_remove_task') throw new Error('Permission denied');
      return undefined;
    };
    manager = new DownloadManager();
    await vi.waitFor(() => expect(manager?.getRuntimeStatus()).toBe('ready'));

    const id = await manager.addTaskAsync('https://example.com/protected.zip');
    await manager.removeTask(id, true);

    const remaining = manager.getTasks().find((t) => t.id === id);
    expect(remaining).toBeTruthy();
    expect(remaining?.errorMessage).toContain('Permission denied');
  });

  it('validates expired URL replacement locally and propagates Rust rejection errors', async () => {
    overrideInvoke = (name) => {
      if (name === 'dl_refresh_url') throw new Error('Remote size mismatch');
      return undefined;
    };
    manager = new DownloadManager();
    await vi.waitFor(() => expect(manager?.getRuntimeStatus()).toBe('ready'));

    const id = await manager.addTaskAsync('https://example.com/file.zip');
    // Queued task cannot replace URL yet
    expect(() => manager!.refreshTaskUrl(id, 'https://example.com/file.zip?v=2')).toThrow('Pause the download');

    manager.pauseTask(id);
    await vi.waitFor(() => expect(manager?.getTasks()[0].status).toBe('paused'));

    expect(() => manager!.refreshTaskUrl(id, 'ftp://example.com/file.zip')).toThrow('HTTP or HTTPS');
    await expect(manager.refreshTaskUrl(id, 'https://example.com/file.zip?token=fresh')).rejects.toThrow(
      'Remote size mismatch'
    );
  });

  it('guards update installation while active downloads are running and toggles dl_set_updating', async () => {
    manager = new DownloadManager();
    await vi.waitFor(() => expect(manager?.getRuntimeStatus()).toBe('ready'));

    const t = makeTask('active_1', { status: 'downloading', speedBytesPerSec: 120000 });
    tasks.push(t);
    await manager.refreshFromNative();

    expect(() => manager!.beginUpdateInstallation()).toThrow('transfers_active');

    t.status = 'completed';
    t.speedBytesPerSec = 0;
    await manager.refreshFromNative();

    manager.beginUpdateInstallation();
    await vi.waitFor(() => expect(calls('dl_set_updating').some((c) => c[1].active === true)).toBe(true));

    manager.endUpdateInstallation();
    await vi.waitFor(() => expect(calls('dl_set_updating').some((c) => c[1].active === false)).toBe(true));
  });

  it('handles native download-state-changed events, settings persistence, and media tool commands', async () => {
    manager = new DownloadManager();
    await vi.waitFor(() => expect(manager?.getRuntimeStatus()).toBe('ready'));

    await manager.updateSettings({ theme: 'light', maxConnectionsPerHost: 8, minimizeToTray: false });
    expect(manager.getSettings().theme).toBe('light');
    expect(manager.getSettings().maxConnectionsPerHost).toBe(8);
    expect(manager.getSettings().minimizeToTray).toBe(false);

    tasks.push(makeTask('dl_1', { status: 'completed', downloadedBytes: 2048 }));
    events.get('download-state-changed')?.({});
    await vi.waitFor(() => expect(manager?.getTasks()).toHaveLength(1));

    manager.clearCompleted();
    await vi.waitFor(() => expect(manager?.getTasks()).toHaveLength(0));

    const before = await manager.mediaToolsStatus();
    expect(before.hasFfmpeg).toBe(false);
    const afterInstall = await manager.installFfmpegTool();
    expect(afterInstall.hasFfmpeg).toBe(true);
    await manager.updateYtdlpTool();
    expect(calls('update_ytdlp_tool')).toHaveLength(1);
  });
});
