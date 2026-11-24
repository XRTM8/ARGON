import { DEFAULTS, httpUrl, isMagnetUrl, shouldCapture, mediaDescriptor, platformVideo, excludedHost } from './core.js';
import { browserHandoff, buildAuthHeaders, connection, sendDownload } from './bridge-client.js';

// Session storage survives MV3 worker suspension, but is cleared with the browser session.
const writes = new Map();
const handoffKey = (id) => `handoff:${id}`;
const handoffAlarm = 'argon-recover-handoffs';
const activeHandoffs = new Set();

function retryRecovery() {
  chrome.alarms?.create(handoffAlarm, { delayInMinutes: 1 });
}

async function reconcileHandoff(key, record) {
  const [download] = await chrome.downloads.search({ id: record.id });
  const item = download?.startTime === record.startTime ? download : undefined;
  const inProgress = item?.state === 'in_progress';
  if (record.phase === 'committing') {
    let { status } = await browserHandoff('status', record.requestId);
    if (status === 'prepared') {
      ({ status } = await browserHandoff(inProgress && item.paused ? 'commit' : 'abort', record.requestId));
    }
    if (status === 'prepared') throw new Error('handoff_still_pending');
    if (status === 'accepted' && inProgress) {
      await chrome.downloads.cancel(item.id);
      await notify(['حُفظ التنزيل في ARGON.', 'Download saved in ARGON.']);
    }
    if (status === 'missing' && inProgress && item.paused) await chrome.downloads.resume(item.id);
  } else if (record.requestId) {
    // A prepare-only request cannot start a native transfer. Restore the browser first.
    if (inProgress && item.paused) await chrome.downloads.resume(item.id);
    await browserHandoff('abort', record.requestId);
  } else if (inProgress) {
    if (record.saved) await chrome.downloads.cancel(item.id);
    else if (item.paused) await chrome.downloads.resume(item.id);
  }
  await chrome.storage.local.remove(key);
}

async function recoverHandoffs() {
  const data = await chrome.storage.local.get(null);
  for (const [key, record] of Object.entries(data)) {
    if (!key.startsWith('handoff:') || !record?.id || activeHandoffs.has(key)) continue;
    activeHandoffs.add(key);
    try {
      await reconcileHandoff(key, record);
    } catch {
      retryRecovery();
    } finally {
      activeHandoffs.delete(key);
    }
  }
}

const recovered = recoverHandoffs().catch(retryRecovery);
chrome.alarms?.onAlarm.addListener((alarm) => {
  if (alarm.name === handoffAlarm) void recoverHandoffs().catch(retryRecovery);
});
const keyFor = (id) => `media:${id}`;
function updateTab(id, operation) {
  const previous = writes.get(id) || Promise.resolve();
  const next = previous.catch(() => {}).then(async () => {
    const key = keyFor(id);
    const data = await chrome.storage.session.get(key);
    const list = operation(data[key] || []);
    if (list === null) await chrome.storage.session.remove(key);
    else await chrome.storage.session.set({ [key]: list });
    const count = list?.filter((item) => item.supported || item.playlist).length || 0;
    await chrome.action.setBadgeText({ tabId: id, text: count ? String(count) : '' }).catch(() => {});
    await chrome.action.setBadgeBackgroundColor({ tabId: id, color: '#2563EB' }).catch(() => {});
    if (typeof chrome.tabs.sendMessage === 'function') void chrome.tabs.sendMessage(id, { type: 'ARGON_MEDIA_CHANGED' }).catch(() => {});
  });
  writes.set(id, next);
  void next.catch(() => {}).finally(() => { if (writes.get(id) === next) writes.delete(id); });
  return next;
}

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.removeAll(() => {
    for (const [id, title, contexts] of [
      ['argon-link', 'Download with ARGON / تنزيل بواسطة ARGON', ['link']],
      ['argon-media', 'Download media with ARGON / تنزيل الوسائط', ['image', 'video', 'audio']],
      ['argon-selection', 'Send selected URL to ARGON / إرسال الرابط المحدد', ['selection']]
    ]) chrome.contextMenus.create({ id, title, contexts });
  });
});

async function notify(message) {
  const { language } = await chrome.storage.local.get(DEFAULTS);
  await chrome.notifications.create({ type: 'basic', iconUrl: 'icons/icon48.png', title: 'ARGON', message: language === 'ar' ? message[0] : message[1] }).catch(() => {});
}

