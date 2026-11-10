import { describe, it, expect, vi, afterEach } from 'vitest';
import { isNewerVersion, fetchReleaseInfo } from './releaseFeed';

describe('ARGON Update Engine', () => {
  it('correctly compares semantic versions', () => {
    expect(isNewerVersion('1.0.1', '1.0.0')).toBe(true);
    expect(isNewerVersion('1.1.0', '1.0.9')).toBe(true);
    expect(isNewerVersion('2.0.0', '1.9.9')).toBe(true);
    expect(isNewerVersion('v1.0.5', '1.0.4')).toBe(true);

    expect(isNewerVersion('1.0.0', '1.0.0')).toBe(false);
    expect(isNewerVersion('1.0.0', '1.0.1')).toBe(false);
    expect(isNewerVersion('0.9.9', '1.0.0')).toBe(false);
  });
});

describe('published release discovery', () => {
  afterEach(() => vi.unstubAllGlobals());
  it('does not mistake a portable EXE for an installer', async () => {
    const fetch = vi.fn().mockResolvedValueOnce({ ok: false }).mockResolvedValueOnce({ ok: true, json: async () => ({
      tag_name: 'v1.5.2', assets: [
        { name: 'ARGON_1.5.2_x64-setup.exe', browser_download_url: 'https://github.com/XRTM8/ARGON/releases/download/v1.5.2/ARGON_1.5.2_x64-setup.exe' },
        { name: 'argon-1.5.2-portable.exe', browser_download_url: 'https://github.com/XRTM8/ARGON/releases/download/v1.5.2/argon-1.5.2-portable.exe' },
      ],
    }) });
    vi.stubGlobal('fetch', fetch);
    const release = await fetchReleaseInfo();
    expect(release.assets.windowsInstaller?.url).toContain('-setup.exe');
    expect(release.assets.windowsPortable?.url).toContain('portable.exe');
  });
  it('rejects release pages and untrusted assets as download targets', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => ({ ok: true, json: async () => ({ version: '1.5.2', tag_name: 'v1.5.2', assets: [{ kind: 'windows-installer', url: 'https://github.com/XRTM8/ARGON/releases/latest' }] }) })));
    await expect(fetchReleaseInfo()).rejects.toThrow('قابل للتنزيل');
  });
  it('honors an already-aborted request', async () => {
    const fetch = vi.fn(); vi.stubGlobal('fetch', fetch);
    const controller = new AbortController(); controller.abort();
    await expect(fetchReleaseInfo(undefined, controller.signal)).rejects.toThrow('cancelled');
    expect(fetch).not.toHaveBeenCalled();
  });
  it('extracts sha256 checksums from GitHub REST API digest fields', async () => {
    const hash = 'b'.repeat(64);
    const fetch = vi.fn().mockResolvedValueOnce({ ok: false }).mockResolvedValueOnce({
      ok: true,
      json: async () => ({
        tag_name: 'v2.3.0',
        assets: [
          {
            name: 'argon-2.3.0-portable.zip',
            browser_download_url: 'https://github.com/XRTM8/ARGON/releases/download/v2.3.0/argon-2.3.0-portable.zip',
            digest: `sha256:${hash}`,
          },
          {
            name: 'ARGON_2.3.0_arm64.apk',
            browser_download_url: 'https://github.com/XRTM8/ARGON/releases/download/v2.3.0/ARGON_2.3.0_arm64.apk',
            digest: `SHA256:${'C'.repeat(64)}`,
          },
        ],
      }),
    });
    vi.stubGlobal('fetch', fetch);
    const release = await fetchReleaseInfo();
    expect(release.assets.windowsPortable?.sha256).toBe(hash);
    expect(release.assets.androidApk?.sha256).toBe('c'.repeat(64));
  });
  it('requires complete stable versions', () => {
    expect(isNewerVersion('1.6.0-beta', '1.5.1')).toBe(false);
    expect(isNewerVersion('nonsense', '1.5.1')).toBe(false);
  });
});
