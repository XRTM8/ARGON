export const DEFAULTS = { autoIntercept: false, showVideoButton: true, forwardCookies: false, excludedHosts: [], minSizeBytes: 0, language: 'ar' };

export function platformVideo(value, title) {
  const url = httpUrl(value);
  if (!url) return null;
  const host = url.hostname.replace(/^www\.|^m\./g, '');
  let platform;
  if ((host === 'youtube.com' && ((url.pathname === '/watch' && url.searchParams.has('v')) || url.pathname.startsWith('/shorts/') || url.pathname.startsWith('/live/'))) || (host === 'youtu.be' && url.pathname.length > 1)) platform = 'YouTube';
  else if ((host === 'tiktok.com' || host.endsWith('.tiktok.com')) && (url.pathname.includes('/video/') || url.pathname.startsWith('/@') || host.startsWith('vt.'))) platform = 'TikTok';
  else if ((host === 'instagram.com' || host.endsWith('.instagram.com')) && /^\/(reel|reels|p|tv|stories)\//.test(url.pathname)) platform = 'Instagram';
  else if (['x.com', 'twitter.com', 't.co'].includes(host) && (url.pathname.includes('/status/') || host === 't.co')) platform = 'X';
  else if (['facebook.com', 'fb.watch', 'fb.com'].some(h => host === h || host.endsWith('.' + h)) && (url.pathname.includes('/watch') || url.pathname.includes('/reel') || url.pathname.includes('/videos') || url.pathname.includes('/story.php') || host === 'fb.watch')) platform = 'Facebook';
  else if (['reddit.com', 'redd.it', 'v.redd.it'].some(h => host === h || host.endsWith('.' + h)) && (url.pathname.includes('/comments/') || host === 'v.redd.it')) platform = 'Reddit';
  else if (['pinterest.com', 'pin.it'].some(h => host === h || host.endsWith('.' + h) || host.includes('pinterest.')) && (url.pathname.includes('/pin/') || host === 'pin.it')) platform = 'Pinterest';
  else if (host === 'snapchat.com' && (url.pathname.includes('/spotlight/') || url.pathname.includes('/p/'))) platform = 'Snapchat';
  else if (host === 'linkedin.com' && (url.pathname.includes('/posts/') || url.pathname.includes('/feed/update/'))) platform = 'LinkedIn';
  else if (host === 'twitch.tv' && (url.pathname.includes('/videos/') || url.pathname.includes('/clip/'))) platform = 'Twitch';
  else if (host === 'kick.com' && (url.pathname.includes('/video/') || url.pathname.includes('/clip/'))) platform = 'Kick';
  else if (['dailymotion.com', 'dai.ly'].some(h => host === h || host.endsWith('.' + h))) platform = 'Dailymotion';
  else if (['bilibili.com', 'b23.tv'].some(h => host === h || host.endsWith('.' + h))) platform = 'Bilibili';
  else if (['threads.net', 'threads.com'].includes(host) && url.pathname.includes('/post/')) platform = 'Threads';
  else if (host === 'vimeo.com' && /^\/\d+/.test(url.pathname)) platform = 'Vimeo';
  else if (host === 'soundcloud.com' && /^\/[^/]+\/[^/]+/.test(url.pathname)) platform = 'SoundCloud';
  else if (host === 'bsky.app' && url.pathname.includes('/post/')) platform = 'Bluesky';
  return platform ? { url: value, filename: title || platform, type: platform, platform: true, supported: true } : null;
}

export function httpUrl(value) {
  try { const url = new URL(value); return ['http:', 'https:'].includes(url.protocol) && !!url.hostname ? url : null; }
  catch { return null; }
}

export function isMagnetUrl(value) {
  return typeof value === 'string' && /^magnet:\?[\s\S]*xt=urn:btih:/i.test(value.trim());
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
  if (['m4s', 'ts', 'cmfv', 'cmfa'].includes(ext)) return null;
  const playlist = ['m3u8', 'mpd'].includes(ext) || /mpegurl|dash\+xml/.test(mime);
  const direct = /^(video|audio)\//.test(mime) || ['mp4', 'webm', 'mkv', 'mov', 'mp3', 'm4a', 'ogg', 'wav', 'flac', 'aac', 'opus', 'm4v', 'avi', 'wmv', 'flv', '3gp'].includes(ext);
  if (!playlist && !direct) return null;
  let filename = url.pathname.split('/').pop() || 'media';
  try { filename = decodeURIComponent(filename); } catch { /* Keep malformed escaped filenames. */ }
  return { url: value, filename: filename.slice(0, 220), type: playlist ? (/dash|mpd/.test(`${mime} ${ext}`) ? 'DASH' : 'HLS') : (ext || mime.split('/')[1] || 'MEDIA').toUpperCase(), supported: !playlist, playlist: !!playlist };
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
    if (/\.(m3u8|mpd|m4s|ts|cmfv|cmfa)(?:$|[?#])/i.test(value)) continue;
    let name = element.getAttribute('download') || element.textContent?.trim().slice(0, 100) || '';
    if (!name) { try { name = decodeURIComponent(new URL(value).pathname.split('/').pop()) || 'File'; } catch { name = 'File'; } }
    const media = element.tagName !== 'A' || /\.(mp4|webm|mkv|mov|mp3|m4a|ogg|wav|flac)(?:$|[?#])/i.test(value);
    found.set(value, { url: value, filename: name, supported: true, type: 'FILE', media });
    if (found.size >= 100) break;
  }
  return [...found.values()];
}
