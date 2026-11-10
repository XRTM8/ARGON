import { beforeEach, afterEach, describe, it, expect, vi } from 'vitest';
import { DownloadManager } from './downloadManager';
import type { DownloadTask } from '../types/downloader';

const { invoke, events } = vi.hoisted(() => ({ invoke: vi.fn(), events: new Map<string, (event: any) => void>() }));
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async (name, callback) => { events.set(name, callback); return () => events.delete(name); }) }));
vi.mock('./sound', () => ({ soundEngine: { enabled: true, volume: 0.5, playStartSweep: vi.fn(), playCompleteChime: vi.fn(), playErrorTone: vi.fn() } }));

const probe = { content_length: 2048, accept_ranges: true, content_type: 'application/octet-stream', etag: '"v1"' };
let manager: DownloadManager;
let implementation: (name: string, args: any) => unknown;
const calls = (name: string) => invoke.mock.calls.filter((call) => call[0] === name);
const task = (id: string) => manager.getTasks().find((t) => t.id === id)!;
const settle = async (id: string) => vi.waitFor(() => expect(['completed', 'error']).toContain(task(id).status));
function defaultInvoke(name: string, args: any): unknown {
  if (name === 'get_default_download_directory') return 'C:\\Downloads';
  if (name === 'probe_url') return { ...probe };
  if (name === 'prepare_download_file') return args.destinationPath;
  if (name === 'download_segment') return { downloaded_bytes: args.useRange ? args.endByte - args.startByte + 1 : args.expectedTotal || 2048, total_bytes: args.expectedTotal || 2048 };
  if (name === 'verify_file_sha256') return 'a'.repeat(64);
  if (name === 'resolve_media') return { downloadUrl: 'https://cdn.example.com/video.mp4', suggestedFilename: 'Video.mp4', headers: { Referer: 'https://www.youtube.com/' } };
}
beforeEach(() => {
  const memory = new Map<string, string>();
  const storage = { getItem: (k: string) => memory.get(k) ?? null, setItem: (k: string, v: string) => memory.set(k, v), removeItem: (k: string) => memory.delete(k) };
  vi.stubGlobal('window', { __TAURI_INTERNALS__: {}, setTimeout, clearTimeout, setInterval, clearInterval, localStorage: storage });
  vi.stubGlobal('localStorage', storage);
  events.clear(); invoke.mockReset(); implementation = defaultInvoke;
  invoke.mockImplementation(async (name, args) => implementation(name, args));
  manager = new DownloadManager();
});
afterEach(async () => { manager.destroy(); await Promise.resolve(); vi.unstubAllGlobals(); });

