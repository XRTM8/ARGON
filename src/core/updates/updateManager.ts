import { getStorage } from '../storage';
import { downloadManager } from '../downloadManager';
import { flowClient, nativeRuntime } from '../flowClient';
import { CURRENT_VERSION } from '../../components/Header';
import { isAndroidRuntime } from '../safeArea';
import { fetchReleaseInfo, isNewerVersion, type ReleaseAsset, type ReleaseInfo } from './releaseFeed';

export const UPDATE_PREFERENCES_KEY = 'argon_update_preferences_v1';
export const UPDATE_INTERVAL = 6 * 60 * 60 * 1000;
type Phase = 'idle' | 'checking' | 'available' | 'downloading' | 'ready' | 'current' | 'installing' | 'queued' | 'error';
type Mode = 'unknown' | 'installed' | 'portable' | 'unsupported' | 'preview';
export interface UpdateState {
  automatic: boolean; mode: Mode; phase: Phase; version?: string; notes?: string;
  downloaded: number; total?: number; error?: string; lastChecked?: number; taskId?: string;
}
interface BackendState { mode: Mode; phase: Phase; version?: string | null; notes?: string | null; downloaded: number; total?: number | null }
interface Dependencies {
  native: boolean; version: string; android: boolean;
  backend: (command: string) => Promise<BackendState | void>;
  listen: (callback: (progress: { downloaded: number; total?: number }) => void) => Promise<() => void>;
  storage: () => Storage | null; fetchRelease: () => Promise<ReleaseInfo>;
  queuePortable: (asset: ReleaseAsset) => string;
  busy: () => boolean; beforeInstall: () => void; installFailed: () => void;
}
const defaults: Dependencies = {
  native: nativeRuntime(), version: CURRENT_VERSION, android: isAndroidRuntime(),
  backend: async command => (await import('@tauri-apps/api/core')).invoke(command),
  listen: async callback => (await import('@tauri-apps/api/event')).listen('update-progress', event => callback(event.payload as { downloaded: number; total?: number })),
  storage: getStorage, fetchRelease: fetchReleaseInfo,
  queuePortable: asset => {
    const previous = downloadManager.getTasks().find(task => (task.originalUrl || task.url) === asset.url && task.status !== 'error');
    if (previous) {
      if (previous.status === 'paused') downloadManager.resumeTask(previous.id);
      return previous.id;
    }
    return downloadManager.addTask(asset.url, undefined, 4, asset.sha256);
  },
  busy: () => downloadManager.getStats().activeCount > 0 || !!flowClient.getState().snapshot?.transfers.some(transfer => ['queued', 'hashing', 'waiting', 'sending', 'receiving', 'verifying'].includes(transfer.status)) || !!flowClient.getState().snapshot?.incoming.length || !!flowClient.getState().snapshot?.pairings.length,
  beforeInstall: () => downloadManager.beginUpdateInstallation(),
  installFailed: () => downloadManager.endUpdateInstallation()
};

export function compatibleManualAsset(release: ReleaseInfo, android = false): ReleaseAsset | undefined {
  const asset = android ? release.assets.androidApk : release.assets.windowsPortable;
  if (!asset?.sha256 || !/^[a-f0-9]{64}$/i.test(asset.sha256)) return;
  const filename = android ? `ARGON_${release.version}_arm64.apk` : `argon-${release.version}-portable.zip`;
  return asset.url === `https://github.com/XRTM8/ARGON/releases/download/v${release.version}/${filename}` ? asset : undefined;
}

