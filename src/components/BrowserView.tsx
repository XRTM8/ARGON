import { useEffect, useState } from 'react';
import { FolderOpen, Check, Copy, RefreshCw, ArrowUpRight, Globe, Compass, Key, RotateCcw } from 'lucide-react';
import { nativeRuntime } from '../core/flowClient';
import { downloadManager } from '../core/downloadManager';
import { writeClipboardText } from '../core/clipboard';

export function BrowserView({ lang }: { lang: 'ar' | 'en' }) {
  const ar = lang === 'ar';
  const isAndroid = /android/i.test(navigator.userAgent);
  const desktop = nativeRuntime() && !isAndroid;
  const [status, setStatus] = useState<'checking' | 'ready' | 'offline'>('checking');
  const [bridgeToken, setBridgeToken] = useState('');
  const [error, setError] = useState('');
  const [copied, setCopied] = useState('');
  const [snifferUrl, setSnifferUrl] = useState('https://');

  const check = async () => {
    setStatus('checking');
    if (!desktop) {
      setStatus('offline');
      return;
    }
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const result = await invoke<{ listening: boolean; ready: boolean; token?: string }>('browser_bridge_status');
      setStatus(result.listening && result.ready ? 'ready' : 'offline');
      if (result.token) setBridgeToken(result.token);
    } catch {
      setStatus('offline');
    }
  };

  useEffect(() => {
    void check();
  }, []);

  const copy = async (value: string) => {
    try {
      await writeClipboardText(value);
      setCopied(value);
      setTimeout(() => setCopied(''), 1600);
    } catch {
      setError(ar ? 'تعذّر النسخ. انسخ القيمة المعروضة يدوياً.' : 'Could not copy. Copy the value manually.');
    }
  };

  const regenerateToken = async () => {
    setError('');
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const fresh = await invoke<string>('regenerate_bridge_token');
      setBridgeToken(fresh);
      setCopied('');
    } catch (err) {
      setError(String(err));
    }
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
      setError(ar ? 'يرجى إدخال رابط صالح لفتحه.' : 'Please enter a valid URL.');
      return;
    }
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('open_media_sniffer', { url });
    } catch (err) {
      setError(String(err));
    }
  };

  return (
    <div className="browser-view">
      <div className="view-header">
        <div className="view-heading">
          <h1>{isAndroid ? (ar ? 'متصفح اقتناص الوسائط' : 'Media Sniffer Browser') : ar ? 'إضافة المتصفح والوسائط' : 'Browser Extension & Media'}</h1>
        </div>
        <Compass size={24} className="accent-text" />
      </div>

      <div className="browser-setup glass-panel" style={{ marginTop: '1rem', marginBottom: '1.5rem' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: '0.75rem', marginBottom: '0.75rem' }}>
          <Globe size={20} className="accent-text" />
          <h2 style={{ margin: 0 }}>
            {isAndroid
              ? ar
                ? 'متصفح اقتناص الوسائط والتدفقات المدمج'
                : 'Built-in Media Sniffer Browser'
              : ar
                ? 'فتح روابط المنصات في المتصفح المقترن'
                : 'Open Platform Pages in Paired Browser'}
          </h2>
        </div>
        <p className="browser-lead">
          {isAndroid
            ? ar
              ? 'افتح صفحة الفيديو داخل المتصفح المدمج لاعتراض روابط البث (.m3u8 / MP4) مباشرةً.'
              : 'Open the page in the built-in Android WebView sniffer to intercept HLS (.m3u8) and direct video streams.'
            : ar
              ? 'يفتح هذا الاختصار الرابط في متصفحك الافتراضي لتلتقطه إضافة ARGON المثبتة في المتصفح.'
              : 'Opens the URL in your default browser so the installed ARGON browser extension can detect video and file links.'}
        </p>

        <div style={{ display: 'flex', gap: '0.5rem', marginBottom: '1rem', flexDirection: 'column' }}>
          <div style={{ display: 'flex', gap: '0.5rem' }}>
            <input
              type="url"
              value={snifferUrl}
              onChange={(e) => setSnifferUrl(e.target.value)}
              placeholder="https://..."
              className="argon-input"
              style={{ flex: 1, fontFamily: 'monospace' }}
              dir="ltr"
            />
            <button
              className="action-button primary"
              onClick={() => void launchSniffer()}
              style={{ whiteSpace: 'nowrap' }}
            >
              <Compass size={16} />
              {isAndroid ? (ar ? 'فتح واقتناص' : 'Sniff Page') : ar ? 'فتح في المتصفح' : 'Open in Browser'}
            </button>
          </div>

          <div style={{ display: 'flex', gap: '0.5rem', flexWrap: 'wrap', marginTop: '0.5rem', alignItems: 'center' }}>
            <span className="browser-shortcuts-label">
              {ar ? 'مواقع مقترحة:' : 'Quick shortcuts:'}
            </span>
            {[
              { name: 'Instagram', url: 'https://instagram.com' },
              { name: 'TikTok', url: 'https://tiktok.com' },
              { name: 'YouTube', url: 'https://m.youtube.com' },
              { name: 'Facebook', url: 'https://m.facebook.com' },
              { name: 'Pinterest', url: 'https://pinterest.com' },
              { name: 'Twitch', url: 'https://m.twitch.tv' }
            ].map((item) => (
              <button
                key={item.name}
                className="action-button"
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

      {desktop && (
        <>
          <div className="bridge-status">
            <span className={`presence ${status === 'ready' ? 'online' : ''}`} />
            <div>
              <strong>
                {status === 'checking'
                  ? ar
                    ? 'جارٍ الفحص…'
                    : 'Checking…'
                  : status === 'ready'
                    ? ar
                      ? 'الجسر المحلي (127.0.0.1:41523) جاهز ومؤمّن بمفتاح ربط'
                      : 'Local bridge (127.0.0.1:41523) is ready & token-authenticated'
                    : ar
                      ? 'الاتصال المحلي غير متاح'
                      : 'Local bridge is unavailable'}
              </strong>
              <p>
                {ar
                  ? 'تتواصل الإضافة مع التطبيق عبر HTTP محلي (127.0.0.1:41523) باستخدام مفتاح الربط أدناه، مع دعم إيقاظ التطبيق عبر Native Messaging.'
                  : 'The extension talks to ARGON over authenticated local loopback HTTP (127.0.0.1:41523) with optional Native Messaging wake.'}
              </p>
            </div>
            <button className="icon-button" onClick={() => void check()} aria-label={ar ? 'إعادة الفحص' : 'Check again'}>
              <RefreshCw size={16} />
            </button>
          </div>

          {bridgeToken && (
            <div className="browser-setup glass-panel" style={{ marginBottom: '1rem' }}>
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 10, flexWrap: 'wrap' }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  <Key size={16} className="accent-text" />
                  <strong>{ar ? 'مفتاح ربط الإضافة (Bridge Key)' : 'Extension Bridge Key'}</strong>
                </div>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
                  <code className="bridge-token-code" dir="ltr">
                    {bridgeToken.slice(0, 8)}••••••••{bridgeToken.slice(-6)}
                  </code>
                  <button type="button" className="action-button primary" onClick={() => void copy(bridgeToken)}>
                    {copied === bridgeToken ? <Check size={14} /> : <Copy size={14} />}
                    <span>{copied === bridgeToken ? (ar ? 'تم النسخ' : 'Copied') : ar ? 'نسخ المفتاح' : 'Copy Key'}</span>
                  </button>
                  <button
                    type="button"
                    className="action-button"
                    onClick={() => void regenerateToken()}
                    title={ar ? 'إصدار مفتاح ربط جديد (يتطلب إعادة لصقه في الإضافة)' : 'Generate a new Bridge Key'}
                  >
                    <RotateCcw size={14} />
                    <span>{ar ? 'تدوير المفتاح' : 'Rotate'}</span>
                  </button>
                </div>
              </div>
              <p className="browser-lead" style={{ marginTop: 8, marginBottom: 0 }}>
                {ar
                  ? 'انسخ هذا المفتاح والصقه مرة واحدة داخل نافذة إضافة المتصفح لإتمام الربط ومنع أي طرف غير مصرح له من إرسال روابط إلى ARGON.'
                  : 'Copy and paste this key once into the browser extension popup to authenticate handoffs and block unauthorized local callers.'}
              </p>
            </div>
          )}

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
                  <h3>{ar ? 'حمّل الإضافة وثبّتها ثم الصق مفتاح الربط' : 'Load the extension and pair with the Bridge Key'}</h3>
                  <p>
                    {ar
                      ? 'فعّل وضع المطوّر، اختر «تحميل إضافة غير مضغوطة»، حدد مجلد browser-extension، ثم افتح الإضافة والصق مفتاح الربط أعلاه.'
                      : 'Enable Developer mode, choose Load unpacked, select browser-extension, then paste the Bridge Key above into the extension popup.'}
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
              ? 'يعمل الجسر في الخلفية عبر شريط النظام. يمكنك تفعيل خيار تمرير الكوكيز داخل الإضافة عند الحاجة لتنزيل الملفات المقيدة بتسجيل الدخول.'
              : 'The bridge runs in the background via System Tray. You can opt in to cookie forwarding inside the extension for authenticated downloads.'}
          </p>
        </div>
        <div>
          <h3>{ar ? 'دعم الوسائط و HLS/DASH' : 'Media & HLS/DASH support'}</h3>
          <p>
            {ar
              ? 'يدعم تنزيل ملفات الصوت والفيديو المباشرة، وروابط المنصات المدعومة، وتدفقات HLS/DASH مع دمج المسارات عبر FFmpeg عند تثبيته من الإعدادات.'
              : 'Supports direct media, supported platforms, and HLS/DASH streams muxed via FFmpeg when installed from Settings.'}
          </p>
        </div>
      </div>

      {error && (
        <p className="inline-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
