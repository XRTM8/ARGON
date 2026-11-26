import { useEffect, useMemo, useRef, useState, type FormEvent } from 'react';
import {
  ArrowDownToLine,
  Clipboard,
  SlidersHorizontal,
  Loader2,
  Film,
  Music,
  Sparkles,
  CheckCircle2,
  AlertCircle,
  Magnet,
  Cpu,
  X
} from 'lucide-react';
import { HuggingFaceModal } from './HuggingFaceModal';
import { downloadManager } from '../core/downloadManager';
import { expandWildcardPattern, extractSharedUrl, parseDownloadInput } from '../core/downloadInput';
import { readClipboardText } from '../core/clipboard';
import { detectPlatform, listMediaFormats } from '../core/mediaResolver';
import { isMagnetUri, parseTorrentFile, type TorrentMetadata } from '../core/torrent';
import { formatBytes } from '../core/segmentation';
import { isAndroidRuntime } from '../core/safeArea';
import type { MediaFormatInfo, MediaMetadata, MediaMetadataInfo, QueueId } from '../types/downloader';

interface Props {
  lang: 'ar' | 'en';
  onAdd: (
    url: string,
    filename?: string,
    segments?: number,
    expectedSha256?: string,
    mediaInfo?: MediaMetadata,
    queueId?: QueueId,
    mirrors?: string[],
    torrentMeta?: TorrentMetadata
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
  const [mediaTab, setMediaTab] = useState<'video' | 'audio'>('video');
  const [torrentMeta, setTorrentMeta] = useState<TorrentMetadata | null>(null);
  const [showHfModal, setShowHfModal] = useState(false);

  const { videoFormats, audioFormats, bestVideoId, bestAudioId } = useMemo(() => {
    if (!mediaMeta || !mediaMeta.formats.length) {
      return { videoFormats: [] as MediaFormatInfo[], audioFormats: [] as MediaFormatInfo[], bestVideoId: '', bestAudioId: '' };
    }

    const videos = mediaMeta.formats.filter((f) => f.hasVideo);
    const audios = mediaMeta.formats.filter((f) => !f.hasVideo && f.hasAudio);

    const rankVideo = (f: MediaFormatInfo) => {
      let score = 0;
      const hMatch = f.resolution.match(/(\d+)p?/i);
      const height = hMatch ? parseInt(hMatch[1], 10) : 0;
      score += height * 100;
      if (f.hasAudio) score += 500;
      if (f.ext === 'mp4') score += 50;
      if (f.fps && f.fps >= 60) score += 10;
      if (f.filesize) score += Math.min(f.filesize / (1024 * 1024), 200);
      return score;
    };

    const rankAudio = (f: MediaFormatInfo) => {
      let score = 0;
      const bMatch = (f.resolution + ' ' + f.label).match(/(\d+)\s*kbps/i);
      const kbps = bMatch ? parseInt(bMatch[1], 10) : 0;
      score += kbps * 100;
      if (f.ext === 'm4a' || f.ext === 'mp3') score += 50;
      if (f.filesize) score += Math.min(f.filesize / 1024, 500);
      return score;
    };

    let bestV = '';
    let maxVScore = -1;
    for (const v of videos) {
      const s = rankVideo(v);
      if (s > maxVScore) {
        maxVScore = s;
        bestV = v.id;
      }
    }

    let bestA = '';
    let maxAScore = -1;
    for (const a of audios) {
      const s = rankAudio(a);
      if (s > maxAScore) {
        maxAScore = s;
        bestA = a.id;
      }
    }

    return {
      videoFormats: videos,
      audioFormats: audios,
      bestVideoId: bestV,
      bestAudioId: bestA
    };
  }, [mediaMeta]);
  const [isDragOver, setIsDragOver] = useState(false);
  const input = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

  const handleTorrentFile = async (file: File) => {
    setError('');
    try {
      const buffer = await file.arrayBuffer();
      const meta = await parseTorrentFile(new Uint8Array(buffer));
      const magnet = `magnet:?xt=urn:btih:${meta.infoHash}&dn=${encodeURIComponent(meta.name)}`;
      setValue(magnet);
      setFilename(meta.name);
      setTorrentMeta(meta);
      setNotice(
        ar
          ? `تم استيراد ملف تورنت: ${meta.name} (${formatBytes(meta.totalLength)})`
          : `Loaded .torrent: ${meta.name} (${formatBytes(meta.totalLength)})`
      );
      setTimeout(() => setNotice((curr) => (curr.includes(meta.name) ? '' : curr)), 4500);
    } catch (err) {
      setError(ar ? `ملف تورنت غير صالح: ${String(err)}` : `Invalid .torrent file: ${String(err)}`);
    }
  };

  const trimmed = value.trim();
  const isMultiline = value.includes('\n');
  const looksLikeWildcard = !isMultiline && /\[(\d+\.\.\d+|[a-zA-Z]\.\.[a-zA-Z])\]/.test(trimmed);
  const wildcard = looksLikeWildcard ? expandWildcardPattern(trimmed) : null;
  const parsed = parseDownloadInput(value);
  const singleUrl = !isMultiline && !looksLikeWildcard && parsed.urls.length === 1 ? parsed.urls[0] : '';
  const isMagnet = !!singleUrl && isMagnetUri(singleUrl);
  const platform = singleUrl && !isMagnet ? detectPlatform(singleUrl) : 'other';
  let streamManifest = false;
  if (singleUrl && !isMagnet) {
    try {
      streamManifest = /\.(?:m3u8|mpd)$/i.test(new URL(singleUrl).pathname);
    } catch {
      streamManifest = false;
    }
  }
  const isMediaPlatform = !isMagnet && (streamManifest || !['direct', 'other'].includes(platform));

  useEffect(() => {
    if (detectedClipboardUrl) {
      setValue(detectedClipboardUrl);
      setError('');
      setMediaMeta(null);
      setSelectedFormatId('');
      setTorrentMeta(null);
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

  useEffect(() => {
    if (!singleUrl || !isMediaPlatform) {
      setMediaMeta(null);
      setSelectedFormatId('');
      return;
    }
    const timer = setTimeout(() => {
      void inspectMedia();
    }, 200);
    return () => clearTimeout(timer);
  }, [singleUrl, isMediaPlatform]);

  const inspectMedia = async () => {
    if (!singleUrl) return;
    setProbingMedia(true);
    setError('');
    try {
      const info = await listMediaFormats(singleUrl);
      setMediaMeta(info);
      const hasV = info.formats.some((f) => f.hasVideo);
      const hasA = info.formats.some((f) => !f.hasVideo && f.hasAudio);
      if (hasV) {
        setMediaTab('video');
        const best = info.formats.find((f) => f.hasVideo && f.hasAudio) || info.formats.find((f) => f.hasVideo);
        setSelectedFormatId(best?.id || info.formats[0]?.id || '');
      } else if (hasA) {
        setMediaTab('audio');
        const best = info.formats.find((f) => !f.hasVideo && f.hasAudio);
        setSelectedFormatId(best?.id || info.formats[0]?.id || '');
      } else {
        setSelectedFormatId(info.formats[0]?.id || '');
      }
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
          ? 'أدخل رابط HTTP أو HTTPS أو Magnet صحيح.'
          : 'Enter a valid HTTP, HTTPS, or Magnet URL.'
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
          useYtdlpMux: streamManifest || !!(chosenFormat.hasVideo && !chosenFormat.hasAudio && mediaMeta?.hasFfmpeg)
        }
      : (isMediaPlatform && platform !== 'direct' && platform !== 'other'
          ? { platform }
          : undefined);

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
        mirrors?.length ? mirrors : undefined,
        candidateList.urls.length === 1 ? torrentMeta || undefined : undefined
      );
    }

    const msg = ar ? `تمت الإضافة (${candidateList.urls.length}).` : `Added (${candidateList.urls.length}).`;
    setNotice(msg);
    setTimeout(() => setNotice((curr) => (curr === msg ? '' : curr)), 3500);
    setValue('');
    setFilename('');
    setExpectedHash('');
    setMirrorsText('');
    setMediaMeta(null);
    setSelectedFormatId('');
    setTorrentMeta(null);
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
    soundcloud: 'SoundCloud',
    pinterest: 'Pinterest',
    snapchat: 'Snapchat',
    linkedin: 'LinkedIn',
    twitch: 'Twitch',
    kick: 'Kick',
    dailymotion: 'Dailymotion',
    bilibili: 'Bilibili',
    kwai: 'Kwai',
    douyin: 'Douyin',
    bluesky: 'Bluesky',
    tumblr: 'Tumblr',
    streamable: 'Streamable',
    loom: 'Loom',
    vk: 'VK',
    rumble: 'Rumble',
    odysee: 'Odysee',
    bandcamp: 'Bandcamp'
  };

  return (
    <section
      className={`quick-add ${isDragOver ? 'drag-active' : ''}`}
      aria-label={ar ? 'إضافة تنزيل' : 'Add download'}
      onDragOver={(e) => {
        e.preventDefault();
        e.stopPropagation();
        setIsDragOver(true);
      }}
      onDragEnter={(e) => {
        e.preventDefault();
        e.stopPropagation();
        setIsDragOver(true);
      }}
      onDragLeave={(e) => {
        e.preventDefault();
        e.stopPropagation();
        setIsDragOver(false);
      }}
      onDrop={async (e) => {
        e.preventDefault();
        e.stopPropagation();
        setIsDragOver(false);
        const files = e.dataTransfer.files;
        if (files && files.length > 0) {
          const file = files[0];
          if (file.name.toLowerCase().endsWith('.torrent') || file.type === 'application/x-bittorrent') {
            await handleTorrentFile(file);
            return;
          }
        }
        const text = (e.dataTransfer.getData('text/uri-list') || e.dataTransfer.getData('text/plain') || '').trim();
        if (text) {
          setValue(text);
          setError('');
          setNotice('');
        }
      }}
    >
      <input
        ref={fileInputRef}
        type="file"
        accept=".torrent,application/x-bittorrent"
        style={{ display: 'none' }}
        onChange={async (e) => {
          const file = e.target.files?.[0];
          if (file) {
            await handleTorrentFile(file);
            e.target.value = '';
          }
        }}
      />
      <form onSubmit={submit}>
        <div className="quick-add-row">
          <div className="url-field">
            <textarea
              ref={input}
              id="download-url"
              dir={value ? 'ltr' : (ar ? 'rtl' : 'ltr')}
              rows={isMultiline ? 3 : 1}
              value={value}
              spellCheck={false}
              autoCapitalize="off"
              autoComplete="off"
              placeholder={
                ar
                  ? 'أدخل رابط التحميل أو أسقط ملفاً'
                  : 'Enter URL or drop a file'
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
                  const text = await readClipboardText();
                  const next = text.includes('\n') ? text.trim() : extractSharedUrl(text);
                  setValue(next);
                  setError('');
                  setNotice('');
                  setMediaMeta(null);
                  input.current?.focus();
                } catch {
                  input.current?.focus();
                  setError(
                    ar
                      ? 'تعذّرت قراءة الحافظة تلقائياً. اضغط مطولاً داخل الحقل ثم اختر "لصق".'
                      : 'Clipboard unavailable. Long-press the input field and select Paste.'
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

            {!isAndroidRuntime() && (
              <button
                type="button"
                className="text-button"
                onClick={() => setShowHfModal(true)}
                title={ar ? 'تنزيل نموذج من Hugging Face' : 'Download model from Hugging Face'}
              >
                <Cpu size={13} />
                <span>Hugging Face</span>
              </button>
            )}

            <button
              type="button"
              className="text-button"
              title={ar ? 'استيراد ملف .torrent (يُنزّل سحابياً عبر Real-Debrid)' : 'Import a .torrent file (resolved via Real-Debrid)'}
              onClick={() => fileInputRef.current?.click()}
            >
              <Magnet size={13} />
              <span>{ar ? 'تورنت (Real-Debrid)' : 'Torrent (Debrid)'}</span>
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

            {torrentMeta && (
              <div
                className="media-pill-btn is-torrent"
                style={{
                  display: 'inline-flex',
                  alignItems: 'center',
                  gap: '0.35rem'
                }}
                title={torrentMeta.infoHash}
              >
                <Magnet size={13} />
                <span dir="auto">
                  {torrentMeta.name} · {formatBytes(torrentMeta.totalLength)}
                  {torrentMeta.files.length > 1 ? ` (${torrentMeta.files.length} ${ar ? 'ملف' : 'files'})` : ''}
                </span>
                <button
                  type="button"
                  style={{
                    background: 'none',
                    border: 'none',
                    color: 'inherit',
                    cursor: 'pointer',
                    display: 'flex',
                    alignItems: 'center',
                    padding: 0,
                    marginInlineStart: 3
                  }}
                  onClick={() => {
                    setTorrentMeta(null);
                    setValue('');
                    setFilename('');
                  }}
                  title={ar ? 'إلغاء' : 'Clear'}
                >
                  <X size={12} />
                </button>
              </div>
            )}

            {!torrentMeta && isMagnet && (
              <div
                className="media-pill-btn is-torrent"
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: '0.35rem'
                }}
              >
                <Magnet size={13} />
                <span>{ar ? 'ماغنت · يتطلب Real-Debrid' : 'Magnet · Requires Real-Debrid'}</span>
              </div>
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
            {videoFormats.length > 0 && audioFormats.length > 0 && (
              <div className="media-category-tabs" role="tablist">
                <button
                  type="button"
                  role="tab"
                  aria-selected={mediaTab === 'video'}
                  className={`media-tab-btn ${mediaTab === 'video' ? 'is-active' : ''}`}
                  onClick={() => {
                    setMediaTab('video');
                    if (bestVideoId && !videoFormats.some((f) => f.id === selectedFormatId)) {
                      setSelectedFormatId(bestVideoId);
                    }
                  }}
                >
                  <Film size={13} />
                  <span>{ar ? 'فيديو' : 'Video'}</span>
                  <span className="media-tab-count">{videoFormats.length}</span>
                </button>
                <button
                  type="button"
                  role="tab"
                  aria-selected={mediaTab === 'audio'}
                  className={`media-tab-btn ${mediaTab === 'audio' ? 'is-active' : ''}`}
                  onClick={() => {
                    setMediaTab('audio');
                    if (bestAudioId && !audioFormats.some((f) => f.id === selectedFormatId)) {
                      setSelectedFormatId(bestAudioId);
                    }
                  }}
                >
                  <Music size={13} />
                  <span>{ar ? 'صوت' : 'Audio'}</span>
                  <span className="media-tab-count">{audioFormats.length}</span>
                </button>
              </div>
            )}

            <div className="media-format-grid">
              {(mediaTab === 'video' ? (videoFormats.length ? videoFormats : audioFormats) : (audioFormats.length ? audioFormats : videoFormats))
                .slice(0, 16)
                .map((fmt) => {
                  const isBest = (mediaTab === 'video' && fmt.id === bestVideoId) || (mediaTab === 'audio' && fmt.id === bestAudioId);
                  const isSelected = selectedFormatId === fmt.id;
                  const resText = fmt.hasVideo
                    ? fmt.resolution
                    : (fmt.resolution !== 'Audio' ? fmt.resolution : fmt.ext.toUpperCase());
                  return (
                    <button
                      key={fmt.id}
                      type="button"
                      className={`format-card-pill ${isSelected ? 'is-active' : ''}`}
                      onClick={() => setSelectedFormatId(fmt.id)}
                    >
                      <div className="format-card-left">
                        <span className="format-res-badge">{resText}</span>
                        <span className="format-ext-badge">{fmt.ext.toUpperCase()}</span>
                        {isBest && (
                          <span className="format-best-badge">
                            <Sparkles size={11} />
                            <span>{ar ? 'الأفضل' : 'Best'}</span>
                          </span>
                        )}
                        {fmt.hasVideo && !fmt.hasAudio && !mediaMeta.hasFfmpeg && (
                          <span className="format-video-only-badge">
                            {ar ? 'فيديو فقط' : 'video only'}
                          </span>
                        )}
                      </div>
                      <div className="format-card-right">
                        {fmt.filesize ? (
                          <span className="format-filesize">{formatBytes(fmt.filesize, 1)}</span>
                        ) : null}
                      </div>
                    </button>
                  );
                })}
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
          <div className="inline-error" role="alert" style={{ display: 'flex', flexDirection: 'column', gap: 6, width: '100%' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
              <AlertCircle size={14} style={{ flexShrink: 0 }} />
              <span>{error}</span>
            </div>
            {singleUrl && (error.includes('تسجيل الدخول') || error.includes('cookies') || error.includes('خاص') || error.includes('login') || error.includes('private')) && (
              <div style={{ display: 'flex', gap: 8, marginTop: 4 }}>
                <button
                  type="button"
                  className="action-button compact primary"
                  onClick={() => {
                    void downloadManager.openMediaSniffer(singleUrl);
                  }}
                  style={{ fontSize: 11, padding: '4px 10px', width: 'fit-content' }}
                >
                  {isAndroidRuntime()
                    ? ar ? 'اقتناص الرابط عبر متصفح ARGON' : 'Open in ARGON media sniffer'
                    : ar ? 'فتح الصفحة في المتصفح' : 'Open page in browser'}
                </button>
              </div>
            )}
          </div>
        )}
        {notice && (
          <output className="inline-notice" aria-live="polite">
            <CheckCircle2 size={14} className="status-ok" />
            <span>{notice}</span>
          </output>
        )}
      </form>

      <HuggingFaceModal
        open={showHfModal}
        lang={lang}
        onClose={() => setShowHfModal(false)}
        onQueueFiles={(files) => {
          for (const f of files) {
            onAdd(f.url, f.filename, segments, f.sha256, undefined, queueId);
          }
          setNotice(
            ar
              ? `تمت إضافة ${files.length} ملفات نموذج إلى الطابور`
              : `Queued ${files.length} model files`
          );
        }}
      />
    </section>
  );
}
