import { useEffect, useRef, useState, type FormEvent } from 'react';
import {
  ArrowDownToLine,
  Clipboard,
  ChevronDown,
  Link2,
  ListPlus,
  Sparkles,
  Loader2,
  Film
} from 'lucide-react';
import { downloadManager } from '../core/downloadManager';
import { expandWildcardPattern, parseDownloadInput } from '../core/downloadInput';
import { detectPlatform, listMediaFormats } from '../core/mediaResolver';
import { formatBytes } from '../core/segmentation';
import type { MediaMetadata, MediaMetadataInfo, QueueId } from '../types/downloader';

interface Props {
  lang: 'ar' | 'en';
  onAdd: (
    url: string,
    filename?: string,
    segments?: number,
    expectedSha256?: string,
    mediaInfo?: MediaMetadata,
    queueId?: QueueId
  ) => void;
  detectedClipboardUrl?: string | null;
  intakeKey?: number;
  onClearClipboardUrl?: () => void;
}

type IntakeMode = 'single' | 'batch' | 'wildcard';

export function QuickAddBar({ lang, onAdd, detectedClipboardUrl, intakeKey, onClearClipboardUrl }: Props) {
  const ar = lang === 'ar';
  const [value, setValue] = useState('');
  const [mode, setMode] = useState<IntakeMode>('single');
  const [advanced, setAdvanced] = useState(false);
  const [filename, setFilename] = useState('');
  const [expectedHash, setExpectedHash] = useState('');
  const [queueId, setQueueId] = useState<QueueId>('main');
  const [segments, setSegments] = useState(() => downloadManager.getSettings().defaultSegmentCount || 8);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [probingMedia, setProbingMedia] = useState(false);
  const [mediaMeta, setMediaMeta] = useState<MediaMetadataInfo | null>(null);
  const [selectedFormatId, setSelectedFormatId] = useState<string>('');
  const input = useRef<HTMLTextAreaElement>(null);

  const parsed = parseDownloadInput(value);
  const wildcard = mode === 'wildcard' ? expandWildcardPattern(value) : null;
  const singleUrl = mode === 'single' && parsed.urls.length === 1 ? parsed.urls[0] : '';
  const platform = singleUrl ? detectPlatform(singleUrl) : 'other';
  const isMediaPlatform = !['direct', 'other'].includes(platform);

  useEffect(() => {
    if (detectedClipboardUrl) {
      setValue(detectedClipboardUrl);
      setMode(detectedClipboardUrl.includes('\n') ? 'batch' : 'single');
      setError('');
      setMediaMeta(null);
      setSelectedFormatId('');
      input.current?.focus();
    }
  }, [detectedClipboardUrl, intakeKey]);

  useEffect(() => {
    const focus = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'n') {
        event.preventDefault();
        input.current?.focus();
      }
    };
    window.addEventListener('keydown', focus);
    return () => window.removeEventListener('keydown', focus);
  }, []);

  const inspectMedia = async () => {
    if (!singleUrl) return;
    setProbingMedia(true);
    setError('');
    try {
      const info = await listMediaFormats(singleUrl);
      setMediaMeta(info);
      setSelectedFormatId(info.formats[0]?.id || '');
    } catch (err) {
      setError(String(err));
      setMediaMeta(null);
    } finally {
      setProbingMedia(false);
    }
  };

  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (mode === 'wildcard') {
      if (!wildcard || wildcard.error || !wildcard.urls.length) {
        const msg =
          wildcard?.error === 'range_too_large'
            ? ar ? 'الحد الأقصى للنمط المتسلسل 100 رابط.' : 'Wildcard range is limited to 100 links.'
            : ar ? 'اكتب نمطاً صحيحاً مثل https://example.com/part_[01..10].zip' : 'Enter a valid pattern like https://example.com/part_[01..10].zip';
        setError(msg);
        return;
      }
      for (const url of wildcard.urls) {
        onAdd(url, undefined, segments, undefined, undefined, queueId);
      }
      setNotice(ar ? `تمت إضافة ${wildcard.urls.length} روابط متسلسلة.` : `Added ${wildcard.urls.length} pattern links.`);
      setValue('');
      setError('');
      onClearClipboardUrl?.();
      return;
    }

    if (parsed.invalid.length || !parsed.urls.length) {
      setError(
        ar
          ? 'أدخل رابط HTTP أو HTTPS كامل في كل سطر. صحّح الروابط غير الصالحة أولاً.'
          : 'Use a complete HTTP or HTTPS URL on each line. Fix invalid links first.'
      );
      return;
    }
    if (parsed.urls.length > 100) {
      setError(ar ? 'الحد الأقصى 100 رابط في الدفعة.' : 'Add up to 100 links at a time.');
      return;
    }

    const chosenFormat = mediaMeta?.formats.find((f) => f.id === selectedFormatId);
    const mediaInfo: MediaMetadata | undefined = chosenFormat
      ? {
          title: mediaMeta?.title,
          author: mediaMeta?.uploader,
          thumbnail: mediaMeta?.thumbnail,
          duration: mediaMeta?.duration,
          platform: platform === 'direct' || platform === 'other' ? undefined : platform,
          quality: chosenFormat.resolution,
          isAudioOnly: !chosenFormat.hasVideo && chosenFormat.hasAudio,
          selectedFormatId: chosenFormat.hasVideo && !chosenFormat.hasAudio && mediaMeta?.hasFfmpeg
            ? `${chosenFormat.id}+bestaudio/best`
            : chosenFormat.id,
          useYtdlpMux: !!(chosenFormat.hasVideo && !chosenFormat.hasAudio && mediaMeta?.hasFfmpeg)
        }
      : undefined;

    for (const url of parsed.urls) {
      onAdd(
        url,
        parsed.urls.length === 1 ? filename.trim() || undefined : undefined,
        segments,
        parsed.urls.length === 1 ? expectedHash.trim() || undefined : undefined,
        parsed.urls.length === 1 ? mediaInfo : undefined,
        queueId
      );
    }

    setNotice(ar ? `تمت إضافة ${parsed.urls.length} إلى القائمة.` : `${parsed.urls.length} added to the list.`);
    setValue('');
    setFilename('');
    setExpectedHash('');
    setMediaMeta(null);
    setSelectedFormatId('');
    setError('');
    onClearClipboardUrl?.();
    input.current?.focus();
  };

  const countToSubmit = mode === 'wildcard' ? wildcard?.urls.length || 0 : parsed.urls.length;

  return (
    <section className="quick-add" aria-label={ar ? 'إضافة تنزيل' : 'Add download'}>
      <form onSubmit={submit}>
        <div className="quick-add-heading">
          <label htmlFor="download-url">
            <Link2 size={15} className="accent-text" />
            <span>{ar ? 'إدخال رابط جديد' : 'Add download'}</span>
          </label>

          <div className="intake-mode-tabs" role="group" aria-label={ar ? 'وضع الإدخال' : 'Intake mode'}>
            <button
              type="button"
              aria-pressed={mode === 'single'}
              onClick={() => { setMode('single'); setError(''); }}
            >
              {ar ? 'مباشر' : 'Single'}
            </button>
            <button
              type="button"
              aria-pressed={mode === 'batch'}
              onClick={() => { setMode('batch'); setError(''); }}
            >
              <ListPlus size={13} />
              {ar ? 'متعدد' : 'Batch'}
            </button>
            <button
              type="button"
              aria-pressed={mode === 'wildcard'}
              onClick={() => { setMode('wildcard'); setError(''); }}
            >
              <Sparkles size={13} />
              {ar ? 'نمط [01..N]' : 'Pattern'}
            </button>
          </div>
        </div>

        <div className="quick-add-row">
          <div className="url-field">
            <textarea
              ref={input}
              id="download-url"
              dir="ltr"
              rows={mode === 'batch' ? 4 : 1}
              value={value}
              spellCheck={false}
              autoCapitalize="off"
              autoComplete="off"
              placeholder={
                mode === 'batch'
                  ? 'https://example.com/file.zip\nhttps://example.com/video.mp4'
                  : mode === 'wildcard'
                    ? 'https://example.com/archive_[01..12].zip'
                    : 'https://example.com/file.zip'
              }
              aria-invalid={!!error}
              aria-describedby="download-input-help"
              onChange={(event) => {
                const next = event.target.value;
                setValue(next);
                setError('');
                setNotice('');
                setMediaMeta(null);
                if (next.includes('\n') && mode === 'single') setMode('batch');
              }}
              onKeyDown={(event) => {
                if (event.key === 'Enter' && (mode !== 'batch' || event.ctrlKey || event.metaKey)) {
                  event.preventDefault();
                  submit(event);
                }
              }}
            />
            <button
              type="button"
              className="icon-button"
              aria-label={ar ? 'لصق الروابط' : 'Paste links'}
              title={ar ? 'لصق' : 'Paste'}
              onClick={async () => {
                try {
                  const text = await navigator.clipboard.readText();
                  const trimmed = text.trim();
                  setValue(trimmed);
                  setMode(trimmed.includes('\n') ? 'batch' : 'single');
                  setError('');
                  setMediaMeta(null);
                  input.current?.focus();
                } catch {
                  setError(
                    ar
                      ? 'تعذّرت قراءة الحافظة. استخدم Ctrl+V داخل حقل الرابط.'
                      : 'Clipboard unavailable. Paste with Ctrl+V in the link field.'
                  );
                }
              }}
            >
              <Clipboard size={15} />
            </button>
          </div>

          <button type="submit" className="action-button primary" disabled={!value.trim()}>
            <ArrowDownToLine size={16} />
            <span>
              {ar
                ? countToSubmit > 1
                  ? `إضافة ${countToSubmit} روابط`
                  : 'تنزيل'
                : countToSubmit > 1
                  ? `Add ${countToSubmit} links`
                  : 'Download'}
            </span>
          </button>
        </div>

        <div className="quick-add-foot" id="download-input-help">
          <div style={{ display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap' }}>
            <button
              type="button"
              className="text-button"
              aria-expanded={advanced}
              aria-controls="download-options"
              onClick={() => setAdvanced(!advanced)}
            >
              <ChevronDown size={14} className={advanced ? 'rotate-180' : ''} />
              {ar ? 'خيارات التنزيل' : 'Download options'}
            </button>

            {isMediaPlatform && (
              <button
                type="button"
                className="text-button"
                disabled={probingMedia}
                onClick={() => void inspectMedia()}
              >
                {probingMedia ? <Loader2 size={13} className="animate-spin" /> : <Film size={13} />}
                <span>{ar ? 'جلب الدقات المتاحة' : 'Inspect qualities'}</span>
              </button>
            )}
          </div>

          <span>
            {value.trim() && mode === 'batch'
              ? ar
                ? `${parsed.urls.length} صالحة · ${parsed.invalid.length} غير صالحة · ${parsed.duplicates} مكررة`
                : `${parsed.urls.length} valid · ${parsed.invalid.length} invalid · ${parsed.duplicates} duplicates`
              : mode === 'wildcard'
                ? wildcard?.urls.length
                  ? ar
                    ? `سيتم توليد ${wildcard.urls.length} رابط متسلسل`
                    : `${wildcard.urls.length} sequential links ready`
                  : ar
                    ? 'يدعم الأرقام [01..20] والأحرف [a..f]'
                    : 'Supports [01..20] and [a..f] ranges'
                : typeof window !== 'undefined' && window.innerWidth <= 760
                  ? ar
                    ? 'يدعم الروابط المباشرة أو لصق عدة روابط'
                    : 'Direct links or batch paste supported'
                  : ar
                    ? 'الصق رابطاً أو اسحبه هنا · Ctrl+N · Ctrl+K'
                    : 'Paste or drop a link · Ctrl+N · Ctrl+K'}
          </span>
        </div>

        {mediaMeta && mediaMeta.formats.length > 0 && (
          <div className="media-probe-box">
            <div className="media-probe-head">
              <strong dir="auto">{mediaMeta.title}</strong>
              <span style={{ fontSize: 11, color: 'var(--argon-subtle)' }}>
                {mediaMeta.hasFfmpeg
                  ? ar ? 'دمج HD/4K مفعّل' : 'FFmpeg HD mux ready'
                  : ar ? 'صيغ مباشرة' : 'Direct formats'}
              </span>
            </div>
            <div className="media-format-chips">
              {mediaMeta.formats.slice(0, 12).map((fmt) => (
                <button
                  key={fmt.id}
                  type="button"
                  className={`format-chip ${selectedFormatId === fmt.id ? 'is-active' : ''}`}
                  onClick={() => setSelectedFormatId(fmt.id)}
                >
                  {fmt.label}
                  {fmt.filesize ? ` (${formatBytes(fmt.filesize, 1)})` : ''}
                </button>
              ))}
            </div>
          </div>
        )}

        {advanced && (
          <div className="download-options" id="download-options">
            <label>
              {ar ? 'اسم الملف (لرابط واحد)' : 'Filename (single link)'}
              <input
                value={filename}
                onChange={(event) => setFilename(event.target.value)}
                disabled={parsed.urls.length > 1 || mode === 'wildcard'}
                placeholder="filename.ext"
                dir="ltr"
              />
            </label>
            <label>
              {ar ? 'الحد الأقصى للاتصالات' : 'Maximum connections'}
              <select value={segments} onChange={(event) => setSegments(Number(event.target.value))}>
                {[1, 4, 8, 16, 32].map((count) => (
                  <option key={count} value={count}>
                    {count}
                  </option>
                ))}
              </select>
            </label>
            <label>
              {ar ? 'الطابور المستهدف' : 'Target queue'}
              <select value={queueId} onChange={(event) => setQueueId(event.target.value as QueueId)}>
                <option value="main">{ar ? 'الطابور الرئيسي' : 'Main Queue'}</option>
                <option value="night">{ar ? 'الجدول الليلي' : 'Night Queue'}</option>
                <option value="media">{ar ? 'طابور الوسائط' : 'Media Queue'}</option>
              </select>
            </label>
            <label>
              {ar ? 'بصمة SHA-256 المتوقعة (اختياري)' : 'Expected SHA-256 (optional)'}
              <input
                value={expectedHash}
                onChange={(event) => setExpectedHash(event.target.value)}
                disabled={parsed.urls.length > 1 || mode === 'wildcard'}
                placeholder="64 hex chars"
                dir="ltr"
              />
            </label>
            <p>
              {ar
                ? 'يقسّم المحرك الملف ديناميكياً إذا دعم الخادم النطاقات؛ ويتراجع تلقائياً لاتصال واحد عند الحاجة.'
                : 'The engine dynamically splits supported byte-range transfers and safely falls back to single stream when required.'}
            </p>
          </div>
        )}

        {error && (
          <p className="inline-error" role="alert">
            {error}
          </p>
        )}
        <span className="inline-notice" role="status">
          {notice}
        </span>
      </form>
    </section>
  );
}
