import { useEffect, useState } from 'react';
import { Puzzle, FolderOpen, Check, Copy, RefreshCw, ArrowUpRight } from 'lucide-react';
import { nativeRuntime } from '../core/flowClient';
import { downloadManager } from '../core/downloadManager';

export function BrowserView({ lang }: { lang: 'ar' | 'en' }) {
  const ar = lang === 'ar';
  const desktop = nativeRuntime() && !/android/i.test(navigator.userAgent);
  const [status, setStatus] = useState<'checking' | 'ready' | 'offline'>('checking');
  const [error, setError] = useState('');
  const [copied, setCopied] = useState('');
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
    try { await navigator.clipboard.writeText(value); setCopied(value); setTimeout(() => setCopied(''), 1600); }
    catch { setError(ar ? 'تعذّر النسخ. انسخ العنوان المعروض يدوياً.' : 'Could not copy. Copy the address shown manually.'); }
  };
  return <div className="browser-view">
    <div className="view-header"><div className="view-heading"><h1>{ar ? 'إضافة المتصفح' : 'Browser extension'}</h1></div><Puzzle size={24} /></div>
    <div className="bridge-status"><span className={`presence ${status === 'ready' ? 'online' : ''}`} /><div><strong>{status === 'checking' ? (ar ? 'جارٍ الفحص…' : 'Checking…') : status === 'ready' ? (ar ? 'الاتصال المحلي جاهز' : 'Local bridge is ready') : (ar ? 'الاتصال المحلي غير متاح' : 'Local bridge is unavailable')}</strong><p>{desktop ? (ar ? 'افتح الإضافة في المتصفح للتحقق من اتصالها بالتطبيق.' : 'Open the extension in your browser to check its connection to ARGON.') : (ar ? 'يتطلب الربط تطبيق ARGON لسطح المكتب.' : 'Browser integration requires the ARGON desktop app.')}</p></div><button className="icon-button" onClick={() => void check()} aria-label={ar ? 'إعادة الفحص' : 'Check again'}><RefreshCw size={16} /></button></div>
    <div className="browser-setup glass-panel"><h2>{ar ? 'إعداد Chrome أو Edge' : 'Set up Chrome or Edge'}</h2>
      <ol className="setup-steps">
        <li><span>01</span><div><h3>{ar ? 'افتح مجلد الإضافة' : 'Find the extension folder'}</h3><p>{ar ? 'المجلد مرفق مع التطبيق ويحتوي manifest.json لـ Chrome/Edge و manifest.firefox.json لـ Firefox.' : 'The folder ships with ARGON containing manifest.json (Chrome/Edge) and manifest.firefox.json (Firefox).'}</p><button className="action-button" disabled={!desktop} onClick={async () => { try { await downloadManager.openBrowserExtensionDir(); } catch (err) { setError(String(err)); } }}><FolderOpen size={15} />{ar ? 'فتح المجلد' : 'Open folder'}</button></div></li>
        <li><span>02</span><div><h3>{ar ? 'افتح مدير إضافات متصفحك' : 'Open your browser’s extension manager'}</h3><div className="browser-addresses">{['chrome://extensions', 'edge://extensions', 'about:debugging#/runtime/this-firefox'].map((address) => <button className="action-button" key={address} onClick={() => void copy(address)} dir="ltr"><code>{address}</code>{copied === address ? <Check size={14} /> : <Copy size={14} />}</button>)}</div><p>{ar ? 'انسخ العنوان وافتحه في شريط المتصفح.' : 'Copy the address into your browser’s address bar.'}</p></div></li>
        <li><span>03</span><div><h3>{ar ? 'حمّل الإضافة وثبّتها في الشريط' : 'Load the extension and pin it'}</h3><p>{ar ? 'فعّل وضع المطوّر، اختر «تحميل إضافة غير مضغوطة»، وحدد مجلد browser-extension (لـ Firefox انسخ manifest.firefox.json فوق manifest.json أو اختره من about:debugging). بعدها افتح الإضافة واضغط «فحص الاتصال».' : 'Enable Developer mode, choose Load unpacked, and select browser-extension (on Firefox, load manifest.firefox.json via about:debugging). Open the extension and check the connection.'}</p></div></li>
      </ol>
    </div>
    <div className="integration-notes"><div><h3>{ar ? 'الالتقاط بالخلفية وإذن الكوكيز' : 'Background capture & optional cookies'}</h3><p>{ar ? 'يعمل الجسر في الخلفية عبر شريط النظام حتى عند إغلاق النافذة. يمكنك تفعيل تمرير الكوكيز اختيارياً من خيارات الإضافة للمواقع المحمية بتسجيل الدخول.' : 'The bridge runs in the background via System Tray even when closed. You can opt into cookie forwarding in the extension options for authenticated sites.'}</p></div><div><h3>{ar ? 'دعم الوسائط و HLS/DASH' : 'Media & HLS/DASH support'}</h3><p>{ar ? 'تنزيل ملفات الصوت والفيديو المباشرة وروابط المنصات، وتُفعّل قوائم HLS/DASH تلقائياً فور تثبيت FFmpeg من الإعدادات.' : 'Downloads direct media and supported platforms. HLS/DASH streams unlock automatically after enabling FFmpeg in Settings.'}</p></div></div>
    {error && <p className="inline-error" role="alert">{error}</p>}
    <span className="integration-foot"><ArrowUpRight size={14} />{ar ? 'الإضافة تتصل بالتطبيق على جهازك فقط.' : 'The extension connects to ARGON on your own device.'}</span>
  </div>;
}
