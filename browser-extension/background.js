import { DEFAULTS, httpUrl, shouldCapture, mediaDescriptor, platformVideo, excludedHost } from './core.js';
import { connection, sendDownload } from './bridge-client.js';

// Session storage survives MV3 worker suspension, but is cleared with the browser session.
const writes = new Map();
const handoffKey = (id) => `handoff:${id}`;
// Recover a paused browser transfer if the worker was interrupted mid-handoff.
const recovered = (async () => {
  const data = await chrome.storage.local.get(null);
  for (const [key, record] of Object.entries(data)) {
    if (!key.startsWith('handoff:') || !record?.id) continue;
    try {
      const [item] = await chrome.downloads.search({ id: record.id });
      if (item?.state === 'in_progress' && item.startTime === record.startTime) {
        if (record.saved) await chrome.downloads.cancel(item.id);
        else if (item.paused) await chrome.downloads.resume(item.id);
      }
      await chrome.storage.local.remove(key);
    } catch { /* Keep the record for the next worker wake-up. */ }
  }
})();
const keyFor = (id) => `media:${id}`;
function updateTab(id, operation) {
  const previous = writes.get(id) || Promise.resolve();
  const next = previous.catch(() => {}).then(async () => {
    const key = keyFor(id);
    const data = await chrome.storage.session.get(key);
    const list = operation(data[key] || []);
    if (list === null) await chrome.storage.session.remove(key);
    else await chrome.storage.session.set({ [key]: list });
    await chrome.action.setBadgeText({ tabId: id, text: list?.filter((item) => item.supported).length ? String(list.filter((item) => item.supported).length) : '' }).catch(() => {});
    await chrome.action.setBadgeBackgroundColor({ tabId: id, color: '#587849' }).catch(() => {});
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
  if (!httpUrl(url)) { void notify(['اختر رابط HTTP أو HTTPS مباشر.', 'Choose a direct HTTP or HTTPS URL.']); return; }
  void connection().then(() => sendDownload(url, undefined, info.frameUrl || tab?.url)).then(
    () => notify(['تمت إضافة الرابط إلى ARGON.', 'Link added to ARGON.']),
    () => notify(['تعذّر تسليم الرابط. افتح ARGON وتحقق من دعم نوع الرابط.', 'Could not hand off this link. Open ARGON and check that the URL type is supported.'])
  );
});

chrome.downloads.onCreated.addListener((item) => { void capture(item).catch(() => {}); });
async function capture(item) {
  await recovered;
  const settings = await chrome.storage.local.get(DEFAULTS);
  if (!shouldCapture(item, settings)) return;
  // Require the durable-handoff protocol before touching any browser download.
  try { await connection(); } catch { return; }
  const [current] = await chrome.downloads.search({ id: item.id });
  if (!current || !shouldCapture(current, settings)) return;
  const key = handoffKey(item.id);
  await chrome.storage.local.set({ [key]: { id: item.id, startTime: current.startTime, saved: false } });
  // Pause, then send: prevents small files completing while ARGON takes over.
  try { await chrome.downloads.pause(item.id); } catch { await chrome.storage.local.remove(key); return; }
  try {
    await sendDownload(current.finalUrl || current.url, current.filename?.split(/[\\/]/).pop(), current.referrer);
  } catch {
    try { await chrome.downloads.resume(item.id); await chrome.storage.local.remove(key); } catch { /* Recover on the next wake-up. */ }
    return;
  }
  // Only an acknowledgement of a saved task permits cancellation. Keep browser history.
  try {
    await chrome.storage.local.set({ [key]: { id: item.id, startTime: current.startTime, saved: true } });
    await chrome.downloads.cancel(item.id);
    await chrome.storage.local.remove(key);
    await notify(['حُفظ التنزيل في ARGON.', 'Download saved in ARGON.']);
  } catch { await notify(['حُفظ التنزيل في ARGON؛ تحقق من تنزيل المتصفح أيضاً.', 'Saved in ARGON; also check the browser download.']); }
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
      const direct = mediaDescriptor(request.source);
      const target = platformVideo(sender.tab.url, sender.tab.title) || (direct?.supported ? direct : (data[keyFor(sender.tab.id)] || []).find(item => item.url === request.source && item.supported));
      if (!target || excludedHost(target.url, settings.excludedHosts)) return respond({ ok: false });
      if (request.type === 'GET_VIDEO_TARGET') return respond({ ok: true, target, language: settings.language });
      try { await connection(); await sendDownload(target.url, target.platform ? undefined : target.filename, sender.tab.url); respond({ ok: true }); }
      catch { respond({ ok: false, error: 'offline' }); }
    })().catch(() => respond({ ok: false }));
    return true;
  }
  if (sender.id !== chrome.runtime.id || request.type !== 'GET_TAB_MEDIA' || !Number.isInteger(request.tabId)) return false;
  const key = keyFor(request.tabId);
  void (writes.get(request.tabId) || Promise.resolve()).catch(() => {}).then(() => chrome.storage.session.get(key)).then((data) => respond({ media: (data[key] || []).filter((item) => Date.now() - item.timestamp < 30 * 60 * 1000) }), () => respond({ media: [] }));
  return true;
});
