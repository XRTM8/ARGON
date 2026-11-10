import { useEffect, useRef, useState, type FormEvent } from 'react';
import {
  ArrowDownToLine,
  Clipboard,
  SlidersHorizontal,
  Loader2,
  Film
} from 'lucide-react';
import { downloadManager } from '../core/downloadManager';
import { expandWildcardPattern, extractSharedUrl, parseDownloadInput } from '../core/downloadInput';
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
    queueId?: QueueId,
    mirrors?: string[]
  ) => void;
  detectedClipboardUrl?: string | null;
  intakeKey?: number;
  onClearClipboardUrl?: () => void;
}

export function QuickAddBar({ lang, onAdd, detectedClipboardUrl, intakeKey, onClearClipboardUrl }: Props) {
  const ar = lang === 'ar';
  const [value, setValue] = useState('');
  const [advanced, setAdvanced] = useState(false);
  const [filename, setFilename] = useState('');
  const [expectedHash, setExpectedHash] = useState('');
  const [mirrorsText, setMirrorsText] = useState('');
  const [queueId, setQueueId] = useState<QueueId>('main');
  const [segments, setSegments] = useState(() => downloadManager.getSettings().defaultSegmentCount || 8);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [probingMedia, setProbingMedia] = useState(false);
  const [mediaMeta, setMediaMeta] = useState<MediaMetadataInfo | null>(null);
  const [selectedFormatId, setSelectedFormatId] = useState<string>('');
  const input = useRef<HTMLTextAreaElement>(null);

  const trimmed = value.trim();
  const isMultiline = value.includes('\n');
  const looksLikeWildcard = !isMultiline && /\[(\d+\.\.\d+|[a-zA-Z]\.\.[a-zA-Z])\]/.test(trimmed);
  const wildcard = looksLikeWildcard ? expandWildcardPattern(trimmed) : null;
  const parsed = parseDownloadInput(value);
  const singleUrl = !isMultiline && !looksLikeWildcard && parsed.urls.length === 1 ? parsed.urls[0] : '';
  const platform = singleUrl ? detectPlatform(singleUrl) : 'other';
  const isMediaPlatform = !['direct', 'other'].includes(platform);

  useEffect(() => {
    if (detectedClipboardUrl) {
      setValue(detectedClipboardUrl);
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
    if (looksLikeWildcard) {
      if (!wildcard || wildcard.error || !wildcard.urls.length) {
        setError(
          wildcard?.error === 'range_too_large'
            ? ar ? 'الحد الأقصى للنمط المتسلسل 100 رابط.' : 'Pattern range is limited to 100 links.'
            : ar ? 'نمط غير صالح. مثال: https://example.com/part_[01..10].zip' : 'Invalid pattern. Example: https://example.com/part_[01..10].zip'
        );
        return;
      }
      for (const url of wildcard.urls) {
        onAdd(url, undefined, segments, undefined, undefined, queueId);
      }
      setNotice(ar ? `تمت إضافة ${wildcard.urls.length} روابط.` : `Added ${wildcard.urls.length} links.`);
      setValue('');
      setError('');
      onClearClipboardUrl?.();
      return;
    }

    const candidateList =
      !isMultiline && !parsed.urls.length && trimmed
        ? parseDownloadInput(extractSharedUrl(trimmed))
        : parsed;

    if (candidateList.invalid.length || !candidateList.urls.length) {
      setError(
        ar
          ? 'أدخل رابط HTTP أو HTTPS صحيح.'
          : 'Enter a valid HTTP or HTTPS URL.'
      );
      return;
    }
    if (candidateList.urls.length > 100) {
      setError(ar ? 'الحد الأقصى 100 رابط في الدفعة.' : 'Maximum 100 links at once.');
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

    const mirrors =
      candidateList.urls.length === 1 && mirrorsText.trim()
        ? mirrorsText
            .split(/\r?\n/)
            .map((u) => u.trim())
            .filter((u) => /^https?:\/\/.+/i.test(u))
        : undefined;

    for (const url of candidateList.urls) {
      onAdd(
        url,
        candidateList.urls.length === 1 ? filename.trim() || undefined : undefined,
        segments,
        candidateList.urls.length === 1 ? expectedHash.trim() || undefined : undefined,
        candidateList.urls.length === 1 ? mediaInfo : undefined,
        queueId,
        mirrors?.length ? mirrors : undefined
      );
    }

    setNotice(ar ? `تمت الإضافة (${candidateList.urls.length}).` : `Added (${candidateList.urls.length}).`);
    setValue('');
    setFilename('');
    setExpectedHash('');
    setMirrorsText('');
    setMediaMeta(null);
    setSelectedFormatId('');
    setError('');
    onClearClipboardUrl?.();
    input.current?.focus();
  };

  const countToSubmit = looksLikeWildcard ? wildcard?.urls.length || 0 : parsed.urls.length;

  const platformNames: Record<string, string> = {
    youtube: 'YouTube',
    instagram: 'Instagram',
    tiktok: 'TikTok',
    twitter: 'X / Twitter',
    facebook: 'Facebook',
    reddit: 'Reddit',
    threads: 'Threads',
    vimeo: 'Vimeo',
    soundcloud: 'SoundCloud'
  };

  return (
    <section className="quick-add" aria-label={ar ? 'إضافة تنزيل' : 'Add download'}>
      <form onSubmit={submit}>
        <div className="quick-add-row">
          <div className="url-field">
            <textarea
              ref={input}
              id="download-url"
              dir="ltr"
              rows={isMultiline ? 3 : 1}
              value={value}
              spellCheck={false}
              autoCapitalize="off"
              autoComplete="off"
              placeholder={
                ar
                  ? 'الصق رابط ملف أو فيديو من إنستغرام، تيك توك، يوتيوب، X…'
                  : 'Paste a file link or YouTube, Instagram, TikTok, X URL…'
              }
              aria-invalid={!!error}
              onChange={(event) => {
                setValue(event.target.value);
                setError('');
                setNotice('');
                setMediaMeta(null);
              }}
              onKeyDown={(event) => {
                if (event.key === 'Enter' && (!isMultiline || event.ctrlKey || event.metaKey)) {
                  event.preventDefault();
                  submit(event);
                }
              }}
            />
            <button
              type="button"
              className="icon-button"
              aria-label={ar ? 'لصق' : 'Paste'}
              title={ar ? 'لصق' : 'Paste'}
              onClick={async () => {
                try {
                  const text = await navigator.clipboard.readText();
                  const next = text.includes('\n') ? text.trim() : extractSharedUrl(text);
                  setValue(next);
                  setError('');
                  setNotice('');
                  setMediaMeta(null);
                  input.current?.focus();
                } catch {
                  setError(
                    ar
                      ? 'تعذّرت قراءة الحافظة. الصق الرابط يدوياً.'
                      : 'Clipboard unavailable. Paste manually.'
                  );
                }
              }}
            >
              <Clipboard size={15} />
            </button>
          </div>

          <button type="submit" className="action-button primary" disabled={!trimmed}>
            <ArrowDownToLine size={16} />
            <span>
              {ar
                ? countToSubmit > 1
                  ? `تنزيل (${countToSubmit})`
                  : 'تنزيل'
                : countToSubmit > 1
                  ? `Download (${countToSubmit})`
                  : 'Download'}
            </span>
          </button>
        </div>

        <div className="quick-add-foot">
          <div style={{ display: 'flex', alignItems: 'center', gap: 10, flexWrap: 'wrap' }}>
            <button
              type="button"
              className="text-button"
              aria-expanded={advanced}
              aria-controls="download-options"
              onClick={() => setAdvanced(!advanced)}
            >
              <SlidersHorizontal size={13} />
              <span>{ar ? 'خيارات' : 'Options'}</span>
            </button>

            {isMediaPlatform && (
              <button
                type="button"
                className="media-pill-btn"
                disabled={probingMedia}
                onClick={() => void inspectMedia()}
              >
                {probingMedia ? <Loader2 size={13} className="animate-spin" /> : <Film size={13} />}
                <span>
                  {probingMedia
                    ? ar ? `جاري فحص ${platformNames[platform] || 'الفيديو'}…` : `Inspecting ${platformNames[platform] || 'media'}…`
                    : ar ? `${platformNames[platform] || 'وسائط'} · اختيار الجودة` : `${platformNames[platform] || 'Media'} · Quality`}
                </span>
              </button>
            )}
          </div>

          {(isMultiline || looksLikeWildcard) && trimmed && (
            <span>
              {looksLikeWildcard
                ? wildcard?.urls.length
                  ? ar ? `${wildcard.urls.length} رابط متسلسل` : `${wildcard.urls.length} pattern links`
                  : ar ? 'نمط غير مكتمل' : 'Incomplete pattern'
                : ar
                  ? `${parsed.urls.length} روابط`
                  : `${parsed.urls.length} links`}
            </span>
          )}
        </div>

        {mediaMeta && mediaMeta.formats.length > 0 && (
          <div className="media-probe-box">
            <div className="media-probe-head">
              {mediaMeta.thumbnail && (
                <img
                  src={mediaMeta.thumbnail}
                  alt=""
                  className="media-probe-thumb"
                  loading="lazy"
                />
              )}
              <div style={{ minWidth: 0, flex: 1 }}>
                <strong dir="auto">{mediaMeta.title}</strong>
                {mediaMeta.uploader && (
                  <span style={{ display: 'block', fontSize: 11, color: 'var(--argon-muted)' }} dir="auto">
                    {mediaMeta.uploader}
                  </span>
                )}
              </div>
              {mediaMeta.hasFfmpeg && (
                <span className="queue-pill">FFmpeg HD</span>
              )}
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
              {ar ? 'اسم الملف' : 'Filename'}
              <input
                value={filename}
                onChange={(event) => setFilename(event.target.value)}
                disabled={parsed.urls.length > 1 || looksLikeWildcard}
                placeholder="filename.ext"
                dir="ltr"
              />
            </label>
            <label>
              {ar ? 'الاتصالات المتوازية' : 'Connections'}
              <select value={segments} onChange={(event) => setSegments(Number(event.target.value))}>
                {[1, 4, 8, 16, 32].map((count) => (
                  <option key={count} value={count}>
                    {count}
                  </option>
                ))}
              </select>
            </label>
            <label>
              {ar ? 'الطابور' : 'Queue'}
              <select value={queueId} onChange={(event) => setQueueId(event.target.value as QueueId)}>
                <option value="main">{ar ? 'رئيسي' : 'Main'}</option>
                <option value="night">{ar ? 'ليلي' : 'Night'}</option>
                <option value="media">{ar ? 'وسائط' : 'Media'}</option>
              </select>
            </label>
            <label>
              {ar ? 'بصمة SHA-256 (اختياري)' : 'Expected SHA-256'}
              <input
                value={expectedHash}
                onChange={(event) => setExpectedHash(event.target.value)}
                disabled={parsed.urls.length > 1 || looksLikeWildcard}
                placeholder="64 hex chars"
                dir="ltr"
              />
            </label>
            <label style={{ gridColumn: '1 / -1' }}>
              {ar ? 'روابط المرايا البديلة (اختياري)' : 'Mirror URLs (optional)'}
              <input
                value={mirrorsText}
                onChange={(event) => setMirrorsText(event.target.value)}
                disabled={parsed.urls.length > 1 || looksLikeWildcard}
                placeholder="https://mirror.example.com/file.zip"
                dir="ltr"
              />
            </label>
          </div>
        )}

        {error && (
          <p className="inline-error" role="alert">
            {error}
          </p>
        )}
        <output className="inline-notice" aria-live="polite">
          {notice}
        </output>
      </form>
    </section>
  );
}
