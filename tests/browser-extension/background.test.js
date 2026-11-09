import { beforeEach, afterEach, describe, expect, it, vi } from 'vitest';
let listeners, local, session, item, chromeMock, fetchMock;
const event = (name) => ({ addListener: (callback) => { listeners[name] = callback; } });
function area(memory) {
  return {
    get: vi.fn(async (keys) => keys === null ? { ...memory } : typeof keys === 'string' ? { [keys]: memory[keys] } : { ...keys, ...memory }),
    set: vi.fn(async (data) => { Object.assign(memory, data); }),
    remove: vi.fn(async (key) => { delete memory[key]; })
  };
}
const response = (data, ok = true) => ({ ok, status: ok ? 200 : 503, json: async () => data });
beforeEach(() => {
  vi.resetModules(); listeners = {}; local = { autoIntercept: true }; session = {};
  item = { id: 42, url: 'https://example.com/file.zip', filename: 'C:\\Downloads\\file.zip', state: 'in_progress', danger: 'safe', startTime: '2026-11-08T12:00:00Z', paused: false };
  chromeMock = {
    runtime: { id: 'test-extension', onInstalled: event('installed'), onMessage: event('message') },
    storage: { local: area(local), session: area(session) },
    contextMenus: { onClicked: event('context'), removeAll: vi.fn(), create: vi.fn() },
    action: { setBadgeText: vi.fn(async () => {}), setBadgeBackgroundColor: vi.fn(async () => {}) },
    notifications: { create: vi.fn(async () => {}) },
    downloads: { onCreated: event('created'), search: vi.fn(async () => [item]), pause: vi.fn(async () => { item.paused = true; }), resume: vi.fn(async () => { item.paused = false; }), cancel: vi.fn(async () => { item.state = 'interrupted'; }) },
    webRequest: { onHeadersReceived: event('headers') }, tabs: { onRemoved: event('removed'), onUpdated: event('updated') }
  };
  fetchMock = vi.fn(async (url) => response(url.endsWith('/ping') ? { status: 'ok', app: 'ARGON', capabilities: ['persisted-handoff'] } : { status: 'accepted' }));
  vi.stubGlobal('chrome', chromeMock); vi.stubGlobal('fetch', fetchMock);
});
afterEach(() => { vi.unstubAllGlobals(); });

describe('confirmed browser handoff', () => {
  it('uses the platform page rather than a silent CDN track for the video button', async () => {
    await import('../../browser-extension/background.js');
    const respond = vi.fn();
    listeners.message({ type: 'DOWNLOAD_VIDEO', source: 'https://cdn.example.com/silent.mp4' }, { id: 'test-extension', tab: { id: 7, url: 'https://www.youtube.com/watch?v=example123', title: 'Example' } }, respond);
    await vi.waitFor(() => expect(respond).toHaveBeenCalledWith({ ok: true }));
    const request = fetchMock.mock.calls.find(([url]) => url.endsWith('/add'));
    expect(JSON.parse(request[1].body).url).toBe('https://www.youtube.com/watch?v=example123');
  });
  it('suppresses the video button on excluded sites, in incognito and when disabled', async () => {
    await import('../../browser-extension/background.js');
    const request = { type: 'GET_VIDEO_TARGET', source: 'https://cdn.example.com/film.mp4' };
    const sender = { id: 'test-extension', tab: { id: 7, url: 'https://example.com/watch' } };
    local.excludedHosts = ['example.com'];
    const excluded = vi.fn(); listeners.message(request, sender, excluded);
    await vi.waitFor(() => expect(excluded).toHaveBeenCalledWith({ ok: false }));
    local.excludedHosts = []; local.showVideoButton = false;
    const disabled = vi.fn(); listeners.message(request, sender, disabled);
    await vi.waitFor(() => expect(disabled).toHaveBeenCalledWith({ ok: false }));
    const privateReply = vi.fn();
    expect(listeners.message(request, { ...sender, tab: { ...sender.tab, incognito: true } }, privateReply)).toBe(false);
    expect(privateReply).not.toHaveBeenCalled();
    expect(fetchMock).not.toHaveBeenCalled();
  });
  it('pauses before dispatch, then cancels only after ARGON acknowledges a saved task', async () => {
    const order = [];
    chromeMock.downloads.pause.mockImplementation(async () => { order.push('pause'); });
    chromeMock.downloads.cancel.mockImplementation(async () => { order.push('cancel'); });
    fetchMock.mockImplementation(async (url) => { if (url.endsWith('/add')) { order.push('saved'); return response({ status: 'accepted' }); } return response({ status: 'ok', app: 'ARGON', capabilities: ['persisted-handoff'] }); });
    await import('../../browser-extension/background.js'); listeners.created({ ...item });
    await vi.waitFor(() => expect(chromeMock.downloads.cancel).toHaveBeenCalledWith(42));
    expect(order).toEqual(['pause', 'saved', 'cancel']); expect(chromeMock.downloads.resume).not.toHaveBeenCalled();
  });
  it('resumes the original browser download if ARGON rejects the handoff', async () => {
    fetchMock.mockImplementation(async (url) => url.endsWith('/add') ? response({ error: 'handoff_not_saved' }, false) : response({ status: 'ok', app: 'ARGON', capabilities: ['persisted-handoff'] }));
    await import('../../browser-extension/background.js'); listeners.created({ ...item });
    await vi.waitFor(() => expect(chromeMock.downloads.resume).toHaveBeenCalledWith(42));
    expect(chromeMock.downloads.cancel).not.toHaveBeenCalled(); expect(local['handoff:42']).toBeUndefined();
  });
  it('leaves browser downloads alone when the app is offline or uses an older protocol', async () => {
    fetchMock.mockResolvedValue(response({ status: 'ok', app: 'ARGON' }));
    await import('../../browser-extension/background.js'); listeners.created({ ...item });
    await vi.waitFor(() => expect(fetchMock).toHaveBeenCalled());
    expect(chromeMock.downloads.pause).not.toHaveBeenCalled(); expect(chromeMock.downloads.cancel).not.toHaveBeenCalled();
  });
  it('recovers a paused, unconfirmed transfer when the service worker restarts', async () => {
    local['handoff:42'] = { id: 42, startTime: item.startTime, saved: false }; item.paused = true;
    await import('../../browser-extension/background.js');
    await vi.waitFor(() => expect(chromeMock.downloads.resume).toHaveBeenCalledWith(42));
    expect(chromeMock.downloads.cancel).not.toHaveBeenCalled();
  });
  it('retains detected media across service-worker suspension', async () => {
    await import('../../browser-extension/background.js');
    listeners.headers({ tabId: 7, statusCode: 200, url: 'https://cdn.example.com/playback?id=42', responseHeaders: [{ name: 'Content-Type', value: 'video/mp4' }] });
    await vi.waitFor(() => expect(session['media:7']).toHaveLength(1));
    vi.resetModules(); await import('../../browser-extension/background.js');
    const respond = vi.fn(); listeners.message({ type: 'GET_TAB_MEDIA', tabId: 7 }, { id: 'test-extension' }, respond);
    await vi.waitFor(() => expect(respond).toHaveBeenCalledWith({ media: [expect.objectContaining({ type: 'MP4', supported: true })] }));
  });
});
