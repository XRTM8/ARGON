import { httpUrl, mediaDescriptor } from './core.js';
const BASE = 'http://127.0.0.1:41523';

async function request(path, payload, timeout = 6000) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeout);
  try {
    const response = await fetch(`${BASE}${path}`, {
      method: payload ? 'POST' : 'GET',
      signal: controller.signal,
      headers: {
        'X-Otium-Client': 'browser-extension',
        ...(payload ? { 'Content-Type': 'application/json' } : {})
      },
      ...(payload ? { body: JSON.stringify(payload) } : {})
    });
    const data = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(data.error || `bridge_${response.status}`);
    return data;
  } finally {
    clearTimeout(timer);
  }
}

export async function connection(allowWake = true) {
  try {
    const data = await request('/ping', null, 1800);
    if (data.app !== 'ARGON' || data.status !== 'ok') throw new Error('invalid_bridge');
    if (!data.capabilities?.includes('persisted-handoff')) throw new Error('update_required');
    return data;
  } catch (err) {
    if (allowWake && typeof chrome !== 'undefined' && chrome.runtime?.sendNativeMessage) {
      try {
        await new Promise((resolve) => {
          chrome.runtime.sendNativeMessage('com.otium.downloader', { action: 'wake' }, () => {
            resolve();
          });
        });
        for (let i = 0; i < 5; i++) {
          await new Promise((r) => setTimeout(r, 600));
          try {
            const data = await request('/ping', null, 1000);
            if (data.app === 'ARGON' && data.status === 'ok') return data;
          } catch {
            // Keep polling
          }
        }
      } catch {
        // Native messaging fallback ignored
      }
    }
    throw err;
  }
}

export async function buildAuthHeaders(url, referer, settings, incognito = false) {
  const headers = {};
  if (httpUrl(referer)) headers.Referer = referer;
  if (!incognito && settings?.forwardCookies && typeof chrome !== 'undefined' && chrome.cookies?.getAll) {
    try {
      const cookies = await chrome.cookies.getAll({ url });
      if (Array.isArray(cookies) && cookies.length > 0) {
        const pair = cookies
          .filter((c) => c?.name)
          .map((c) => `${c.name}=${c.value ?? ''}`)
          .join('; ');
        if (pair) headers.Cookie = pair;
      }
    } catch {
      // Optional cookies permission not granted yet.
    }
  }
  return Object.keys(headers).length ? headers : undefined;
}

export async function sendDownload(
  url,
  filename,
  referer,
  extraHeaders,
  method,
  postBody,
  allowPlaylists = false
) {
  const desc = mediaDescriptor(url);
  if (!httpUrl(url) || (desc?.supported === false && !(allowPlaylists && desc?.playlist))) {
    throw new Error('unsupported_url');
  }
  const merged = { ...(httpUrl(referer) ? { Referer: referer } : {}), ...(extraHeaders || {}) };
  const headers = Object.keys(merged).length ? merged : undefined;
  const result = await request('/add', {
    url,
    filename: filename || null,
    headers,
    ...(method ? { method } : {}),
    ...(postBody ? { postBody } : {})
  });
  if (result.status !== 'accepted') throw new Error('handoff_not_saved');
  return result;
}
