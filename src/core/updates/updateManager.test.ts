import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { UpdateManager, UPDATE_INTERVAL, UPDATE_PREFERENCES_KEY, compatibleManualAsset } from './updateManager';

let memory: Map<string, string>;
const status = (phase: string, mode = 'installed') => ({ mode, phase, version: phase === 'idle' || phase === 'current' ? undefined : '2.3.0', downloaded: phase === 'ready' ? 100 : 0, total: phase === 'ready' ? 100 : undefined });
function setup(mode = 'installed') {
  const backend = vi.fn(async (command: string) => status(command === 'update_check' ? 'available' : command === 'update_download' ? 'ready' : 'idle', mode) as any);
  const storage = { getItem: (key: string) => memory.get(key) || null, setItem: (key: string, value: string) => { memory.set(key, value); } } as Storage;
  const fetchRelease = vi.fn();
  const queuePortable = vi.fn(() => 'task-update');
  const beforeInstall = vi.fn(); const installFailed = vi.fn(); const busy = vi.fn(() => false);
  const manager = new UpdateManager({ native: true, version: '2.2.0', android: false, backend, storage: () => storage, fetchRelease, queuePortable, busy, beforeInstall, installFailed, listen: async () => () => {} });
  return { manager, backend, storage, fetchRelease, queuePortable, beforeInstall, installFailed, busy };
}
beforeEach(() => { memory = new Map(); });
afterEach(() => { vi.useRealTimers(); });