chrome.contextMenus.onClicked.addListener((info, tab) => {
  const url = info.menuItemId === 'argon-link' ? info.linkUrl : info.menuItemId === 'argon-media' ? info.srcUrl : info.selectionText?.trim();
  if (!httpUrl(url) && !isMagnetUrl(url)) { void notify(['اختر رابط تنزيل مباشر أو رابط Magnet صالح.', 'Choose a direct HTTP/HTTPS URL or valid Magnet link.']); return; }
  void (async () => {
    const settings = await chrome.storage.local.get(DEFAULTS);
    const ping = await connection();
    const allowPlaylists = !!ping.capabilities?.includes('ffmpeg-hls');
    const headers = isMagnetUrl(url) ? undefined : await buildAuthHeaders(url, info.frameUrl || tab?.url, settings, !!tab?.incognito);
    await sendDownload(url, undefined, info.frameUrl || tab?.url, headers, undefined, undefined, allowPlaylists);
  })().then(
    () => notify(['تمت إضافة الرابط إلى ARGON.', 'Link added to ARGON.']),
    () => notify(['تعذّر تسليم الرابط. افتح ARGON وتحقق من دعم نوع الرابط.', 'Could not hand off this link. Open ARGON and check that the URL type is supported.'])
  );
});

chrome.downloads.onCreated.addListener((item) => { void capture(item).catch(() => {}); });

async function capture(item) {
  if (item.state !== 'in_progress' || item.paused || item.incognito || item.byExtensionId) return;
  await recovered;
  const settings = await chrome.storage.local.get(DEFAULTS);
  if (!shouldCapture(item, settings)) return;
  let ping;
  try { ping = await connection(); } catch { return; }
  const [current] = await chrome.downloads.search({ id: item.id });
  if (!current || !shouldCapture(current, settings)) return;
  const key = handoffKey(item.id);
  if (activeHandoffs.has(key)) return;
  activeHandoffs.add(key);
  try {
    if (ping.capabilities?.includes('staged-handoff')) {
      await captureStaged(key, current, settings, ping);
    } else {
      await captureLegacy(key, current, settings, ping);
    }
  } finally {
    activeHandoffs.delete(key);
  }
}

async function captureStaged(key, item, settings, ping) {
  const requestId = crypto.randomUUID();
  const record = { id: item.id, startTime: item.startTime, requestId, phase: 'preparing' };
  await chrome.storage.local.set({ [key]: record });
  try {
    await chrome.downloads.pause(item.id);
    const url = item.finalUrl || item.url;
    const headers = await buildAuthHeaders(url, item.referrer, settings);
    const filename = item.filename?.split(/[\\/]+/).pop();
    try {
      await sendDownload(url, filename, item.referrer, headers, undefined, undefined, !!ping.capabilities?.includes('ffmpeg-hls'), { requestId });
    } catch (error) {
      const { status } = await browserHandoff('status', requestId);
      if (status === 'missing') throw error;
    }
    await chrome.storage.local.set({ [key]: { ...record, phase: 'committing' } });
    record.phase = 'committing';
    await reconcileHandoff(key, record);
  } catch {
    try {
      await reconcileHandoff(key, record);
    } catch {
      retryRecovery();
      if (record.phase === 'committing') await notify([
        'توقف تنزيل المتصفح مؤقتاً بانتظار التحقق من تسليمه إلى ARGON.',
        'The browser download is paused while ARGON verifies the handoff.'
      ]);
    }
  }
}

