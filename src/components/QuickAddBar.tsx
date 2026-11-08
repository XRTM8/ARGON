import { useEffect, useRef, useState, type FormEvent } from 'react';
import { ArrowDownToLine, Clipboard, ChevronDown, Link2, ListPlus } from 'lucide-react';
import { downloadManager } from '../core/downloadManager';
import { parseDownloadInput } from '../core/downloadInput';

interface Props {
  lang: 'ar' | 'en';
  onAdd: (url: string, filename?: string, segments?: number) => void;
  detectedClipboardUrl?: string | null;
  intakeKey?: number;
  onClearClipboardUrl?: () => void;
}

export function QuickAddBar({ lang, onAdd, detectedClipboardUrl, intakeKey, onClearClipboardUrl }: Props) {
  const ar = lang === 'ar';
  const [value, setValue] = useState('');
  const [batch, setBatch] = useState(false);
  const [advanced, setAdvanced] = useState(false);
  const [filename, setFilename] = useState('');
  const [segments, setSegments] = useState(() => downloadManager.getSettings().defaultSegmentCount || 8);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const input = useRef<HTMLTextAreaElement>(null);
  const parsed = parseDownloadInput(value);

  useEffect(() => {
    if (detectedClipboardUrl) {
      setValue(detectedClipboardUrl); setBatch(detectedClipboardUrl.includes('\n'));
      setError(''); input.current?.focus();
    }
  }, [detectedClipboardUrl, intakeKey]);

  useEffect(() => {
    const focus = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'n') {
        event.preventDefault(); input.current?.focus();
      }
    };
    window.addEventListener('keydown', focus);
    return () => window.removeEventListener('keydown', focus);
  }, []);

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (parsed.invalid.length || !parsed.urls.length) {
      setError(ar ? 'أدخل رابط HTTP أو HTTPS كامل في كل سطر. صحّح الروابط غير الصالحة أولاً.' : 'Use a complete HTTP or HTTPS URL on each line. Fix invalid links first.'); return;
    }
    if (parsed.urls.length > 100) {
      setError(ar ? 'الحد الأقصى 100 رابط في الدفعة.' : 'Add up to 100 links at a time.'); return;
    }
    for (const url of parsed.urls) onAdd(url, parsed.urls.length === 1 ? filename.trim() || undefined : undefined, segments);
    setNotice(ar ? `تمت إضافة ${parsed.urls.length} إلى القائمة.` : `${parsed.urls.length} added to the list.`);
    setValue(''); setFilename(''); setError(''); onClearClipboardUrl?.(); input.current?.focus();
  };

  return (
    <section className="quick-add" aria-label={ar ? 'إضافة تنزيل' : 'Add download'}>
      <form onSubmit={submit}>
        <div className="quick-add-heading">
          <label htmlFor="download-url"><Link2 size={15} />{ar ? 'رابط جديد' : 'New download'}</label>
          <button type="button" className="text-button" aria-pressed={batch} onClick={() => setBatch(!batch)}>
            <ListPlus size={15} />{ar ? 'روابط متعددة' : 'Multiple links'}
          </button>
        </div>
        <div className="quick-add-row">
          <div className="url-field">
            <textarea ref={input} id="download-url" dir="ltr" rows={batch ? 4 : 1}
              value={value} spellCheck={false} autoCapitalize="off" autoComplete="off"
              placeholder={batch ? 'https://example.com/file.zip\nhttps://example.com/video.mp4' : 'https://example.com/file.zip'}
              aria-invalid={!!error} aria-describedby="download-input-help"
              onChange={(event) => { setValue(event.target.value); setError(''); setNotice(''); if (event.target.value.includes('\n')) setBatch(true); }}
              onKeyDown={(event) => { if (event.key === 'Enter' && (!batch || event.ctrlKey || event.metaKey)) { event.preventDefault(); submit(event); } }} />
            <button type="button" className="icon-button" aria-label={ar ? 'لصق الروابط' : 'Paste links'} title={ar ? 'لصق' : 'Paste'} onClick={async () => {
              try { const text = await navigator.clipboard.readText(); setValue(text.trim()); setBatch(text.includes('\n')); setError(''); input.current?.focus(); }
              catch { setError(ar ? 'تعذّرت قراءة الحافظة. استخدم Ctrl+V داخل حقل الرابط.' : 'Clipboard unavailable. Paste with Ctrl+V in the link field.'); }
            }}><Clipboard size={16} /></button>
          </div>
          <button type="submit" className="action-button primary" disabled={!value.trim()}>
            <ArrowDownToLine size={16} />{ar ? (parsed.urls.length > 1 ? `إضافة ${parsed.urls.length} روابط` : 'تنزيل') : (parsed.urls.length > 1 ? `Add ${parsed.urls.length} links` : 'Download')}
          </button>
        </div>
        <div className="quick-add-foot" id="download-input-help">
          <button type="button" className="text-button" aria-expanded={advanced} aria-controls="download-options" onClick={() => setAdvanced(!advanced)}>
            <ChevronDown size={14} className={advanced ? 'rotate-180' : ''} />{ar ? 'خيارات التنزيل' : 'Download options'}
          </button>
          <span>{value.trim() && batch ? (ar ? `${parsed.urls.length} صالحة · ${parsed.invalid.length} غير صالحة · ${parsed.duplicates} مكررة` : `${parsed.urls.length} valid · ${parsed.invalid.length} invalid · ${parsed.duplicates} duplicates`) : (ar ? 'الصق رابطاً أو اسحبه هنا · Ctrl+N' : 'Paste a link or drop it here · Ctrl+N')}</span>
        </div>
        {advanced && <div className="download-options" id="download-options">
          <label>{ar ? 'اسم الملف (لرابط واحد)' : 'Filename (single link)'}
            <input value={filename} onChange={(event) => setFilename(event.target.value)} disabled={parsed.urls.length > 1} placeholder="filename.ext" dir="ltr" />
          </label>
          <label>{ar ? 'الحد الأقصى للاتصالات' : 'Maximum connections'}
            <select value={segments} onChange={(event) => setSegments(Number(event.target.value))}>
              {[1, 4, 8, 16, 32].map((count) => <option key={count} value={count}>{count}</option>)}
            </select>
          </label>
          <p>{ar ? 'يستخدم المحرك اتصالاً واحداً إذا كان الخادم لا يدعم تقسيم الملف.' : 'The engine uses one connection when the server does not support ranges.'}</p>
        </div>}
        {error && <p className="inline-error" role="alert">{error}</p>}
        <span className="inline-notice" role="status">{notice}</span>
      </form>
    </section>
  );
}
