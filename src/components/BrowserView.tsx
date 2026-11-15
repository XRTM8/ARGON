import { useEffect, useState } from 'react';
import { Puzzle, FolderOpen, Check, Copy, RefreshCw, ArrowUpRight, Globe, Compass } from 'lucide-react';
import { nativeRuntime } from '../core/flowClient';
import { downloadManager } from '../core/downloadManager';
import { writeClipboardText } from '../core/clipboard';

export function BrowserView({ lang }: { lang: 'ar' | 'en' }) {
  const ar = lang === 'ar';
  const isAndroid = /android/i.test(navigator.userAgent);
  const desktop = nativeRuntime() && !isAndroid;
  const [status, setStatus] = useState<'checking' | 'ready' | 'offline'>('checking');
  const [error, setError] = useState('');
  const [copied, setCopied] = useState('');
  const [snifferUrl, setSnifferUrl] = useState('https://');

  const check = async () => {
    setStatus('checking');
    if (!desktop) { setStatus('offline'); return; }
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const result = await invoke<{ listening: boolean; ready: boolean }>('browser_bridge_status');
      setStatus(result.listening && result.ready ? 'ready' : 'offline');
    } catch { setStatus('offline'); }
  };

  useEffect(() => { void check(); }, []);

  const copy = async (value: string) => {
    try { await writeClipboardText(value); setCopied(value); setTimeout(() => setCopied(''), 1600); }
    catch { setError(ar ? 'تعذّر النسخ. انسخ العنوان المعروض يدوياً.' : 'Could not copy. Copy the address shown manually.'); }
  };

  const openTest = async () => {
    setError('');
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('open_browser_test');
    } catch (err) {
      setError(String(err));
    }
  };

  const launchSniffer = async (targetUrl?: string) => {
    setError('');
    const url = targetUrl || snifferUrl.trim();
    if (!url || url === 'https://') {
      setError(ar ? 'يرجى إدخال رابط صالح لتصفحه واقتناص وسائطه.' : 'Please enter a valid URL to sniff.');
      return;
    }
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('open_media_sniffer', { url });
    } catch (err) {
      setError(String(err));
    }
  };

  return <div className="browser-view">
    <div className="view-header">
      <div className="view-heading">
        <h1>{ar ? (isAndroid ? 'اقتناص وسائط الويب' : 'إضافة المتصفح والالتقاط') : (isAndroid ? 'Mobile Media Sniffer' : 'Browser Integration')}</h1>
      </div>
      {isAndroid ? <Compass size={24} /> : <Puzzle size={24} />}
    </div>

    {isAndroid ? (
      <div className="browser-setup glass-panel" style={{ marginTop: '1rem' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: '0.75rem', marginBottom: '1rem' }}>
          <Globe size={22} className="text-accent" />
          <h2 style={{ margin: 0 }}>{ar ? 'متصفح اقتناص التدفقات الداخلي' : 'In-App Media Sniffer'}</h2>
        </div>
        <p style={{ color: 'var(--text-secondary)', marginBottom: '1.25rem', lineHeight: 1.6 }}>
          {ar
            ? 'تصفح أي موقع فيديو أو تواصل اجتماعي محمي. يقوم المتصفح الداخلي باعتراض تدفقات HLS (.m3u8) و DASH (.mpd) وملفات الفيديو تلقائياً مع زر تنزيل فوري بضغطة واحدة.'
            : 'Browse any video or authenticated streaming site. The in-app sniffer automatically intercepts HLS (.m3u8), DASH (.mpd), and direct video streams with one-tap download.'}
        </p>

        <div style={{ display: 'flex', gap: '0.5rem', marginBottom: '1rem', flexDirection: 'column' }}>
          <div style={{ display: 'flex', gap: '0.5rem' }}>
            <input
              type="url"
              value={snifferUrl}
              onChange={(e) => setSnifferUrl(e.target.value)}
              placeholder="https://..."
              className="action-input"
              style={{
                flex: 1,
                padding: '0.75rem 1rem',
                borderRadius: '8px',
                background: 'rgba(255,255,255,0.05)',
                border: '1px solid rgba(255,255,255,0.1)',
                color: '#fff',
                fontFamily: 'monospace'
              }}
              dir="ltr"
            />
            <button
              className="action-button primary"
              onClick={() => void launchSniffer()}
              style={{ padding: '0.75rem 1.25rem', whiteSpace: 'nowrap' }}
            >
              <Compass size={16} />
              {ar ? 'فتح واقتناص' : 'Sniff Page'}
            </button>
          </div>

          <div style={{ display: 'flex', gap: '0.5rem', flexWrap: 'wrap', marginTop: '0.5rem' }}>
            <span style={{ fontSize: '0.85rem', color: 'var(--text-muted)', alignSelf: 'center' }}>
              {ar ? 'مواقع مقترحة:' : 'Quick shortcuts:'}
            </span>
            {[
              { name: 'YouTube', url: 'https://m.youtube.com' },
              { name: 'Instagram', url: 'https://instagram.com' },
              { name: 'TikTok', url: 'https://tiktok.com' },
              { name: 'Facebook', url: 'https://m.facebook.com' },
              { name: 'Pinterest', url: 'https://pinterest.com' },
              { name: 'Twitch', url: 'https://m.twitch.tv' },
            ].map((item) => (
              <button
                key={item.name}
                className="action-button secondary"
                style={{ fontSize: '0.8rem', padding: '0.35rem 0.65rem' }}
                onClick={() => {
                  setSnifferUrl(item.url);
                  void launchSniffer(item.url);
                }}
              >
                {item.name}
              </button>
            ))}
          </div>
        </div>
      </div>
    ) : (
      <>
        <div className="bridge-status">
          <span className={`presence ${status === 'ready' ? 'online' : ''}`} />
          <div>
            <strong>
              {status === 'checking'
                ? (ar ? 'جارٍ الفحص…' : 'Checking…')
                : status === 'ready'
                ? (ar ? 'الاتصال المحلي وجسر Native Messaging جاهزان' : 'Local bridge & Native Messaging are ready')
                : (ar ? 'الاتصال المحلي غير متاح' : 'Local bridge is unavailable')}
            </strong>
            <p>
              {desktop
                ? (ar
                  ? 'افتح الإضافة في المتصفح للتحقق من اتصالها بالتطبيق، أو استخدم Native Messaging Host المسجل تلقائياً.'
                  : 'Open the extension in your browser to check its connection to ARGON, or use the auto-registered Native Messaging Host.')
                : (ar ? 'يتطلب الربط تطبيق ARGON لسطح المكتب.' : 'Browser integration requires the ARGON desktop app.')}
            </p>
          </div>
          <button className="icon-button" onClick={() => void check()} aria-label={ar ? 'إعادة الفحص' : 'Check again'}>
            <RefreshCw size={16} />
          </button>
        </div>

        <div className="browser-setup glass-panel">
          <h2>{ar ? 'إعداد Chrome أو Edge أو Firefox' : 'Set up Chrome, Edge, or Firefox'}</h2>
          <ol className="setup-steps">
            <li>
              <span>01</span>
              <div>
                <h3>{ar ? 'افتح مجلد الإضافة' : 'Find the extension folder'}</h3>
                <p>
                  {ar
                    ? 'المجلد مرفق مع التطبيق ويحتوي manifest.json لـ Chrome/Edge و manifest.firefox.json لـ Firefox.'
                    : 'The folder ships with ARGON containing manifest.json (Chrome/Edge) and manifest.firefox.json (Firefox).'}
                </p>
                <button
                  className="action-button"
                  disabled={!desktop}
                  onClick={async () => {
                    try {
                      await downloadManager.openBrowserExtensionDir();
                    } catch (err) {
                      setError(String(err));
                    }
                  }}
                >
                  <FolderOpen size={15} />
                  {ar ? 'فتح المجلد' : 'Open folder'}
                </button>
              </div>
            </li>
            <li>
              <span>02</span>
              <div>
                <h3>{ar ? 'افتح مدير إضافات متصفحك' : 'Open your browser’s extension manager'}</h3>
                <div className="browser-addresses">
                  {['chrome://extensions', 'edge://extensions', 'about:debugging#/runtime/this-firefox'].map((address) => (
                    <button className="action-button" key={address} onClick={() => void copy(address)} dir="ltr">
                      <code>{address}</code>
                      {copied === address ? <Check size={14} /> : <Copy size={14} />}
                    </button>
                  ))}
                </div>
                <p>{ar ? 'انسخ العنوان وافتحه في شريط المتصفح.' : 'Copy the address into your browser’s address bar.'}</p>
              </div>
            </li>
            <li>
              <span>03</span>
              <div>
                <h3>{ar ? 'حمّل الإضافة وثبّتها في الشريط' : 'Load the extension and pin it'}</h3>
                <p>
                  {ar
                    ? 'فعّل وضع المطوّر، اختر «تحميل إضافة غير مضغوطة»، وحدد مجلد browser-extension. الإضافة مدعومة بـ Native Messaging للاتصال الفوري دون الحاجة لمنافذ شبكية.'
                    : 'Enable Developer mode, choose Load unpacked, and select browser-extension. Backed by Native Messaging Host for instant zero-port IPC.'}
                </p>
              </div>
            </li>
            <li>
              <span>04</span>
              <div>
                <h3>{ar ? 'جرّب أول تنزيل' : 'Try your first download'}</h3>
                <p>
                  {ar
                    ? 'فعّل الالتقاط التلقائي في الإضافة ثم افتح ملف الاختبار. ابحث عن argon-browser-test.txt في قائمة تنزيلات ARGON.'
                    : 'Enable browser capture in the extension, then open the test file. Look for argon-browser-test.txt in ARGON.'}
                </p>
                <button className="action-button" disabled={status !== 'ready'} onClick={() => void openTest()}>
                  <ArrowUpRight size={15} />
                  {ar ? 'تنزيل ملف الاختبار' : 'Download test file'}
                </button>
              </div>
            </li>
          </ol>
        </div>
      </>
    )}

    <div className="integration-notes">
      <div>
        <h3>{ar ? 'الالتقاط بالخلفية وإذن الكوكيز' : 'Background capture & optional cookies'}</h3>
        <p>
          {ar
            ? 'يعمل الجسر في الخلفية عبر شريط النظام حتى عند إغلاق النافذة. يدعم ARGON استخراج جلسات الكوكيز تلقائياً من المتصفح لتنزيل الوسائط المحمية أو المقيدة بتسجيل دخول.'
            : 'The bridge runs in the background via System Tray. ARGON supports safe browser cookie extraction to download protected or age-restricted media seamlessly.'}
        </p>
      </div>
      <div>
        <h3>{ar ? 'دعم الوسائط و HLS/DASH' : 'Media & HLS/DASH support'}</h3>
        <p>
          {ar
            ? 'تنزيل ملفات الصوت والفيديو المباشرة وروابط المنصات الاجتماعية، وتُدمج مسارات الصوت والصورة تلقائياً بأعلى دقة عبر FFmpeg المدمج.'
            : 'Downloads direct media, social platforms, and HLS/DASH streams. High-definition separate audio/video streams are muxed losslessly via FFmpeg.'}
        </p>
      </div>
    </div>

    {error && <p className="inline-error" role="alert">{error}</p>}
    <span className="integration-foot">
      <ArrowUpRight size={14} />
      {ar ? 'البيانات والتنزيلات تدار محلياً على جهازك دون أي وسيط خارجي.' : 'Data and downloads are handled 100% locally on your device.'}
    </span>
  </div>;
}