async function captureLegacy(key, current, settings, ping) {
  const id = current.id;
  await chrome.storage.local.set({ [key]: { id, startTime: current.startTime, saved: false } });
  // Pause, then send: prevents small files completing while ARGON takes over.
  try { await chrome.downloads.pause(id); } catch { await chrome.storage.local.remove(key); return; }
  try {
    const targetUrl = current.finalUrl || current.url;
    const headers = await buildAuthHeaders(targetUrl, current.referrer, settings, !!current.incognito);
    await sendDownload(
      targetUrl,
      current.filename?.split(/[\\/]/).pop(),
      current.referrer,
      headers,
      undefined,
      undefined,
      !!ping.capabilities?.includes('ffmpeg-hls')
    );
  } catch {
    try { await chrome.downloads.resume(id); await chrome.storage.local.remove(key); } catch { retryRecovery(); }
    return;
  }
  // Only an acknowledgement of a saved task permits cancellation. Keep browser history.
  try {
    await chrome.storage.local.set({ [key]: { id, startTime: current.startTime, saved: true } });
    await chrome.downloads.cancel(id);
    await chrome.storage.local.remove(key);
    await notify(['حُفظ التنزيل في ARGON.', 'Download saved in ARGON.']);
  } catch {
    retryRecovery();
    await notify(['حُفظ التنزيل في ARGON؛ تحقق من تنزيل المتصفح أيضاً.', 'Saved in ARGON; also check the browser download.']);
  }
}

chrome.webRequest.onHeadersReceived.addListener((details) => {
  if (details.tabId < 0 || details.incognito || details.statusCode < 200 || details.statusCode >= 400) return;
  const mime = details.responseHeaders?.find((header) => header.name.toLowerCase() === 'content-type')?.value || '';
  const descriptor = mediaDescriptor(details.url, mime);
  if (!descriptor) return;
  void updateTab(details.tabId, (previous) => {
    const list = previous.filter((item) => Date.now() - item.timestamp < 30 * 60 * 1000 && item.url !== descriptor.url);
    return [...list, { ...descriptor, timestamp: Date.now() }].slice(-40);
  }).catch(() => {});
}, { urls: ['http://*/*', 'https://*/*'] }, ['responseHeaders']);

chrome.tabs.onRemoved.addListener((id) => { void updateTab(id, () => null).catch(() => {}); });
chrome.tabs.onUpdated.addListener((id, change) => {
  if (change.status === 'loading') void updateTab(id, () => []).catch(() => {});
});

chrome.runtime.onMessage.addListener((request, sender, respond) => {
  if (sender.id === chrome.runtime.id && ['GET_VIDEO_TARGET', 'DOWNLOAD_VIDEO'].includes(request.type) && Number.isInteger(sender.tab?.id) && !sender.tab.incognito) {
    void (async () => {
      const settings = await chrome.storage.local.get(DEFAULTS);
      if (!settings.showVideoButton || excludedHost(sender.tab.url, settings.excludedHosts)) return respond({ ok: false });
      const data = await chrome.storage.session.get(keyFor(sender.tab.id));
      const media = (data[keyFor(sender.tab.id)] || []).filter(item => Date.now() - item.timestamp < 30 * 60 * 1000);
      const streams = media.filter(item => item.playlist && !excludedHost(item.url, settings.excludedHosts));
      const direct = mediaDescriptor(request.source);
      const target = platformVideo(sender.tab.url, sender.tab.title) || direct
        || media.find(item => item.url === request.source)
        || (request.source?.startsWith('blob:') ? (streams[0] || (media.length ? media[media.length - 1] : undefined)) : undefined);
      if (!target || excludedHost(target.url, settings.excludedHosts)) return respond({ ok: false });
      if (request.type === 'GET_VIDEO_TARGET') return respond({ ok: true, target, language: settings.language });
      try {
        const ping = await connection();
        const headers = await buildAuthHeaders(target.url, sender.tab.url, settings, false);
        if (target.playlist && !ping.capabilities?.includes('ffmpeg-hls')) return respond({ ok: false, error: 'media_tools_required' });
        await sendDownload(
          target.url,
          target.platform ? undefined : target.filename,
          sender.tab.url,
          headers,
          undefined,
          undefined,
          !!ping.capabilities?.includes('ffmpeg-hls'),
          { streamKind: target.playlist ? target.type : undefined }
        );
        respond({ ok: true });
      }
      catch { respond({ ok: false, error: 'offline' }); }
    })().catch(() => respond({ ok: false }));
    return true;
  }
  if (sender.id !== chrome.runtime.id || request.type !== 'GET_TAB_MEDIA' || !Number.isInteger(request.tabId)) return false;
  const key = keyFor(request.tabId);
  void (writes.get(request.tabId) || Promise.resolve()).catch(() => {}).then(() => chrome.storage.session.get(key)).then((data) => respond({ media: (data[key] || []).filter((item) => Date.now() - item.timestamp < 30 * 60 * 1000) }), () => respond({ media: [] }));
  return true;
});
