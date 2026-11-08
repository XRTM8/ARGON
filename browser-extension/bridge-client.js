import { httpUrl, mediaDescriptor } from './core.js';
const BASE = 'http://127.0.0.1:41523';

async function request(path, payload, timeout = 6000) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeout);
  try {
    const response = await fetch(`${BASE}${path}`, {
      method: payload ? 'POST' : 'GET', signal: controller.signal,
      headers: { 'X-Otium-Client': 'browser-extension', ...(payload ? { 'Content-Type': 'application/json' } : {}) },
      ...(payload ? { body: JSON.stringify(payload) } : {})
    });
    const data = await response.json().catch(() => ({}));
    if (!response.ok) throw new Error(data.error || `bridge_${response.status}`);
    return data;
  } finally { clearTimeout(timer); }
}

export async function connection() {
  const data = await request('/ping', null, 1800);
  if (data.app !== 'ARGON' || data.status !== 'ok') throw new Error('invalid_bridge');
  if (!data.capabilities?.includes('persisted-handoff')) throw new Error('update_required');
  return data;
}

export async function sendDownload(url, filename, referer) {
  if (!httpUrl(url) || mediaDescriptor(url)?.supported === false) throw new Error('unsupported_url');
  const headers = httpUrl(referer) ? { Referer: referer } : undefined;
  const result = await request('/add', { url, filename: filename || null, headers });
  if (result.status !== 'accepted') throw new Error('handoff_not_saved');
  return result;
}
