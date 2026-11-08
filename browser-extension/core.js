export const DEFAULTS = { autoIntercept: false, excludedHosts: [], minSizeBytes: 0, language: 'ar' };

export function httpUrl(value) {
  try { const url = new URL(value); return ['http:', 'https:'].includes(url.protocol) && !!url.hostname ? url : null; }
  catch { return null; }
}

export function excludedHost(value, hosts = []) {
  const host = httpUrl(value)?.hostname.toLowerCase();
  return !!host && hosts.some((raw) => {
    const rule = String(raw).trim().toLowerCase().replace(/^\.+|\.+$/g, '');
    return rule && (host === rule || host.endsWith(`.${rule}`));
  });
}

export function mediaDescriptor(value, contentType = '') {
  const url = httpUrl(value);
  if (!url) return null;
  const mime = contentType.toLowerCase().split(';')[0].trim();
  const ext = /\.([a-z0-9]+)$/i.exec(url.pathname)?.[1]?.toLowerCase();
  const playlist = ['m3u8', 'mpd'].includes(ext) || /mpegurl|dash\+xml/.test(mime);
  const direct = /^(video|audio)\//.test(mime) || ['mp4', 'webm', 'mkv', 'mov', 'mp3', 'm4a', 'ogg', 'wav', 'flac'].includes(ext);
  if (!playlist && !direct) return null;
  let filename = url.pathname.split('/').pop() || 'media';
  try { filename = decodeURIComponent(filename); } catch { /* Keep malformed escaped filenames. */ }
  return { url: value, filename: filename.slice(0, 220), type: playlist ? (/dash|mpd/.test(`${mime} ${ext}`) ? 'DASH' : 'HLS') : (ext || mime.split('/')[1] || 'MEDIA').toUpperCase(), supported: !playlist };
}

export function shouldCapture(item, settings) {
  if (!settings.autoIntercept || item.state !== 'in_progress' || item.paused || item.incognito || item.byExtensionId) return false;
  if (item.danger && item.danger !== 'safe') return false;
  const url = item.finalUrl || item.url;
  if (!httpUrl(url) || excludedHost(url, settings.excludedHosts) || excludedHost(item.referrer, settings.excludedHosts)) return false;
  if (mediaDescriptor(url, item.mime)?.supported === false) return false;
  const bytes = item.fileSize > 0 ? item.fileSize : item.totalBytes;
  // Unknown-size downloads stay in the browser when a minimum-size rule is active.
  return !settings.minSizeBytes || (bytes > 0 && bytes >= settings.minSizeBytes);
}

export function parseUrls(text) {
  const urls = [], invalid = [], seen = new Set();
  for (const raw of text.split(/\r?\n/)) {
    const value = raw.trim();
    if (!value) continue;
    if (!httpUrl(value) || mediaDescriptor(value)?.supported === false) invalid.push(value);
    else if (!seen.has(value)) { seen.add(value); urls.push(value); }
  }
  return { urls, invalid };
}

// Self-contained so executeScript can run it only after the user requests a scan.
export function scanPageLinks() {
  const found = new Map();
  const extensions = /\.(zip|rar|7z|tar|gz|iso|exe|msi|dmg|apk|deb|rpm|pdf|epub|docx?|xlsx?|pptx?|csv|mp4|webm|mkv|mov|mp3|m4a|ogg|wav|flac)(?:$|[?#])/i;
  for (const element of document.querySelectorAll('a[href],video[src],audio[src],source[src]')) {
    const value = element.href || element.src;
    if (!value || !/^https?:\/\//i.test(value) || found.has(value)) continue;
    if (!extensions.test(value) && !element.hasAttribute('download') && element.tagName === 'A') continue;
    if (/\.(m3u8|mpd)(?:$|[?#])/i.test(value)) continue;
    let name = element.getAttribute('download') || element.textContent?.trim().slice(0, 100) || '';
    if (!name) { try { name = decodeURIComponent(new URL(value).pathname.split('/').pop()) || 'File'; } catch { name = 'File'; } }
    found.set(value, { url: value, filename: name, supported: true, type: 'FILE' });
    if (found.size >= 100) break;
  }
  return [...found.values()];
}
