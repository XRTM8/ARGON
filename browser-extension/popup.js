import { DEFAULTS, httpUrl, excludedHost, parseUrls, scanPageLinks } from './core.js';
import { connection, sendDownload } from './bridge-client.js';
const $ = (id) => document.getElementById(id);
let settings = await chrome.storage.local.get(DEFAULTS);
let language = settings.language === 'en' ? 'en' : 'ar';
let currentTab;
let media = [], links = [], activeList = 'media', selected = new Set(), sent = new Set(), busy = false, online = false, scanDone = false;
const strings = {
  integration:['إضافة المتصفح','Browser extension'], retry:['فحص الاتصال','Check connection'], capture:['التقاط التنزيلات تلقائياً','Capture downloads automatically'], captureHelp:['تنتقل التنزيلات إلى ARGON عند تأكيد الحفظ.','Downloads move to ARGON after it confirms saving.'],
  linkLabel:['إضافة رابط أو عدة روابط','Add one or more links'], usePage:['استخدام رابط الصفحة','Use page URL'], send:['إضافة إلى ARGON','Add to ARGON'], pageFiles:['ملفات هذه الصفحة','Files on this page'], scan:['فحص روابط الصفحة','Scan page links'], media:['وسائط مكتشفة','Detected media'], links:['روابط ملفات','File links'], selectAll:['تحديد الكل','Select all'], preferences:['تفضيلات الالتقاط','Capture preferences'], minSize:['الحد الأدنى للحجم','Minimum file size'], anySize:['أي حجم','Any size'], excludedHosts:['مواقع مستثناة (موقع بكل سطر)','Excluded sites (one host per line)'], save:['حفظ الاستثناءات','Save exclusions'],
  privacy:['تُحفظ التفضيلات على جهازك. تُحفظ الوسائط مؤقتاً لجلسة المتصفح. ملفات HLS/DASH غير مدعومة.','Preferences stay on your device. Detected media is kept for this browser session. HLS/DASH downloads are unsupported.'], local:['اتصال محلي فقط','Local connection only'], checking:['جارٍ فحص الاتصال…','Checking connection…'], connected:['متصل بـ ARGON','Connected to ARGON'], offline:['التطبيق غير متصل','App is not connected'], openApp:['افتح تطبيق ARGON ثم أعد الفحص.','Open the ARGON desktop app, then check again.'], ready:['جاهز لاستقبال روابطك','Ready to receive your links'], update:['حدّث تطبيق ARGON لدعم التسليم المؤكد.','Update ARGON to support confirmed handoffs.'],
  emptyMedia:['شغّل الوسائط في الصفحة ثم أعد فتح الإضافة. تظهر ملفات الصوت والفيديو المباشرة هنا.','Play media on the page, then reopen this popup. Direct audio and video files appear here.'], emptyLinks:['اضغط «فحص روابط الصفحة» لاختيار روابط الملفات الظاهرة في الصفحة.','Scan page links to choose file URLs visible on this page.'], noLinks:['لم يُعثر على روابط ملفات مباشرة في هذه الصفحة.','No direct file links were found on this page.'], playlist:['قائمة بث؛ غير مدعومة','Stream playlist; unsupported'], saved:['تمت الإضافة','Added'], invalid:['صحّح الروابط غير الصالحة. استخدم HTTP/HTTPS؛ قوائم HLS/DASH غير مدعومة.','Fix invalid links. Use HTTP/HTTPS; HLS/DASH playlists are unsupported.'], limit:['الحد الأقصى 100 رابط في الدفعة.','Add up to 100 links at a time.'], sending:['جارٍ الإرسال…','Sending…'], failure:['تعذّر التسليم. افتح ARGON وتحقق من الاتصال.','Could not hand off the download. Open ARGON and check the connection.'], restricted:['لا يمكن فحص هذه الصفحة. جرّب صفحة ويب عادية.','This page cannot be scanned. Try a regular web page.'], excluded:['استثناء هذا الموقع','Exclude this site'], included:['إعادة تفعيل هذا الموقع','Enable this site again'], hostsSaved:['تم حفظ الاستثناءات.','Exclusions saved.'], hostInvalid:['أدخل أسماء مواقع فقط، مثل example.com.','Enter hostnames only, such as example.com.']
};
const t = (key) => strings[key]?.[language === 'ar' ? 0 : 1] || key;
function feedback(message = '', error = false) { $('feedback').textContent = message; $('feedback').className = error ? 'error' : ''; }
function translate() {
  document.documentElement.lang = language; document.documentElement.dir = language === 'ar' ? 'rtl' : 'ltr';
  document.querySelectorAll('[data-i18n]').forEach((element) => { element.textContent = t(element.dataset.i18n); });
  $('language').textContent = language === 'ar' ? 'EN' : 'عربي';
  $('version').textContent = `ARGON ${chrome.runtime.getManifest().version}`;
  updateSiteButton(); render();
}
async function checkStatus() {
  $('statusText').textContent = t('checking'); $('checkStatus').disabled = true;
  try {
    const result = await connection(); online = true;
    $('statusText').textContent = `${t('connected')} · ${result.version}`;
    $('statusHelp').textContent = t('ready'); $('statusDot').className = 'dot online';
  } catch (error) {
    online = false; $('statusText').textContent = t('offline');
    $('statusHelp').textContent = error.message === 'update_required' ? t('update') : t('openApp'); $('statusDot').className = 'dot';
  } finally { $('checkStatus').disabled = false; }
}
function updateSiteButton() {
  const url = httpUrl(currentTab?.url);
  $('excludeSite').hidden = !url;
  if (url) $('excludeSite').textContent = `${t(excludedHost(url.href, settings.excludedHosts) ? 'included' : 'excluded')} · ${url.hostname}`;
}
function list() { return activeList === 'media' ? media : links; }
function render() {
  const items = list(); $('mediaList').replaceChildren();
  $('emptyMedia').hidden = !!items.length;
  $('emptyMedia').textContent = t(activeList === 'media' ? 'emptyMedia' : scanDone ? 'noLinks' : 'emptyLinks');
  $('mediaTab').setAttribute('aria-pressed', String(activeList === 'media'));
  $('linksTab').setAttribute('aria-pressed', String(activeList === 'links'));
  for (const item of items) {
    const row = document.createElement('label'); row.className = `media-item ${!item.supported ? 'unsupported' : ''}`;
    const checkbox = document.createElement('input'); checkbox.type = 'checkbox'; checkbox.checked = selected.has(item.url) || sent.has(item.url); checkbox.disabled = busy || !item.supported || sent.has(item.url); checkbox.setAttribute('aria-label', item.filename);
    checkbox.addEventListener('change', () => { checkbox.checked ? selected.add(item.url) : selected.delete(item.url); render(); });
    const info = document.createElement('div'); const title = document.createElement('span'); title.className = 'media-title'; title.textContent = item.filename; title.title = item.url;
    const type = document.createElement('span'); type.className = 'media-type'; type.textContent = sent.has(item.url) ? t('saved') : `${item.type} · ${item.supported ? httpUrl(item.url)?.hostname || '' : t('playlist')}`;
    info.append(title, type); row.append(checkbox, info); $('mediaList').append(row);
  }
  const available = items.filter((item) => item.supported && !sent.has(item.url));
  const count = available.filter((item) => selected.has(item.url)).length;
  $('selectionActions').hidden = !available.length;
  $('selectAll').checked = !!available.length && count === available.length;
  $('selectAll').indeterminate = count > 0 && count < available.length;
  $('selectAll').disabled = busy;
  $('sendSelected').disabled = busy || !count;
  $('sendSelected').textContent = busy ? t('sending') : `${t('send')} (${count})`;
}
async function sendItems(items, manual = false) {
  if (busy) return;
  busy = true; $('sendBtn').disabled = true; $('scanPage').disabled = true; render(); feedback(t('sending'));
  const failed = []; let success = 0;
  try {
    await connection();
    for (const item of items) {
      try { await sendDownload(item.url, item.filename, currentTab?.url); success++; sent.add(item.url); selected.delete(item.url); }
      catch { failed.push(item.url); }
    }
    if (manual) $('manualUrl').value = failed.join('\n');
    const result = language === 'ar' ? `أُضيفت ${success} · تعذّرت ${failed.length}.` : `${success} added · ${failed.length} failed.`;
    feedback(result + (failed.length ? ` ${t('failure')}` : ''), failed.length > 0);
  } catch (error) { feedback(error.message === 'update_required' ? t('update') : t('failure'), true); }
  finally { busy = false; $('sendBtn').disabled = false; $('scanPage').disabled = false; render(); void checkStatus(); }
}
$('manualForm').addEventListener('submit', (event) => {
  event.preventDefault(); const parsed = parseUrls($('manualUrl').value);
  if (parsed.invalid.length || !parsed.urls.length) { feedback(t('invalid'), true); return; }
  if (parsed.urls.length > 100) { feedback(t('limit'), true); return; }
  void sendItems(parsed.urls.map((url) => ({ url })), true);
});
$('usePage').addEventListener('click', () => { if (httpUrl(currentTab?.url)) { $('manualUrl').value = currentTab.url; $('manualUrl').focus(); } else feedback(t('restricted'), true); });
$('checkStatus').addEventListener('click', checkStatus);
$('language').addEventListener('click', async () => { language = language === 'ar' ? 'en' : 'ar'; await chrome.storage.local.set({ language }); translate(); feedback(); void checkStatus(); });
$('interceptToggle').checked = settings.autoIntercept;
$('interceptToggle').addEventListener('change', async () => { settings.autoIntercept = $('interceptToggle').checked; await chrome.storage.local.set({ autoIntercept: settings.autoIntercept }); });
$('excludeSite').addEventListener('click', async () => {
  const url = httpUrl(currentTab?.url); if (!url) return;
  if (excludedHost(url.href, settings.excludedHosts)) settings.excludedHosts = settings.excludedHosts.filter((host) => !excludedHost(url.href, [host]));
  else settings.excludedHosts = [...settings.excludedHosts, url.hostname];
  await chrome.storage.local.set({ excludedHosts: settings.excludedHosts }); $('excludedHosts').value = settings.excludedHosts.join('\n'); updateSiteButton();
});
$('minSize').value = String(settings.minSizeBytes);
$('minSize').addEventListener('change', async () => { settings.minSizeBytes = Number($('minSize').value); await chrome.storage.local.set({ minSizeBytes: settings.minSizeBytes }); });
$('excludedHosts').value = settings.excludedHosts.join('\n');
$('saveHosts').addEventListener('click', async () => {
  const hosts = [...new Set($('excludedHosts').value.split(/\r?\n/).map((host) => host.trim().toLowerCase()).filter(Boolean))];
  if (hosts.length > 100 || hosts.some((host) => !/^(?:[a-z0-9](?:[a-z0-9-]*[a-z0-9])?\.)*[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/.test(host))) { feedback(t('hostInvalid'), true); return; }
  settings.excludedHosts = hosts; await chrome.storage.local.set({ excludedHosts: hosts }); updateSiteButton(); feedback(t('hostsSaved'));
});
$('mediaTab').addEventListener('click', () => { activeList = 'media'; render(); });
$('linksTab').addEventListener('click', () => { activeList = 'links'; render(); });
$('selectAll').addEventListener('change', () => { for (const item of list().filter((item) => item.supported && !sent.has(item.url))) $('selectAll').checked ? selected.add(item.url) : selected.delete(item.url); render(); });
$('sendSelected').addEventListener('click', () => void sendItems(list().filter((item) => item.supported && selected.has(item.url) && !sent.has(item.url))));
$('scanPage').addEventListener('click', async () => {
  if (currentTab?.id === undefined || !httpUrl(currentTab.url)) { feedback(t('restricted'), true); return; }
  $('scanPage').disabled = true;
  try {
    const results = await chrome.scripting.executeScript({ target: { tabId: currentTab.id }, func: scanPageLinks });
    links = results[0]?.result || []; scanDone = true; activeList = 'links'; feedback(); render();
  } catch { feedback(t('restricted'), true); }
  finally { $('scanPage').disabled = false; }
});
try { [currentTab] = await chrome.tabs.query({ active: true, currentWindow: true }); } catch { /* Internal browser pages may not expose a tab. */ }
translate(); void checkStatus();
if (currentTab?.id !== undefined) {
  try { const result = await chrome.runtime.sendMessage({ type: 'GET_TAB_MEDIA', tabId: currentTab.id }); media = result?.media || []; render(); } catch { /* Popup remains usable when no media is available. */ }
  chrome.storage.onChanged.addListener((changes, area) => { if (area === 'session' && changes[`media:${currentTab.id}`]) { media = changes[`media:${currentTab.id}`].newValue || []; render(); } });
}
