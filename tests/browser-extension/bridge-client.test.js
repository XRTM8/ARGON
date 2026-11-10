import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest';
import { buildAuthHeaders, connection, sendDownload } from '../../browser-extension/bridge-client.js';

describe('browser-extension bridge-client', () => {
  const originalFetch = globalThis.fetch;
  const originalChrome = globalThis.chrome;

  beforeEach(() => {
    globalThis.fetch = vi.fn();
    globalThis.chrome = {
      cookies: {
        getAll: vi.fn().mockResolvedValue([
          { name: 'session', value: 'abc123' },
          { name: 'cf_clearance', value: 'xyz' }
        ])
      }
    };
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    globalThis.chrome = originalChrome;
    vi.restoreAllMocks();
  });

  it('checks persisted-handoff capability on connection()', async () => {
    globalThis.fetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ app: 'ARGON', status: 'ok', capabilities: ['persisted-handoff', 'ffmpeg-hls'] })
    });
    const info = await connection();
    expect(info.capabilities).toContain('ffmpeg-hls');

    globalThis.fetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ app: 'ARGON', status: 'ok', capabilities: [] })
    });
    await expect(connection()).rejects.toThrow('update_required');
  });

  it('forwards cookies only when opt-in enabled and non-incognito', async () => {
    const disabled = await buildAuthHeaders('https://cdn.example.com/a.zip', 'https://example.com/', { forwardCookies: false }, false);
    expect(disabled).toEqual({ Referer: 'https://example.com/' });
    expect(globalThis.chrome.cookies.getAll).not.toHaveBeenCalled();

    const incognito = await buildAuthHeaders('https://cdn.example.com/a.zip', 'https://example.com/', { forwardCookies: true }, true);
    expect(incognito).toEqual({ Referer: 'https://example.com/' });

    const enabled = await buildAuthHeaders('https://cdn.example.com/a.zip', 'https://example.com/', { forwardCookies: true }, false);
    expect(enabled).toEqual({
      Referer: 'https://example.com/',
      Cookie: 'session=abc123; cf_clearance=xyz'
    });
  });

  it('gates HLS/DASH playlists behind allowPlaylists and forwards POST/method metadata', async () => {
    await expect(sendDownload('https://cdn.example.com/live/master.m3u8', 'master.m3u8', 'https://example.com/'))
      .rejects.toThrow('unsupported_url');

    globalThis.fetch.mockResolvedValueOnce({
      ok: true,
      json: async () => ({ status: 'accepted', id: 'dl_1' })
    });

    const result = await sendDownload(
      'https://cdn.example.com/live/master.m3u8',
      'master.mp4',
      'https://example.com/',
      { Cookie: 'session=abc123' },
      'POST',
      'token=123',
      true
    );

    expect(result.status).toBe('accepted');
    const [, options] = globalThis.fetch.mock.calls[0];
    expect(JSON.parse(options.body)).toEqual({
      url: 'https://cdn.example.com/live/master.m3u8',
      filename: 'master.mp4',
      headers: { Referer: 'https://example.com/', Cookie: 'session=abc123' },
      method: 'POST',
      postBody: 'token=123'
    });
  });
});
