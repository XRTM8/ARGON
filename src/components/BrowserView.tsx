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
    <div className="view-header"><div className="view-heading"><span className="eyebrow">{ar ? 'من المتصفح إلى ملفاتك' : 'FROM YOUR BROWSER TO YOUR FILES'}</span><h1>{ar ? 'إضافة المتصفح' : 'Browser extension'}</h1><p className="view-description">{ar ? 'أرسل رابطاً، اختر ملفات الصفحة، وتحكم بالتقاط التنزيلات.' : 'Send a link, choose files from a page, and control download capture.'}</p></div><Puzzle size={28} /></div>
    <div className="bridge-status"><span className={`presence ${status === 'ready' ? 'online' : ''}`} /><div><strong>{status === 'checking' ? (ar ? 'جارٍ الفحص…' : 'Checking…') : status === 'ready' ? (ar ? 'الاتصال المحلي جاهز' : 'Local bridge is ready') : (ar ? 'الاتصال المحلي غير متاح' : 'Local bridge is unavailable')}</strong><p>{desktop ? (ar ? 'افتح الإضافة في المتصفح للتحقق من اتصالها بالتطبيق.' : 'Open the extension in your browser to check its connection to ARGON.') : (ar ? 'يتطلب الربط تطبيق ARGON لسطح المكتب.' : 'Browser integration requires the ARGON desktop app.')}</p></div><button className="icon-button" onClick={() => void check()} aria-label={ar ? 'إعادة الفحص' : 'Check again'}><RefreshCw size={16} /></button></div>
    <div className="browser-setup glass-panel"><h2>{ar ? 'إعداد Chrome أو Edge' : 'Set up Chrome or Edge'}</h2>
      <ol className="setup-steps">
        <li><span>01</span><div><h3>{ar ? 'افتح مجلد الإضافة' : 'Find the extension folder'}</h3><p>{ar ? 'المجلد مرفق مع التطبيق ويحتوي ملف manifest.json.' : 'The folder ships with ARGON and contains manifest.json.'}</p><button className="action-button" disabled={!desktop} onClick={async () => { try { await downloadManager.openBrowserExtensionDir(); } catch (err) { setError(String(err)); } }}><FolderOpen size={15} />{ar ? 'فتح المجلد' : 'Open folder'}</button></div></li>
        <li><span>02</span><div><h3>{ar ? 'افتح مدير إضافات متصفحك' : 'Open your browser’s extension manager'}</h3><div className="browser-addresses">{['chrome://extensions', 'edge://extensions'].map((address) => <button className="action-button" key={address} onClick={() => void copy(address)} dir="ltr"><code>{address}</code>{copied === address ? <Check size={14} /> : <Copy size={14} />}</button>)}</div><p>{ar ? 'انسخ العنوان وافتحه في شريط المتصفح.' : 'Copy the address into your browser’s address bar.'}</p></div></li>
        <li><span>03</span><div><h3>{ar ? 'حمّل الإضافة وثبّتها في الشريط' : 'Load the extension and pin it'}</h3><p>{ar ? 'فعّل وضع المطوّر، اختر «تحميل إضافة غير مضغوطة»، وحدد مجلد browser-extension. بعدها افتح الإضافة واضغط «فحص الاتصال».' : 'Enable Developer mode, choose Load unpacked, and select browser-extension. Open the extension and check the connection.'}</p></div></li>
      </ol>
    </div>
    <div className="integration-notes"><div><h3>{ar ? 'الالتقاط بإيدك' : 'Capture on your terms'}</h3><p>{ar ? 'فعّل الالتقاط التلقائي من الإضافة واستثنِ المواقع التي تفضّل تنزيل ملفاتها بالمتصفح. المتصفح يحتفظ بتنزيله إذا فشل التسليم.' : 'Enable automatic capture in the extension and exclude websites you prefer to download from in the browser. The browser keeps its download if handoff fails.'}</p></div><div><h3>{ar ? 'دعم واضح للوسائط' : 'Clear media support'}</h3><p>{ar ? 'تنزيل ملفات الصوت والفيديو المباشرة وروابط المنصات المدعومة. قوائم HLS/DASH وملفات blob ليست ملفات مباشرة، ولا تدعمها هذه النسخة.' : 'Download direct audio/video files and supported platform URLs. HLS/DASH playlists and blob URLs are not direct files and are not supported in this version.'}</p></div></div>
    {error && <p className="inline-error" role="alert">{error}</p>}
    <span className="integration-foot"><ArrowUpRight size={14} />{ar ? 'الإضافة تتصل بالتطبيق على جهازك فقط.' : 'The extension connects to ARGON on your own device.'}</span>
  </div>;
}
