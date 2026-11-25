import { useEffect, useState } from 'react';
import { Key, X } from 'lucide-react';
import { downloadManager } from '../core/downloadManager';

interface Props {
  open: boolean;
  lang: 'ar' | 'en';
  onClose: () => void;
}

export function CookieModal({ open, lang, onClose }: Props) {
  const ar = lang === 'ar';
  const [cookieText, setCookieText] = useState('');
  const [loading, setLoading] = useState(false);
  const [status, setStatus] = useState<string | null>(null);
  const [statusOk, setStatusOk] = useState(false);

  useEffect(() => {
    if (!open) return;
    setLoading(true);
    setStatus(null);
    void downloadManager
      .loadCookiesText()
      .then((existing) => setCookieText(existing))
      .catch((err) => {
        setStatusOk(false);
        setStatus(String(err));
      })
      .finally(() => setLoading(false));
  }, [open]);

  if (!open) return null;

  const handleSave = async () => {
    setLoading(true);
    setStatus(null);
    try {
      let clean = cookieText.trim();
      if (clean && !clean.includes('\t') && !clean.startsWith('#')) {
        if (clean.includes('sessionid=')) {
          const match = clean.match(/sessionid=([^;\s]+)/);
          if (match) clean = match[1];
        }
        if (/^[a-zA-Z0-9%_-]{15,}$/.test(clean)) {
          clean = `# Netscape HTTP Cookie File\n# Converted by ARGON\n.instagram.com\tTRUE\t/\tTRUE\t2147483647\tsessionid\t${clean}\n`;
        }
      }
      await downloadManager.saveCookiesText(clean);
      setStatusOk(true);
      setStatus(ar ? `تم حفظ ملف الكوكيز بنجاح (${clean.length} بايت).` : `Cookies saved successfully (${clean.length} bytes).`);
    } catch (err) {
      setStatusOk(false);
      setStatus(String(err));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="dialog-backdrop" onClick={onClose}>
      <div
        className="glass-panel cookie-modal-panel"
        role="dialog"
        aria-modal="true"
        aria-labelledby="cookie-modal-title"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="cookie-modal-head">
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <Key size={18} className="accent-text" />
            <h3 id="cookie-modal-title" style={{ margin: 0, fontSize: 16 }}>
              {ar ? 'إدارة كوكيز الحسابات (cookies.txt)' : 'Account Cookies Manager'}
            </h3>
          </div>
          <button type="button" className="icon-button" onClick={onClose} aria-label={ar ? 'إغلاق' : 'Close'}>
            <X size={16} />
          </button>
        </div>

        <p className="cookie-modal-desc">
          {ar
            ? 'تتيح الكوكيز لـ ARGON تنزيل القصص والمنشورات المقيدة عبر yt-dlp. الصق محتوى ملف cookies.txt بصيغة Netscape أو رمز sessionid:'
            : 'Cookies allow downloading gated stories and posts via yt-dlp. Paste Netscape cookies.txt content or a sessionid token:'}
        </p>

        <textarea
          dir="ltr"
          value={cookieText}
          onChange={(e) => setCookieText(e.target.value)}
          placeholder={ar ? '# الصق كوكيز Netscape هنا أو رمز sessionid...' : '# Paste Netscape cookies here or sessionid...'}
          className="argon-input cookie-modal-textarea"
          rows={9}
        />

        {status && (
          <div className={`cookie-modal-status ${statusOk ? 'is-ok' : 'is-err'}`} role="status">
            {status}
          </div>
        )}

        <div className="cookie-modal-foot">
          <button
            type="button"
            className="action-button"
            onClick={() => {
              setCookieText('');
              void downloadManager.saveCookiesText('');
              setStatusOk(true);
              setStatus(ar ? 'تم مسح الكوكيز.' : 'Cookies cleared.');
            }}
            disabled={loading}
          >
            {ar ? 'مسح الكل' : 'Clear'}
          </button>
          <div style={{ display: 'flex', gap: 8 }}>
            <button type="button" className="action-button" onClick={onClose}>
              {ar ? 'إلغاء' : 'Cancel'}
            </button>
            <button type="button" className="action-button primary" onClick={() => void handleSave()} disabled={loading}>
              {loading ? (ar ? 'جارٍ الحفظ...' : 'Saving...') : ar ? 'حفظ وتطبيق الكوكيز' : 'Save & Apply Cookies'}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
