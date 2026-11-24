(() => {
  if (window.top !== window || chrome.extension?.inIncognitoContext) return;
  const host = document.createElement('span');
  host.style.cssText = 'all:initial;position:fixed;z-index:2147483646;display:none;pointer-events:none;';
  const shadow = host.attachShadow({ mode: 'closed' });
  const style = document.createElement('style');
  style.textContent = 'button{font:600 12px/1.3 "Tajawal","Segoe UI",Tahoma,system-ui,sans-serif;display:flex;align-items:center;gap:7px;background:rgba(11,14,21,0.92);backdrop-filter:blur(10px);-webkit-backdrop-filter:blur(10px);color:#F8FAFC;border:1px solid rgba(59,130,246,0.45);border-radius:8px;padding:7px 12px;cursor:pointer;pointer-events:auto;box-shadow:0 8px 24px rgba(0,0,0,0.55),inset 0 1px 0 rgba(255,255,255,0.12);transition:all .14s ease}button:hover{background:rgba(22,29,43,0.96);border-color:#60A5FA;box-shadow:0 10px 28px rgba(0,0,0,0.65),0 0 12px rgba(59,130,246,0.35);transform:translateY(-1px)}button:focus-visible{outline:2px solid #3B82F6;outline-offset:2px}button:disabled{opacity:.65;cursor:wait}';
  const button = document.createElement('button');
  button.type = 'button'; button.textContent = '↓ ARGON';
  shadow.append(style, button); document.documentElement.append(host);
  let video, available = false, language = 'ar', timer, refreshTimer, frame;
  function position() {
    if (!video || !available || !video.isConnected) { host.style.display = 'none'; return; }
    const bounds = video.getBoundingClientRect();
    if (bounds.width < 160 || bounds.height < 90 || bounds.bottom < 0 || bounds.top > innerHeight) { host.style.display = 'none'; return; }
    const width = button.offsetWidth || 140;
    host.style.left = `${Math.max(4, Math.min(innerWidth - width - 4, bounds.right - width - 10))}px`;
    host.style.top = `${Math.max(4, bounds.top + 10)}px`;
  }
  function getVideoSource(el) {
    if (!el) return '';
    return el.currentSrc || el.src || el.querySelector('source')?.src || '';
  }
  async function refresh() {
    if (!video) return;
    try {
      const response = await chrome.runtime.sendMessage({ type: 'GET_VIDEO_TARGET', source: getVideoSource(video) });
      available = !!response?.ok; language = response?.language || 'ar';
      button.textContent = language === 'ar' ? '↓ تنزيل بـ ARGON' : '↓ Download · ARGON';
      if (!available) host.style.display = 'none';
    } catch { available = false; host.style.display = 'none'; }
  }
  function show(element) {
    video = element; clearTimeout(timer);
    void refresh().then(() => { if (available) { host.style.display = 'block'; position(); timer = setTimeout(() => { host.style.display = 'none'; }, 4000); } });
  }
  document.addEventListener('pointerover', event => { if (event.target instanceof HTMLVideoElement) show(event.target); }, { passive: true });
  document.addEventListener('play', event => { if (event.target instanceof HTMLVideoElement) show(event.target); }, true);
  document.addEventListener('loadedmetadata', event => { if (event.target instanceof HTMLVideoElement) show(event.target); }, true);
  for (const event of ['scroll', 'resize']) window.addEventListener(event, () => { if (!frame) frame = requestAnimationFrame(() => { frame = null; position(); }); }, { passive: true, capture: true });
  host.addEventListener('pointerenter', () => clearTimeout(timer));
  host.addEventListener('pointerleave', () => { timer = setTimeout(() => { host.style.display = 'none'; }, 1500); });
  button.addEventListener('click', async event => {
    if (!event.isTrusted || !video || button.disabled) return;
    button.disabled = true;
    try {
      const result = await chrome.runtime.sendMessage({ type: 'DOWNLOAD_VIDEO', source: getVideoSource(video) });
      button.textContent = result?.ok ? (language === 'ar' ? 'أُضيف إلى ARGON' : 'Added to ARGON')
        : result?.error === 'media_tools_required' ? (language === 'ar' ? 'ثبّت FFmpeg من الإعدادات' : 'Install FFmpeg in Settings')
        : (language === 'ar' ? 'شغّل ARGON أولاً' : 'Open ARGON first');
    } catch { button.textContent = language === 'ar' ? 'أعد تحميل الصفحة' : 'Reload this page'; }
    finally { setTimeout(() => { button.disabled = false; host.style.display = 'none'; }, 2500); }
  });
  chrome.runtime.onMessage.addListener(request => { if (request.type === 'ARGON_MEDIA_CHANGED' && video) { clearTimeout(refreshTimer); refreshTimer = setTimeout(() => void refresh(), 250); } });
  chrome.storage.onChanged.addListener((changes, area) => { if (area === 'local' && (changes.showVideoButton || changes.excludedHosts)) void refresh(); });
  const playing = [...document.querySelectorAll('video')].find(element => !element.paused);
  if (playing) show(playing);
})();
