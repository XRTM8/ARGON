// Development-only fixture for reviewing the real popup without installing it.
// Open /scripts/extension-preview.html with Vite. No request reaches the native bridge.
const preferences = { autoIntercept: false, language: 'ar', excludedHosts: [], minSizeBytes: 0 };
window.chrome = {
  runtime: {
    getManifest: () => ({ version: '2.0.0' }),
    sendMessage: async () => ({ media: [
      { url: 'https://cdn.example.com/film.mp4', filename: 'field-recording.mp4', type: 'MP4', supported: true },
      { url: 'https://cdn.example.com/audio.mp3', filename: 'interview-audio.mp3', type: 'MP3', supported: true },
      { url: 'https://cdn.example.com/master.m3u8', filename: 'master.m3u8', type: 'HLS', supported: false }
    ] })
  },
  tabs: { query: async () => [{ id: 1, url: 'https://example.com/downloads' }] },
  storage: { local: { get: async (defaults) => ({ ...defaults, ...preferences }), set: async (value) => Object.assign(preferences, value) }, onChanged: { addListener: () => {} } },
  scripting: { executeScript: async () => [{ result: [{ url: 'https://example.com/archive.zip', filename: 'archive.zip', supported: true, type: 'FILE' }, { url: 'https://example.com/manual.pdf', filename: 'manual.pdf', supported: true, type: 'FILE' }] }] }
};
const actualFetch = window.fetch;
window.fetch = async (url, options) => String(url).startsWith('http://127.0.0.1:41523') ? new Response(JSON.stringify(options?.method === 'POST' ? { status: 'accepted' } : { app: 'ARGON', version: '2.0.0', status: 'ok', capabilities: ['persisted-handoff'] }), { headers: { 'Content-Type': 'application/json' } }) : actualFetch(url, options);