describe('ARGON real download lifecycle', () => {
  it('persists queued work and blocks new transfers until a failed update releases the queue', async () => {
    manager.beginUpdateInstallation();
    const id = manager.addTask('https://example.com/after-update.zip');
    expect(task(id).status).toBe('queued'); expect(calls('probe_url')).toHaveLength(0);
    manager.endUpdateInstallation(); await settle(id); expect(task(id).status).toBe('completed');
  });
  it('refreshes signed links without forwarding old credentials and revalidates partial data', async () => {
    const id = manager.addTask('https://example.com/file.zip?token=old'); await settle(id);
    const current = task(id); current.status = 'paused'; current.downloadedBytes = 512;
    current.chunks = [{ index: 0, startByte: 0, endByte: 2047, downloaded: 512, status: 'pending' }];
    current.headers = { Authorization: 'Bearer old-secret', Cookie: 'old-cookie' };
    manager.refreshTaskUrl(id, 'https://example.com/file.zip?token=new'); await settle(id);
    expect(task(id).url).toBe('https://example.com/file.zip?token=new');
    expect(task(id).headers).not.toHaveProperty('Authorization'); expect(task(id).headers).not.toHaveProperty('Cookie');
    expect(calls('prepare_download_file').at(-1)![1].reset).toBe(false);
    expect(calls('download_segment').at(-1)![1].currentDownloaded).toBe(512);
  });
  it('starts from zero when a replacement link identifies a different resource', async () => {
    const id = manager.addTask('https://example.com/first.zip'); await settle(id);
    task(id).status = 'paused'; task(id).downloadedBytes = 512;
    manager.refreshTaskUrl(id, 'https://another.example.org/different.zip'); await settle(id);
    expect(calls('prepare_download_file').at(-1)![1].reset).toBe(true);
  });
  it('keeps the task and reports a disk deletion failure', async () => {
    const id = manager.addTask('https://example.com/keep.zip'); await settle(id);
    implementation = (name, args) => { if (name === 'delete_file_item') throw new Error('access denied'); return defaultInvoke(name, args); };
    await manager.removeTask(id, true);
    expect(task(id).status).toBe('completed'); expect(task(id).errorMessage).toContain('access denied');
    expect(task(id).savePath).toContain('keep.zip');
  });
  it('persists a browser task before acknowledging or probing it', async () => {
    await vi.waitFor(() => expect(events.has('browser-add-download')).toBe(true));
    let release!: () => void;
    const pending = new Promise<void>((resolve) => { release = resolve; });
    implementation = (name, args) => {
      if (name === 'acknowledge_browser_download') {
        expect(args).toEqual({ requestId: 'bridge-test', accepted: true });
        expect(window.localStorage.getItem('otium_download_tasks_v1')).toContain('from-browser.zip');
        return pending;
      }
      return defaultInvoke(name, args);
    };
    events.get('browser-add-download')!({ payload: { url: 'https://example.com/from-browser.zip', request_id: 'bridge-test' } });
    await vi.waitFor(() => expect(calls('acknowledge_browser_download')).toHaveLength(1));
    manager.processQueue();
    expect(calls('probe_url')).toHaveLength(0);
    release();
    await vi.waitFor(() => expect(manager.getTasks()[0]?.status).toBe('completed'));
  });
  it('does not acknowledge or start a browser task that cannot be persisted', async () => {
    await vi.waitFor(() => expect(events.has('browser-add-download')).toBe(true));
    vi.spyOn(window.localStorage, 'setItem').mockImplementation(() => { throw new Error('quota'); });
    events.get('browser-add-download')!({ payload: { url: 'https://example.com/not-saved.zip', request_id: 'bridge-quota' } });
    await vi.waitFor(() => expect(calls('acknowledge_browser_download')[0]?.[1]).toEqual({ requestId: 'bridge-quota', accepted: false }));
    expect(manager.getTasks()).toHaveLength(0); expect(calls('probe_url')).toHaveLength(0);
  });
  it('removes an unconfirmed handoff and leaves the browser responsible when acknowledgement expires', async () => {
    await vi.waitFor(() => expect(events.has('browser-add-download')).toBe(true));
    implementation = (name, args) => { if (name === 'acknowledge_browser_download') throw new Error('expired'); return defaultInvoke(name, args); };
    events.get('browser-add-download')!({ payload: { url: 'https://example.com/expired.zip', request_id: 'bridge-expired' } });
    await vi.waitFor(() => expect(calls('acknowledge_browser_download')).toHaveLength(1));
    await vi.waitFor(() => expect(manager.getTasks()).toHaveLength(0));
    expect(calls('probe_url')).toHaveLength(0);
  });
  it('reports preview mode instead of simulating a download', () => {
    vi.stubGlobal('window', { setTimeout, clearTimeout, setInterval, clearInterval });
    const id = manager.addTask('https://example.com/file.zip');
    expect(task(id).status).toBe('error'); expect(task(id).errorMessage).toContain('معاينة');
    expect(task(id).downloadedBytes).toBe(0); expect(task(id).totalBytes).toBe(0);
    expect(calls('download_segment')).toHaveLength(0);
  });
  it('probes and verifies a real transfer before completion', async () => {
    const id = manager.addTask('https://example.com/file.zip', undefined, 4);
    await settle(id);
    expect(task(id).status).toBe('completed'); expect(task(id).totalBytes).toBe(2048);
    expect(task(id).downloadedBytes).toBe(2048); expect(calls('download_segment')).toHaveLength(4);
    expect(calls('validate_download_file')[0][1]).toMatchObject({ expectedBytes: 2048 });
  });
  it('uses the same probe pipeline when a queued task starts', async () => {
    await manager.updateSettings({ maxConcurrentDownloads: 1 });
    let release!: () => void;
    const blocked = new Promise<void>((resolve) => { release = resolve; });
    implementation = async (name, args) => {
      if (name === 'download_segment' && args.url.includes('first')) await blocked;
      return defaultInvoke(name, args);
    };
    const first = manager.addTask('https://example.com/first.zip');
    const second = manager.addTask('https://example.com/second.zip');
    expect(task(second).status).toBe('queued'); release();
    await settle(first); await settle(second);
    expect(calls('probe_url').map((c) => c[1].url)).toEqual(['https://example.com/first.zip', 'https://example.com/second.zip']);
  });
  it('keeps HTTP probe errors visible and never prepares the file', async () => {
    implementation = (name, args) => { if (name === 'probe_url') throw new Error('Server returned HTTP 403'); return defaultInvoke(name, args); };
    const id = manager.addTask('https://example.com/private.zip'); await settle(id);
    expect(task(id).errorMessage).toContain('403'); expect(calls('prepare_download_file')).toHaveLength(0);
  });
  it('rejects HTML pages instead of saving fake media', async () => {
    implementation = (name, args) => name === 'probe_url' ? { ...probe, content_type: 'text/html' } : defaultInvoke(name, args);
    const id = manager.addTask('https://example.com/view'); await settle(id);
    expect(task(id).status).toBe('error'); expect(calls('download_segment')).toHaveLength(0);
  });
  it('falls back from rejected ranges to one full stream', async () => {
    implementation = (name, args) => { if (name === 'download_segment' && args.useRange) throw new Error('RANGE_UNSUPPORTED'); return defaultInvoke(name, args); };
    const id = manager.addTask('https://example.com/file.zip', undefined, 4); await settle(id);
    expect(task(id).status).toBe('completed'); expect(task(id).chunks).toHaveLength(1);
    expect(calls('download_segment').filter((c) => !c[1].useRange)).toHaveLength(1);
    expect(calls('cancel_task').length).toBeGreaterThan(0);
  });
  it('handles unknown content length with one stream', async () => {
    implementation = (name, args) => name === 'probe_url' ? { accept_ranges: false } : defaultInvoke(name, args);
    const id = manager.addTask('https://example.com/stream'); await settle(id);
    expect(task(id).status).toBe('completed'); expect(task(id).totalBytes).toBe(2048);
    expect(calls('download_segment')[0][1]).toMatchObject({ useRange: false, expectedTotal: null });
  });
  it('waits for checksum verification after the last progress event', async () => {
    let finishHash!: (hash: string) => void;
    const pendingHash = new Promise<string>((resolve) => { finishHash = resolve; });
    implementation = (name, args) => {
      if (name === 'verify_file_sha256') return pendingHash;
      if (name === 'download_segment') events.get('download-progress')?.({ payload: { task_id: args.taskId, segment_index: args.segmentIndex, downloaded_bytes: args.endByte - args.startByte + 1, total_bytes: 2048, is_finished: true } });
      return defaultInvoke(name, args);
    };
    const id = manager.addTask('https://example.com/file.zip', undefined, 1, 'a'.repeat(64));
    await vi.waitFor(() => expect(calls('verify_file_sha256')).toHaveLength(1));
    expect(task(id).status).toBe('downloading'); finishHash('b'.repeat(64)); await settle(id);
    expect(task(id).status).toBe('error'); expect(task(id).checksum?.verified).toBe(false);
  });
  it('does not complete when disk verification fails', async () => {
    implementation = (name, args) => { if (name === 'validate_download_file') throw new Error('Incomplete file'); return defaultInvoke(name, args); };
    const id = manager.addTask('https://example.com/file.zip'); await settle(id);
    expect(task(id).status).toBe('error'); expect(task(id).errorMessage).toContain('Incomplete');
  });
  it('ignores progress from a cancelled attempt and serializes rapid resume', async () => {
    let finish!: () => void;
    const pending = new Promise<void>((resolve) => { finish = resolve; });
    let oldId = '';
    implementation = async (name, args) => {
      if (name === 'download_segment' && !oldId) { oldId = args.taskId; await pending; throw new Error('CANCELLED'); }
      return defaultInvoke(name, args);
    };
    const id = manager.addTask('https://example.com/file.zip', undefined, 1);
    await vi.waitFor(() => expect(oldId).not.toBe(''));
    manager.pauseTask(id); manager.resumeTask(id);
    events.get('download-progress')?.({ payload: { task_id: oldId, segment_index: 0, downloaded_bytes: 999999, total_bytes: 999999, is_finished: true } });
    expect(task(id).downloadedBytes).toBe(0); expect(calls('probe_url')).toHaveLength(1);
    finish(); await settle(id);
    expect(task(id).status).toBe('completed'); expect(calls('probe_url')).toHaveLength(2);
    expect(calls('download_segment')[1][1].taskId).not.toBe(oldId);
  });
  it('pause all leaves every queued task paused', async () => {
    await manager.updateSettings({ maxConcurrentDownloads: 1 });
    manager.addBatchTasks(['https://example.com/a.zip', 'https://example.com/b.zip', 'https://example.com/c.zip']);
    manager.pauseAll(); expect(manager.getTasks().every((t) => t.status === 'paused')).toBe(true);
    await Promise.resolve(); expect(calls('download_segment')).toHaveLength(0);
  });
  it('extracts social URLs for browser and batch entry points too', async () => {
    const id = manager.addTask('https://www.youtube.com/watch?v=test'); await settle(id);
    expect(calls('resolve_media')).toHaveLength(1);
    expect(calls('probe_url')[0][1]).toMatchObject({ url: 'https://cdn.example.com/video.mp4', customHeaders: { Referer: 'https://www.youtube.com/' } });
    expect(task(id).filename).toBe('Video.mp4');
  });
  it('preserves an imported audio-only request without inventing MP3 conversion', async () => {
    const id = manager.addTask('https://www.youtube.com/watch?v=test', undefined, 1, undefined, undefined, undefined, { isAudioOnly: true });
    await settle(id);
    expect(calls('resolve_media')[0][1]).toMatchObject({ audioOnly: true });
  });
  it('never downloads the original social page after extraction failure', async () => {
    implementation = (name, args) => { if (name === 'resolve_media') throw new Error('Extraction failed'); return defaultInvoke(name, args); };
    const id = manager.addTask('https://www.tiktok.com/@user/video/123'); await settle(id);
    expect(task(id).errorMessage).toContain('Extraction failed'); expect(calls('download_segment')).toHaveLength(0);
  });
  it('reports unsupported torrents and malformed URLs explicitly', () => {
    for (const url of ['magnet:?xt=urn:btih:da39a3ee5e6b4b0d3255bfef95601890afd80709', 'file:///secret', 'nonsense']) {
      const id = manager.addTask(url); expect(task(id).status).toBe('error'); expect(task(id).errorMessage).toBeTruthy();
    }
  });
  it('preserves custom filenames and uses native collision-safe paths', async () => {
    implementation = (name, args) => name === 'prepare_download_file' ? 'C:\\Downloads\\custom (1).zip' : defaultInvoke(name, args);
    const id = manager.addTask('https://example.com/file.zip', 'custom.zip'); await settle(id);
    expect(task(id).filename).toBe('custom (1).zip'); expect(calls('prepare_download_file')[0][1].fresh).toBe(true);
  });
  it('restarts from zero and clears a previous checksum result', async () => {
    const id = manager.addTask('https://example.com/file.zip', undefined, 1, 'a'.repeat(64)); await settle(id);
    manager.restartTask(id); await settle(id);
    expect(calls('probe_url')).toHaveLength(2);
    expect(calls('prepare_download_file')[1][1]).toMatchObject({ fresh: false, reset: true });
    expect(task(id).checksum?.verified).toBe(true);
  });
  it('exports and imports tasks without fabricated file sizes', () => {
    manager.pauseAll();
    const count = manager.importTasks('https://example.com/a.zip\nhttps://example.com/b.zip');
    expect(count).toBe(2); expect(JSON.parse(manager.exportTasksJson())).toHaveLength(2);
    expect(manager.getTasks().every((t: DownloadTask) => t.totalBytes === 0)).toBe(true);
  });
  it('resumes only when the remote validator and size still match', async () => {
    let finish!: () => void;
    const pending = new Promise<void>((resolve) => { finish = resolve; });
    let firstAttempt = '';
    implementation = async (name, args) => {
      if (name === 'download_segment' && !firstAttempt) {
        firstAttempt = args.taskId;
        events.get('download-progress')?.({ payload: { task_id: args.taskId, segment_index: 0, downloaded_bytes: 512, total_bytes: 2048, is_finished: false } });
        await pending; throw new Error('CANCELLED');
      }
      return defaultInvoke(name, args);
    };
    const id = manager.addTask('https://example.com/file.zip', undefined, 1);
    await vi.waitFor(() => expect(task(id).downloadedBytes).toBe(512));
    manager.pauseTask(id); manager.resumeTask(id); finish(); await settle(id);
    expect(calls('prepare_download_file')[1][1].reset).toBe(false);
    expect(calls('download_segment')[1][1]).toMatchObject({ currentDownloaded: 512, validator: '"v1"' });
  });
  it('validates and persists a user-selected download directory', async () => {
    implementation = (name, args) => name === 'validate_download_directory' ? args.directory : defaultInvoke(name, args);
    await manager.updateSettings({ defaultDirectory: 'C:\\ARGON Downloads' });
    const id = manager.addTask('https://example.com/file.zip'); await settle(id);
    expect(calls('prepare_download_file')[0][1].destinationPath).toBe('C:\\ARGON Downloads\\file.zip');
    expect(manager.getSettings().defaultDirectory).toBe('C:\\ARGON Downloads');
  });
  it('steals tail bytes from a slower active segment dynamically', async () => {
    let releaseSlow!: () => void;
    const slowGate = new Promise<void>((resolve) => { releaseSlow = resolve; });
    implementation = async (name, args) => {
      if (name === 'probe_url') return { content_length: 2_000_000, accept_ranges: true, etag: '"big"' };
      if (name === 'split_active_segment') {
        releaseSlow();
        return [1_500_000, 1_999_999];
      }
      if (name === 'download_segment') {
        if (args.segmentIndex === 1) {
          await slowGate;
          return { downloaded_bytes: 500_000, total_bytes: 2_000_000, end_byte: 1_499_999 };
        }
        return { downloaded_bytes: args.endByte - args.startByte + 1, total_bytes: 2_000_000, end_byte: args.endByte };
      }
      return defaultInvoke(name, args);
    };
    const id = manager.addTask('https://example.com/large.bin', undefined, 2);
    await settle(id);
    expect(task(id).status).toBe('completed');
    expect(task(id).downloadedBytes).toBe(2_000_000);
    expect(calls('split_active_segment')).toHaveLength(1);
    expect(task(id).chunks).toHaveLength(3);
  });
  it('applies per-task speed cap and theme preference persistence', async () => {
    await manager.updateSettings({ theme: 'light' });
    expect(manager.getSettings().theme).toBe('light');
    const id = manager.addTask('https://example.com/capped.zip');
    await manager.setTaskSpeedLimit(id, 1024 * 1024);
    await settle(id);
    expect(task(id).speedLimitBytesPerSec).toBe(1024 * 1024);
    expect(calls('set_task_speed_limit').some((c) => c[1].bytesPerSec === 1048576)).toBe(true);
  });
});
