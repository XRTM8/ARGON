import { useEffect, useRef, useState } from 'react';
import {
  Play,
  Pause,
  Trash2,
  FolderOpen,
  Copy,
  Check,
  ChevronDown,
  ExternalLink,
  RotateCcw,
  Send,
  X,
  AlertCircle,
  File,
  ShieldCheck,
  Share2
} from 'lucide-react';
import type { DownloadTask } from '../types/downloader';
import { formatBytes, formatSpeed, formatEta, humanizeErrorMessage } from '../core/segmentation';
import { downloadHost } from '../core/downloadInput';
import { downloadManager } from '../core/downloadManager';
import { isAndroidRuntime } from '../core/safeArea';
import { telemetryStore } from '../core/telemetryStore';

interface Props {
  task: DownloadTask;
  lang: 'ar' | 'en';
  onPause: (id: string) => void;
  onResume: (id: string) => void;
  onRemove: (id: string, disk?: boolean) => void;
  onStartImmediately?: (id: string) => void;
  onRestart?: (id: string) => void;
  isSelected?: boolean;
  isInspected?: boolean;
  onInspect?: (id: string) => void;
  onToggleSelect?: (id: string) => void;
  onShare?: (task: DownloadTask) => void;
}

export function TaskCard({
  task,
  lang,
  onPause,
  onResume,
  onRemove,
  onStartImmediately,
  onRestart,
  isSelected,
  isInspected,
  onInspect,
  onToggleSelect,
  onShare
}: Props) {
  const ar = lang === 'ar';
  const android = isAndroidRuntime();
  const [expanded, setExpanded] = useState(false);
  const [copied, setCopied] = useState(false);
  const [verifying, setVerifying] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [execWarning, setExecWarning] = useState(false);
  const [actionError, setActionError] = useState('');
  const [replacementUrl, setReplacementUrl] = useState('');

  const completed = task.status === 'completed';
  const active = task.status === 'downloading';
  const known = task.totalBytes > 0;
  const ratio = completed ? 1 : known ? Math.min(1, Math.max(0, task.downloadedBytes / task.totalBytes)) : 0;
  const percent = completed ? 100 : known ? Math.min(100, Math.floor(ratio * 100)) : 0;

  const progressBarRef = useRef<HTMLSpanElement | null>(null);
  const bytesRef = useRef<HTMLSpanElement | null>(null);
  const speedRef = useRef<HTMLElement | null>(null);
  const etaRef = useRef<HTMLSpanElement | null>(null);
  const compactRateRef = useRef<HTMLSpanElement | null>(null);

  useEffect(() => {
    if (!active) return;
    telemetryStore.registerTaskDom(task.id, {
      progressBar: progressBarRef.current,
      bytesText: bytesRef.current,
      speedText: speedRef.current,
      etaText: etaRef.current,
      compactRateText: compactRateRef.current
    });
    return () => telemetryStore.unregisterTaskDom(task.id);
  }, [active, task.id]);
  const status = {
    queued: ar ? 'في الانتظار' : 'Queued',
    paused: ar ? 'متوقف مؤقتاً' : 'Paused',
    completed: ar ? 'مكتمل' : 'Completed',
    error: ar ? 'يحتاج انتباهك' : 'Needs attention',
    downloading:
      task.downloadedBytes === 0
        ? ar ? 'جارٍ الاتصال' : 'Connecting'
        : known && task.downloadedBytes >= task.totalBytes
          ? ar ? 'جارٍ التحقق' : 'Finalizing'
          : ar ? 'جارٍ التنزيل' : 'Downloading'
  }[task.status];

  const queueLabel =
    task.queueId === 'night'
      ? ar ? 'ليلي' : 'Night'
      : task.queueId === 'media'
        ? ar ? 'وسائط' : 'Media'
        : '';

  const extension = task.filename.includes('.') ? task.filename.split('.').pop()!.slice(0, 5).toUpperCase() : '';
  const action = async (operation: () => Promise<unknown>) => {
    try {
      setActionError('');
      await operation();
    } catch (error) {
      setActionError(String(error));
    }
  };
  const open = () => {
    if (/\.(exe|bat|cmd|msi|ps1|vbs|reg|scr|com|pif|jar)$/i.test(task.filename)) setExecWarning(true);
    else void action(() => downloadManager.openFile(task.savePath));
  };

  return (
    <article
      className={`download-row ${isSelected ? 'is-selected' : ''} ${isInspected ? 'is-inspected' : ''}`}
      data-status={task.status}
    >
      <div className="download-row-main">
        <input
          type="checkbox"
          className="task-check"
          checked={!!isSelected}
          onChange={() => onToggleSelect?.(task.id)}
          aria-label={`${ar ? 'تحديد' : 'Select'} ${task.filename}`}
        />
        <div className="file-tile" aria-hidden="true">
          {extension || <File size={17} />}
        </div>
        <div
          className="download-identity"
          onClick={() => onInspect?.(task.id)}
        >
          <h3 title={task.filename} dir="auto">
            {task.filename}
          </h3>
          <div className="download-source">
            <span dir="ltr">{downloadHost(task.originalUrl || task.url)}</span>
            {task.mediaInfo?.quality && <span className="queue-pill">{task.mediaInfo.quality}</span>}
            {queueLabel && <span className="queue-pill">{queueLabel}</span>}
          </div>
        </div>

        <div className="download-transfer">
          <span className="task-status">
            <i />
            {status}
          </span>
          <span ref={bytesRef} className="transfer-numbers" dir="ltr">
            {formatBytes(task.downloadedBytes)}
            {known ? ` / ${formatBytes(task.totalBytes)}` : completed ? '' : ` · ${ar ? 'حجم غير معروف' : 'Unknown size'}`}
          </span>
          {active && (
            <span ref={compactRateRef} className="compact-rate" dir="ltr">
              {task.speedBytesPerSec > 0
                ? `${formatSpeed(task.speedBytesPerSec)}${task.etaSeconds > 0 ? ` · ${formatEta(task.etaSeconds)}` : ''}`
                : ''}
            </span>
          )}
        </div>

        <div className="download-rate" dir="ltr">
          <strong ref={speedRef}>
            {active && task.speedBytesPerSec > 0
              ? formatSpeed(task.speedBytesPerSec)
              : completed
                ? '100%'
                : known && task.downloadedBytes > 0
                  ? `${percent}%`
                  : '—'}
          </strong>
          <span ref={etaRef}>{active && task.etaSeconds > 0 ? formatEta(task.etaSeconds) : ''}</span>
        </div>

        <div className="row-actions">
          {active && (
            <button
              className="icon-button"
              onClick={() => onPause(task.id)}
              aria-label={ar ? 'إيقاف مؤقت' : 'Pause'}
              title={ar ? 'إيقاف مؤقت' : 'Pause'}
            >
              <Pause size={16} />
            </button>
          )}
          {(task.status === 'paused' || task.status === 'error') && (
            <button
              className="icon-button"
              onClick={() => onResume(task.id)}
              aria-label={ar ? 'إعادة المحاولة' : 'Resume or retry'}
              title={ar ? 'استئناف / إعادة المحاولة' : 'Resume / retry'}
            >
              <Play size={16} />
            </button>
          )}
          {task.status === 'queued' && onStartImmediately && (
            <button
              className="icon-button"
              onClick={() => onStartImmediately(task.id)}
              aria-label={ar ? 'بدء الآن' : 'Start now'}
              title={ar ? 'بدء الآن' : 'Start now'}
            >
              <Play size={16} />
            </button>
          )}
          {completed && (
            <button
              className="icon-button"
              onClick={open}
              aria-label={ar ? 'فتح الملف' : 'Open file'}
              title={ar ? 'فتح الملف' : 'Open file'}
            >
              <ExternalLink size={16} />
            </button>
          )}
          <button
            className="icon-button details-toggle"
            aria-expanded={expanded}
            aria-controls={`details-${task.id}`}
            onClick={() => setExpanded(!expanded)}
            aria-label={ar ? 'تفاصيل التنزيل' : 'Download details'}
            title={ar ? 'التفاصيل والإجراءات' : 'Details and actions'}
          >
            <ChevronDown size={16} className={expanded ? 'rotate-180' : ''} />
          </button>
        </div>
      </div>

      {!completed && (
        <div
          className="row-progress"
          role="progressbar"
          aria-label={`${task.filename}: ${status}`}
          aria-valuenow={known ? percent : undefined}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuetext={known ? `${percent}%` : status}
        >
          <span ref={progressBarRef} style={{ transform: `scaleX(${ratio})` }} />
        </div>
      )}

      {task.errorMessage && (
        <p className="task-error">
          <AlertCircle size={14} />
          <span>{humanizeErrorMessage(task.errorMessage, lang)}</span>
        </p>
      )}

      {expanded && (
        <div className="task-details" id={`details-${task.id}`}>
          <dl>
            <div>
              <dt>{ar ? 'الرابط' : 'Source'}</dt>
              <dd dir="ltr">{task.originalUrl || task.url}</dd>
            </div>
            {task.savePath && (
              <div>
                <dt>{ar ? 'المسار' : 'Saved to'}</dt>
                <dd dir="ltr">{task.savePath}</dd>
              </div>
            )}
            <div>
              <dt>{ar ? 'الاتصالات' : 'Connections'}</dt>
              <dd>
                {task.probed ? task.chunks.length : ar ? 'تُحدد بعد الفحص' : 'Determined after probe'}
                {task.acceptRanges === false
                  ? ar ? ' · الخادم لا يدعم التقسيم' : ' · Server has no range support'
                  : ''}
              </dd>
            </div>
          </dl>

          {(task.status === 'paused' || task.status === 'error') && (
            <form
              className="refresh-url"
              onSubmit={(event) => {
                event.preventDefault();
                try {
                  downloadManager.refreshTaskUrl(task.id, replacementUrl);
                  setReplacementUrl('');
                  setActionError('');
                } catch (error) {
                  setActionError(String(error));
                }
              }}
            >
              <label htmlFor={`refresh-${task.id}`}>
                {ar ? 'تحديث رابط منتهي الصلاحية' : 'Replace an expired link'}
              </label>
              <div>
                <input
                  id={`refresh-${task.id}`}
                  value={replacementUrl}
                  onChange={(event) => setReplacementUrl(event.target.value)}
                  type="url"
                  required
                  dir="ltr"
                  placeholder="https://example.com/file.zip?new-token"
                />
                <button className="action-button" disabled={!replacementUrl.trim()}>
                  {ar ? 'تحديث واستئناف' : 'Update & resume'}
                </button>
              </div>
              <p>
                {ar
                  ? 'يُستأنف الملف فقط إذا تطابق مساره وحجمه وبصمة الخادم. وإلا يبدأ من الصفر.'
                  : 'Resume requires matching resource path, size and server validator. Otherwise it restarts cleanly.'}
              </p>
            </form>
          )}

          {active && task.chunks.length > 1 && (
            <div className="segment-track" aria-label={ar ? 'تقدم أجزاء الملف' : 'Segment progress'}>
              {task.chunks.map((chunk) => (
                <span key={chunk.index} title={`#${chunk.index + 1}`}>
                  <i
                    style={{
                      width: `${Math.min(100, (chunk.downloaded / Math.max(1, chunk.endByte - chunk.startByte + 1)) * 100)}%`
                    }}
                  />
                </span>
              ))}
            </div>
          )}

          <div className="detail-actions">
            {completed && task.savePath && !android && (
              <button className="action-button" onClick={() => void action(() => downloadManager.openFolder(task.savePath))}>
                <FolderOpen size={14} />
                {ar ? 'عرض في المجلد' : 'Show in folder'}
              </button>
            )}
            {completed && task.savePath && android && (
              <button className="action-button" onClick={() => void action(() => downloadManager.shareFile(task.savePath))}>
                <Share2 size={14} />
                {ar ? 'مشاركة الملف' : 'Share file'}
              </button>
            )}
            {completed && task.savePath && onShare && (
              <button className="action-button" onClick={() => onShare(task)}>
                <Send size={14} />
                {ar ? 'إرسال لجهاز' : 'Send to device'}
              </button>
            )}
            <button
              className="action-button"
              onClick={() =>
                void action(async () => {
                  await navigator.clipboard.writeText(task.originalUrl || task.url);
                  setCopied(true);
                  setTimeout(() => setCopied(false), 1800);
                })
              }
            >
              {copied ? <Check size={14} /> : <Copy size={14} />}
              {ar ? (copied ? 'تم النسخ' : 'نسخ الرابط') : copied ? 'Copied' : 'Copy link'}
            </button>
            {(completed || task.status === 'error') && onRestart && (
              <button className="action-button" onClick={() => onRestart(task.id)}>
                <RotateCcw size={14} />
                {ar ? 'إعادة التنزيل' : 'Download again'}
              </button>
            )}
            <button
              className="action-button danger-text"
              onClick={() => setConfirmDelete(!confirmDelete)}
              aria-expanded={confirmDelete}
            >
              <Trash2 size={14} />
              {ar ? 'إزالة' : 'Remove'}
            </button>
          </div>

          {completed && (
            <div className="checksum-line">
              <ShieldCheck size={14} className="accent-text" />
              {task.checksum?.actual ? (
                <span dir="ltr" className="hash-value">
                  SHA-256: {task.checksum.actual}
                  {task.checksum.verified ? ' · ✓' : ''}
                </span>
              ) : (
                <button
                  className="text-button"
                  disabled={verifying}
                  onClick={async () => {
                    setVerifying(true);
                    await action(async () => {
                      if (!(await downloadManager.verifyHash(task.id))) {
                        throw new Error(
                          ar
                            ? 'تعذّر حساب البصمة. تحقق من وجود الملف وإمكانية قراءته.'
                            : 'Could not calculate the hash. Check that the file exists and can be read.'
                        );
                      }
                    });
                    setVerifying(false);
                  }}
                >
                  {ar
                    ? verifying ? 'جارٍ حساب البصمة…' : 'حساب بصمة SHA-256'
                    : verifying ? 'Calculating…' : 'Calculate SHA-256'}
                </button>
              )}
              {task.checksum?.actual && !task.checksum.expected && (
                <span>{ar ? 'محسوبة؛ لا توجد بصمة للمقارنة.' : 'Calculated; no reference hash to compare.'}</span>
              )}
            </div>
          )}
        </div>
      )}

      {confirmDelete && (
        <div className="row-confirm" role="group" aria-label={ar ? 'خيارات الإزالة' : 'Removal options'}>
          <span>{ar ? 'إزالة من القائمة أم حذف الملف من القرص؟' : 'Remove from the list or delete the file from disk?'}</span>
          <button className="action-button" onClick={() => onRemove(task.id, false)}>
            {ar ? 'من القائمة فقط' : 'List only'}
          </button>
          {task.savePath && (
            <button className="action-button danger-text" onClick={() => onRemove(task.id, true)}>
              {ar ? 'حذف الملف نهائياً' : 'Delete file permanently'}
            </button>
          )}
          <button className="icon-button" aria-label={ar ? 'إلغاء' : 'Cancel'} onClick={() => setConfirmDelete(false)}>
            <X size={14} />
          </button>
        </div>
      )}

      {execWarning && (
        <div className="row-confirm" role="alert">
          <AlertCircle size={16} />
          <span>{ar ? 'هذا ملف تنفيذي. افتحه فقط إذا كنت تثق بمصدره.' : 'This is an executable. Open it only if you trust its source.'}</span>
          <button
            className="action-button"
            onClick={() => {
              setExecWarning(false);
              void action(() => downloadManager.openFile(task.savePath));
            }}
          >
            {ar ? 'فتح الملف' : 'Open file'}
          </button>
          <button className="action-button" onClick={() => setExecWarning(false)}>
            {ar ? 'إلغاء' : 'Cancel'}
          </button>
        </div>
      )}

      {actionError && (
        <p className="inline-error" role="alert" style={{ paddingInline: 16, paddingBottom: 12 }}>
          {actionError}
        </p>
      )}
    </article>
  );
}