export class UpdateManager {
  private deps: Dependencies;
  private state: UpdateState;
  private subscribers = new Set<() => void>();
  private initialized?: Promise<void>;
  private checking?: Promise<UpdateState>;
  private downloading?: Promise<UpdateState>;
  private installing?: Promise<void>;
  private manualAsset?: ReleaseAsset;
  private references = 0;
  private interval?: ReturnType<typeof setInterval>;
  private startup?: ReturnType<typeof setTimeout>;
  private unlisten?: () => void;
  private listenerGeneration = 0;
  constructor(deps: Partial<Dependencies> = {}) {
    this.deps = { ...defaults, ...deps };
    let automatic = false;
    try { automatic = JSON.parse(this.deps.storage()?.getItem(UPDATE_PREFERENCES_KEY) || '{}').automatic === true; } catch { /* Default is opt-out. */ }
    this.state = { automatic, mode: this.deps.native ? 'unknown' : 'preview', phase: 'idle', downloaded: 0 };
  }
  getState = () => this.state;
  subscribe = (listener: () => void) => { this.subscribers.add(listener); return () => { this.subscribers.delete(listener); }; };
  private patch(value: Partial<UpdateState>) { this.state = { ...this.state, ...value }; this.subscribers.forEach(listener => listener()); }
  private accept(value: BackendState) {
    this.patch({ automatic: value.mode === 'installed' && this.state.automatic, mode: value.mode, phase: value.phase, version: value.version || undefined, notes: value.notes || undefined, downloaded: value.downloaded, total: value.total || undefined, error: undefined });
  }
  initialize() {
    return this.initialized ||= (async () => {
      if (!this.deps.native) return;
      try { const state = await this.deps.backend('update_status'); if (state) this.accept(state); }
      catch { this.patch({ mode: 'unsupported', error: 'updater_unavailable' }); }
    })();
  }
  start = () => {
    this.references++;
    if (this.references === 1) {
      void this.initialize();
      const generation = ++this.listenerGeneration;
      if (this.deps.native) void this.deps.listen(progress => { if (generation === this.listenerGeneration) this.patch({ downloaded: progress.downloaded, total: progress.total || undefined }); })
        .then(unlisten => { if (!this.references || generation !== this.listenerGeneration) unlisten(); else this.unlisten = unlisten; }).catch(() => {});
      this.startup = setTimeout(() => { if (this.state.automatic) void this.check(); }, 5000);
      this.interval = setInterval(() => { if (this.state.automatic) void this.check(); }, UPDATE_INTERVAL);
      if (typeof window !== 'undefined') window.addEventListener('online', this.online);
    }
    return () => {
      this.references = Math.max(0, this.references - 1);
      if (!this.references) {
        clearTimeout(this.startup); clearInterval(this.interval);
        this.listenerGeneration++;
        this.unlisten?.(); this.unlisten = undefined;
        if (typeof window !== 'undefined') window.removeEventListener('online', this.online);
      }
    };
  };
  private online = () => { if (this.state.automatic) void this.check(); };
  async setAutomatic(automatic: boolean) {
    await this.initialize();
    if (this.state.mode !== 'installed') return;
    try {
      const storage = this.deps.storage(); if (!storage) throw new Error('preferences_not_saved');
      storage.setItem(UPDATE_PREFERENCES_KEY, JSON.stringify({ automatic }));
      this.patch({ automatic, error: undefined });
    } catch { this.patch({ error: 'preferences_not_saved' }); return; }
    if (automatic) await this.check();
  }
  check = (): Promise<UpdateState> => {
    if (this.checking) return this.checking;
    if (this.downloading) return this.downloading;
    if (['ready', 'installing'].includes(this.state.phase)) return Promise.resolve(this.state);
    return this.checking = (async () => {
      await this.initialize();
      if (this.state.mode === 'preview') return this.state;
      this.patch({ phase: 'checking', error: undefined });
      try {
        if (this.state.mode === 'installed') {
          const result = await this.deps.backend('update_check'); if (result) this.accept(result);
        } else {
          const release = await this.deps.fetchRelease();
          if (isNewerVersion(release.version, this.deps.version)) {
            this.manualAsset = compatibleManualAsset(release, this.deps.android);
            if (this.deps.android && !release.assets.androidApk) throw new Error('android_apk_unavailable');
            if (!this.manualAsset || (this.state.mode === 'unsupported' && !this.deps.android)) throw new Error('incompatible_update_package');
            this.patch({ phase: 'available', version: release.version, notes: release.notes });
          } else this.patch({ phase: 'current', version: undefined });
        }
        this.patch({ lastChecked: Date.now() });
        if (this.state.automatic && this.state.mode === 'installed' && this.state.phase === 'available') await this.download();
      } catch (error) { this.patch({ phase: 'error', error: String(error) }); }
      return this.state;
    })().finally(() => { this.checking = undefined; });
  };
  download = (): Promise<UpdateState> => {
    if (this.downloading) return this.downloading;
    if (['ready', 'installing', 'queued'].includes(this.state.phase)) return Promise.resolve(this.state);
    return this.downloading = (async () => {
      this.patch({ error: undefined });
      try {
        if (this.state.mode === 'installed') {
          this.patch({ phase: 'downloading', downloaded: 0, total: undefined });
          const result = await this.deps.backend('update_download'); if (result) this.accept(result);
        } else {
          if (!this.manualAsset) throw new Error('incompatible_update_package');
          const taskId = this.deps.queuePortable(this.manualAsset);
          this.patch({ phase: 'queued', taskId });
        }
      } catch (error) { this.patch({ phase: this.state.version ? 'available' : 'error', error: String(error) }); }
      return this.state;
    })().finally(() => { this.downloading = undefined; });
  };
  install = (): Promise<void> => {
    if (this.installing) return this.installing;
    if (this.state.mode !== 'installed' || this.state.phase !== 'ready') return Promise.resolve();
    return this.installing = (async () => {
      let locked = false;
      try {
        if (this.deps.busy()) throw new Error('transfers_active');
        this.deps.beforeInstall(); locked = true; this.patch({ phase: 'installing', error: undefined });
        await this.deps.backend('update_install');
      } catch (error) {
        if (locked) this.deps.installFailed();
        const retryDownload = /cached_update_changed|download_update_first|signature|pubkey|verification/i.test(String(error));
        this.patch({ phase: retryDownload ? 'available' : 'ready', error: String(error) });
      }
    })().finally(() => { this.installing = undefined; });
  };
}
export const updateManager = new UpdateManager();