describe('optional updater coordination', () => {
  it('defaults to off and does no network update checks at startup or on its timer', async () => {
    vi.useFakeTimers(); const { manager, backend } = setup(); const stop = manager.start();
    await vi.advanceTimersByTimeAsync(UPDATE_INTERVAL + 5000);
    expect(manager.getState().automatic).toBe(false);
    expect(backend.mock.calls.map(call => call[0])).toEqual(['update_status']); stop();
  });
  it('remembers opt-in and downloads a signed package once without installing it', async () => {
    const { manager, backend } = setup();
    await manager.setAutomatic(true);
    expect(JSON.parse(memory.get(UPDATE_PREFERENCES_KEY)!).automatic).toBe(true);
    expect(manager.getState().phase).toBe('ready');
    await manager.check(); await manager.download();
    expect(backend.mock.calls.filter(call => call[0] === 'update_check')).toHaveLength(1);
    expect(backend.mock.calls.filter(call => call[0] === 'update_download')).toHaveLength(1);
    expect(backend.mock.calls.filter(call => call[0] === 'update_install')).toHaveLength(0);
  });
  it('coalesces checks when settings and startup request the same operation', async () => {
    const { manager, backend } = setup();
    let resolve!: () => void; const waiting = new Promise<void>(done => { resolve = done; });
    backend.mockImplementation(async command => { if (command === 'update_check') await waiting; return status(command === 'update_check' ? 'available' : 'idle') as any; });
    const first = manager.check(), second = manager.check(); expect(first).toBe(second); resolve(); await first;
    expect(backend.mock.calls.filter(call => call[0] === 'update_check')).toHaveLength(1);
  });
  it('does not report opt-in as saved if local storage fails', async () => {
    const { manager, storage, backend } = setup(); vi.spyOn(storage, 'setItem').mockImplementation(() => { throw new Error('quota'); });
    await manager.setAutomatic(true); expect(manager.getState().automatic).toBe(false); expect(manager.getState().error).toBe('preferences_not_saved');
    expect(backend.mock.calls.filter(call => call[0] === 'update_check')).toHaveLength(0);
  });
  it('blocks installation during transfers and restores the queue if installation fails', async () => {
    const { manager, backend, busy, beforeInstall, installFailed } = setup();
    await manager.check(); await manager.download(); busy.mockReturnValue(true); await manager.install();
    expect(beforeInstall).not.toHaveBeenCalled(); expect(manager.getState().error).toContain('transfers_active');
    busy.mockReturnValue(false); backend.mockRejectedValueOnce(new Error('installer failed')); await manager.install();
    expect(beforeInstall).toHaveBeenCalledOnce(); expect(installFailed).toHaveBeenCalledOnce(); expect(manager.getState().phase).toBe('ready');
  });
  it('allows redownloading a cached package changed after download', async () => {
    const { manager, backend } = setup(); await manager.check(); await manager.download();
    backend.mockRejectedValueOnce(new Error('cached_update_changed')); await manager.install();
    expect(manager.getState().phase).toBe('available'); await manager.download();
    expect(backend.mock.calls.filter(call => call[0] === 'update_download')).toHaveLength(2);
  });
  it('never installs or automatically downloads an installer into a Portable copy', async () => {
    memory.set(UPDATE_PREFERENCES_KEY, '{"automatic":true}');
    const { manager, backend, fetchRelease, queuePortable } = setup('portable');
    const asset = { url: 'https://github.com/XRTM8/ARGON/releases/download/v2.3.0/argon-2.3.0-portable.zip', sha256: 'a'.repeat(64) };
    fetchRelease.mockResolvedValue({ version: '2.3.0', notes: '', pubDate: '', assets: { windowsPortable: asset } });
    await manager.check(); expect(manager.getState().automatic).toBe(false); expect(queuePortable).not.toHaveBeenCalled();
    await manager.download(); await manager.download(); await manager.install();
    expect(queuePortable).toHaveBeenCalledOnce(); expect(queuePortable).toHaveBeenCalledWith(asset);
    expect(backend.mock.calls.map(call => call[0])).toEqual(['update_status']);
  });
  it('rejects swapped package types and release-version mismatches', () => {
    const release = { version: '2.3.0', notes: '', pubDate: '', assets: { windowsPortable: { url: 'https://github.com/XRTM8/ARGON/releases/download/v2.3.0/ARGON_2.3.0_x64-setup.exe', sha256: 'a'.repeat(64) } } };
    expect(compatibleManualAsset(release)).toBeUndefined();
    release.assets.windowsPortable.url = 'https://github.com/XRTM8/ARGON/releases/download/v2.2.0/argon-2.3.0-portable.zip';
    expect(compatibleManualAsset(release)).toBeUndefined();
  });
  it('reports android_apk_unavailable when a GitHub release omits an Android APK', async () => {
    const storage = { getItem: (key: string) => memory.get(key) || null, setItem: (key: string, value: string) => { memory.set(key, value); } } as Storage;
    const manager = new UpdateManager({
      native: true,
      version: '2.2.0',
      android: true,
      backend: async () => status('idle', 'unsupported') as any,
      storage: () => storage,
      fetchRelease: async () => ({
        version: '2.3.0',
        notes: '',
        pubDate: '',
        assets: { windowsPortable: { url: 'https://github.com/XRTM8/ARGON/releases/download/v2.3.0/argon-2.3.0-portable.zip', sha256: 'a'.repeat(64) } },
      }),
      listen: async () => () => {},
    });
    const result = await manager.check();
    expect(result.phase).toBe('error');
    expect(result.error).toContain('android_apk_unavailable');
  });
  it('accepts valid Android APK releases and queues APK download on Android', async () => {
    const storage = { getItem: (key: string) => memory.get(key) || null, setItem: (key: string, value: string) => { memory.set(key, value); } } as Storage;
    const apk = { url: 'https://github.com/XRTM8/ARGON/releases/download/v2.3.0/ARGON_2.3.0_arm64.apk', sha256: 'd'.repeat(64) };
    const queuePortable = vi.fn(() => 'apk-task-1');
    const manager = new UpdateManager({
      native: true,
      version: '2.2.0',
      android: true,
      backend: async () => status('idle', 'unsupported') as any,
      storage: () => storage,
      fetchRelease: async () => ({ version: '2.3.0', notes: '', pubDate: '', assets: { androidApk: apk } }),
      queuePortable,
      listen: async () => () => {},
    });
    const checked = await manager.check();
    expect(checked.phase).toBe('available');
    const downloaded = await manager.download();
    expect(downloaded.phase).toBe('queued');
    expect(downloaded.taskId).toBe('apk-task-1');
    expect(queuePortable).toHaveBeenCalledWith(apk);
  });
});
