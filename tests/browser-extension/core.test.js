import { describe, expect, it } from 'vitest';
import { DEFAULTS, excludedHost, mediaDescriptor, shouldCapture, parseUrls, platformVideo } from '../../browser-extension/core.js';

const download = { id: 1, url: 'https://files.example.com/file.zip', state: 'in_progress', danger: 'safe', totalBytes: 5000 };
const enabled = { ...DEFAULTS, autoIntercept: true };
describe('browser capture eligibility', () => {
  it('uses the platform page for supported videos rather than separate CDN audio/video tracks', () => {
    expect(platformVideo('https://www.youtube.com/watch?v=abc', 'My video')).toMatchObject({ type: 'YouTube', platform: true, filename: 'My video' });
    expect(platformVideo('https://youtube.com/@channel')).toBeNull();
    expect(platformVideo('https://notyoutube.com/watch?v=abc')).toBeNull();
    expect(mediaDescriptor('https://cdn.example.com/part.m4s', 'video/mp4')).toBeNull();
    expect(mediaDescriptor('https://cdn.example.com/segment.ts', 'video/mp2t')).toBeNull();
  });
  it('is opt-in and respects hostname boundaries and source-site exclusions', () => {
    expect(shouldCapture(download, DEFAULTS)).toBe(false);
    expect(excludedHost(download.url, ['example.com'])).toBe(true);
    expect(excludedHost('https://notexample.com/file', ['example.com'])).toBe(false);
    expect(shouldCapture({ ...download, referrer: 'https://private.example.org/page' }, { ...enabled, excludedHosts: ['example.org'] })).toBe(false);
  });
  it('leaves private, flagged, extension-created, local, finished and undersized transfers in the browser', () => {
    for (const changed of [{ incognito: true }, { danger: 'unwanted' }, { byExtensionId: 'other-extension' }, { url: 'blob:https://example.com/id' }, { state: 'complete' }]) expect(shouldCapture({ ...download, ...changed }, enabled)).toBe(false);
    expect(shouldCapture(download, { ...enabled, minSizeBytes: 10000 })).toBe(false);
    expect(shouldCapture({ ...download, totalBytes: -1 }, { ...enabled, minSizeBytes: 100 })).toBe(false);
    expect(shouldCapture(download, enabled)).toBe(true);
  });
  it('distinguishes direct media from unsupported playlists using MIME as well as extension', () => {
    expect(mediaDescriptor('https://cdn.example.com/videoplayback?id=123', 'video/mp4')).toMatchObject({ supported: true, type: 'MP4' });
    expect(mediaDescriptor('https://cdn.example.com/master', 'application/vnd.apple.mpegurl')).toMatchObject({ supported: false, type: 'HLS' });
    expect(shouldCapture({ ...download, url: 'https://example.com/video.mpd' }, enabled)).toBe(false);
    expect(mediaDescriptor('https://example.com/segment.ts')).toBeNull();
  });
  it('keeps signed URLs byte-for-byte and rejects unsupported batch entries', () => {
    const url = 'https://example.com/file?sig=a%2Fb&x=1';
    expect(parseUrls(`${url}\n${url}\nmagnet:?xt=x\nhttps://example.com/master.m3u8`)).toEqual({ urls: [url], invalid: ['magnet:?xt=x', 'https://example.com/master.m3u8'] });
  });
});
