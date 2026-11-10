export interface ReleaseAsset {
  url: string;
  size?: number;
  sha256?: string;
}

export interface ReleaseAssets {
  windowsInstaller?: ReleaseAsset;
  windowsPortable?: ReleaseAsset;
  androidApk?: ReleaseAsset;
}

export interface ReleaseInfo {
  version: string;
  notes: string;
  pubDate: string;
  assets: ReleaseAssets;
}

export const ARGON_RELEASES_URL = 'https://github.com/XRTM8/ARGON/releases/latest';
export const ARGON_RELEASE_FEED_URL = `${ARGON_RELEASES_URL}/download/latest.json`;
export const GITHUB_API_URL = 'https://api.github.com/repos/XRTM8/ARGON/releases/latest';

function normalizeVersion(version: string): string { return version.trim().replace(/^v/i, ''); }

export function isNewerVersion(candidate: string, current: string): boolean {
  const a = normalizeVersion(candidate), b = normalizeVersion(current);
  if (!/^\d+\.\d+\.\d+$/.test(a) || !/^\d+\.\d+\.\d+$/.test(b)) return false;
  const left = a.split('.').map(Number), right = b.split('.').map(Number);
  for (let i = 0; i < 3; i++) { if (left[i] !== right[i]) return left[i] > right[i]; }
  return false;
}

function parseAssetSha256(raw: any): string | undefined {
  if (typeof raw?.sha256 === 'string' && raw.sha256.length > 0) {
    return /^[a-f0-9]{64}$/i.test(raw.sha256) ? raw.sha256.toLowerCase() : undefined;
  }
  if (typeof raw?.digest === 'string') {
    const match = /^sha256:([a-f0-9]{64})$/i.exec(raw.digest.trim());
    if (match) return match[1].toLowerCase();
  }
  return undefined;
}

function asset(raw: any): ReleaseAsset | undefined {
  const url = raw?.url || raw?.browser_download_url;
  if (typeof url !== 'string') return;
  try {
    const parsed = new URL(url);
    if (parsed.protocol !== 'https:' || parsed.hostname !== 'github.com' || !parsed.pathname.startsWith('/XRTM8/ARGON/releases/download/')) return;
    if (raw.sha256 && !/^[a-f0-9]{64}$/i.test(raw.sha256)) return;
    return { url, size: raw.size, sha256: parseAssetSha256(raw) };
  } catch { return; }
}

function release(data: any, github: boolean): ReleaseInfo {
  const version = normalizeVersion(String(github ? data.tag_name || '' : data.version || ''));
  if (!/^\d+\.\d+\.\d+$/.test(version)) throw new Error('Invalid release version');
  const assets: ReleaseAssets = {};
  if (Array.isArray(data.assets)) {
    for (const raw of data.assets) {
      const value = asset(github ? { ...raw, url: raw.browser_download_url } : raw);
      if (!value) continue;
      const name = String(raw.name || '').toLowerCase();
      if (raw.kind === 'windows-portable' || name.includes('portable')) assets.windowsPortable = value;
      else if (raw.kind === 'windows-installer' || /(-setup\.exe|\.msi)$/.test(name)) assets.windowsInstaller = value;
      else if (raw.kind === 'android-apk' || name.endsWith('.apk')) assets.androidApk = value;
    }
  } else if (data.assets && typeof data.assets === 'object') {
    for (const key of ['windowsInstaller', 'windowsPortable', 'androidApk'] as const) assets[key] = asset(data.assets[key]);
  }
  if (!Object.values(assets).some(Boolean)) throw new Error('No published download assets in this release');
  return { version, notes: String(github ? data.body || '' : data.notes || ''), pubDate: String(data.published_at || data.pub_date || ''), assets };
}

export async function fetchReleaseInfo(url: string = ARGON_RELEASE_FEED_URL, signal?: AbortSignal): Promise<ReleaseInfo> {
  if (signal?.aborted) throw new Error('Update check cancelled');
  const hasTauriInvoke =
    typeof window !== 'undefined' &&
    '__TAURI_INTERNALS__' in window &&
    typeof (window as { __TAURI_INTERNALS__?: { invoke?: unknown } }).__TAURI_INTERNALS__?.invoke === 'function';
  if (hasTauriInvoke && url === ARGON_RELEASE_FEED_URL) {
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const nativeFeed = await invoke<{ github: boolean; payload: unknown }>('fetch_release_feed');
      if (signal?.aborted) throw new Error('Update check cancelled');
      return release(nativeFeed.payload, !!nativeFeed.github);
    } catch (error) {
      if (signal?.aborted || (error instanceof Error && error.message === 'Update check cancelled')) {
        throw new Error('Update check cancelled');
      }
      /* Fall back to direct WebView fetch below if native helper failed. */
    }
  }
  for (const [endpoint, github] of [[url, false], [GITHUB_API_URL, true]] as const) {
    if (signal?.aborted) throw new Error('Update check cancelled');
    const controller = new AbortController();
    const timer = globalThis.setTimeout(() => controller.abort(), 12000);
    const abort = () => controller.abort();
    signal?.addEventListener('abort', abort, { once: true });
    try {
      const response = await fetch(endpoint, { cache: 'no-store', headers: { Accept: 'application/json' }, signal: controller.signal });
      if (response.ok) return release(await response.json(), github);
    } catch { /* Each fallback receives its own timeout. */ }
    finally { globalThis.clearTimeout(timer); signal?.removeEventListener('abort', abort); }
  }
  throw new Error('تعذر الوصول إلى إصدار منشور قابل للتنزيل؛ تحقق من الاتصال وحاول مجددًا.');
}
