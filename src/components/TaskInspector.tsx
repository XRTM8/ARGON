import { useState } from 'react';
import {
  X,
  FolderOpen,
  Copy,
  Check,
  ShieldCheck,
  Gauge,
  Layers,
  Share2,
  ExternalLink
} from 'lucide-react';
import type { DownloadTask, QueueId } from '../types/downloader';
import { formatBytes, formatSpeed, formatEta } from '../core/segmentation';
import { downloadManager } from '../core/downloadManager';
import { isAndroidRuntime } from '../core/safeArea';

interface Props {
  task: DownloadTask;
  lang: 'ar' | 'en';
  onClose: () => void;
}

export function TaskInspector({ task, lang, onClose }: Props) {
  const ar = lang === 'ar';
  const android = isAndroidRuntime();
  const [copied, setCopied] = useState(false);
  const [verifying, setVerifying] = useState(false);
  const [actionError, setActionError] = useState('');
  const [replacementUrl, setReplacementUrl] = useState('');

  const known = task.totalBytes > 0;
  const completed = task.status === 'completed';
  const pct = completed
    ? 100
    : known
      ? Math.min(100, Math.round((task.downloadedBytes / task.totalBytes) * 100))
      : 0;

  return (
    <aside className="task-inspector" aria-label={ar ? 'فاحص المهمة' : 'Task inspector'}>
      <div className="inspector-head">
        <div>
          <h2 dir="auto">{task.filename}</h2>
          <span>
            {formatBytes(task.downloadedBytes)}
            {known ? ` / ${formatBytes(task.totalBytes)} (${pct}%)` : ''}
            {task.speedBytesPerSec > 0 ? ` · ${formatSpeed(task.speedBytesPerSec)}` : ''}
            {task.etaSeconds > 0 ? ` · ${formatEta(task.etaSeconds)}` : ''}
          </span>
        </div>
        <button className="icon-button" onClick={onClose} aria-label={ar ? 'إغلاق الفاحص' : 'Close inspector'}>
          <X size={15} />
        </button>
      </div>

      {/* Live Segment Connection Matrix */}
      <div>
        <div className="section-title" style={{ marginBottom: 8 }}>
          <span style={{ display: 'flex', alignItems: 'center', gap: 6, fontWeight: 600 }}>
            <Layers size={14} className="accent-text" />
            {ar ? 'خريطة المسارات الحية' : 'Live Segment Map'}
          </span>
          <span dir="ltr">{task.chunks.length} conn</span>
        </div>
        <div className="segment-matrix">
          {task.chunks.slice(0, 16).map((chunk) => {
            const span = Math.max(1, chunk.endByte - chunk.startByte + 1);
            const segPct = chunk.status === 'completed' ? 100 : Math.min(100, Math.round((chunk.downloaded / span) * 100));
            return (
              <div key={chunk.index} className="segment-row" dir="ltr">
                <span>#{chunk.index + 1}</span>
                <div className="segment-bar">
                  <i style={{ width: `${segPct}%` }} />
                </div>
                <span style={{ textAlign: 'end' }}>{segPct}%</span>
              </div>
            );
          })}
        </div>
      </div>

      {/* Queue & Per-Task Speed Cap */}
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10 }}>
        <div className="inspector-field">
          <label htmlFor={`ins-q-${task.id}`}>{ar ? 'الطابور' : 'Queue'}</label>
          <select
            id={`ins-q-${task.id}`}
            className="argon-select"
            value={task.queueId || 'main'}
            onChange={(e) => downloadManager.setTaskQueue(task.id, e.target.value as QueueId)}
          >
            <option value="main">{ar ? 'رئيسي' : 'Main'}</option>
            <option value="night">{ar ? 'ليلي' : 'Night'}</option>
            <option value="media">{ar ? 'وسائط' : 'Media'}</option>
          </select>
        </div>

        <div className="inspector-field">
          <label htmlFor={`ins-spd-${task.id}`}>
            <Gauge size={12} style={{ display: 'inline', marginInlineEnd: 4 }} />
            {ar ? 'سقف سرعة المهمة' : 'Task speed cap'}
          </label>
          <select
            id={`ins-spd-${task.id}`}
            className="argon-select"
            value={task.speedLimitBytesPerSec || 0}
            onChange={(e) => void downloadManager.setTaskSpeedLimit(task.id, Number(e.target.value))}
          >
            <option value={0}>{ar ? 'غير محدود' : 'Unlimited'}</option>
            <option value={512 * 1024}>512 KB/s</option>
            <option value={1024 * 1024}>1 MB/s</option>
            <option value={2 * 1024 * 1024}>2 MB/s</option>
            <option value={5 * 1024 * 1024}>5 MB/s</option>
          </select>
        </div>
      </div>

      {/* Metadata */}
      <dl style={{ display: 'grid', gap: 7, margin: 0, fontSize: 11.5 }}>
        <div style={{ display: 'grid', gridTemplateColumns: '76px minmax(0, 1fr)', gap: 8 }}>
          <dt style={{ color: 'var(--argon-subtle)' }}>{ar ? 'النطاقات' : 'Ranges'}</dt>
          <dd style={{ margin: 0 }}>
            {task.probed
              ? task.acceptRanges
                ? ar ? 'مدعوم (متعدد)' : 'Supported (Segmented)'
                : ar ? 'اتصال واحد فقط' : 'Single-stream only'
              : ar ? 'قيد الفحص' : 'Pending probe'}
          </dd>
        </div>
        {task.etag && (
          <div style={{ display: 'grid', gridTemplateColumns: '76px minmax(0, 1fr)', gap: 8 }}>
            <dt style={{ color: 'var(--argon-subtle)' }}>ETag</dt>
            <dd style={{ margin: 0, fontFamily: 'monospace', overflowWrap: 'anywhere' }} dir="ltr">
              {task.etag}
            </dd>
          </div>
        )}
        {task.savePath && (
          <div style={{ display: 'grid', gridTemplateColumns: '76px minmax(0, 1fr)', gap: 8 }}>
            <dt style={{ color: 'var(--argon-subtle)' }}>{ar ? 'المسار' : 'Path'}</dt>
            <dd style={{ margin: 0, fontFamily: 'monospace', overflowWrap: 'anywhere' }} dir="ltr">
              {task.savePath}
            </dd>
          </div>
        )}
      </dl>

      {/* Replace expired URL */}
      {(task.status === 'paused' || task.status === 'error') && (
        <form
          className="refresh-url"
          style={{ marginBottom: 0, paddingBottom: 12 }}
          onSubmit={(e) => {
            e.preventDefault();
            try {
              downloadManager.refreshTaskUrl(task.id, replacementUrl);
              setReplacementUrl('');
              setActionError('');
            } catch (err) {
              setActionError(String(err));
            }
          }}
        >
          <label htmlFor={`ins-ref-${task.id}`}>{ar ? 'استبدال رابط منتهٍ' : 'Replace expired URL'}</label>
          <div>
            <input
              id={`ins-ref-${task.id}`}
              type="url"
              dir="ltr"
              required
              value={replacementUrl}
              onChange={(e) => setReplacementUrl(e.target.value)}
              placeholder="https://example.com/file.zip?new-token"
            />
            <button className="action-button" disabled={!replacementUrl.trim()}>
              {ar ? 'استئناف' : 'Resume'}
            </button>
          </div>
        </form>
      )}

      {/* Actions */}
      <div className="detail-actions">
        <button
          className="action-button"
          onClick={() => {
            void navigator.clipboard.writeText(task.originalUrl || task.url).then(() => {
              setCopied(true);
              setTimeout(() => setCopied(false), 1600);
            });
          }}
        >
          {copied ? <Check size={14} /> : <Copy size={14} />}
          <span>{copied ? (ar ? 'تم النسخ' : 'Copied') : ar ? 'نسخ الرابط' : 'Copy link'}</span>
        </button>

        {completed && task.savePath && !android && (
          <button className="action-button" onClick={() => void downloadManager.openFolder(task.savePath).catch((e) => setActionError(String(e)))}>
            <FolderOpen size={14} />
            <span>{ar ? 'فتح المجلد' : 'Open folder'}</span>
          </button>
        )}

        {completed && task.savePath && android && (
          <>
            <button className="action-button" onClick={() => void downloadManager.openFile(task.savePath).catch((e) => setActionError(String(e)))}>
              <ExternalLink size={14} />
              <span>{ar ? 'فتح' : 'Open'}</span>
            </button>
            <button className="action-button" onClick={() => void downloadManager.shareFile(task.savePath).catch((e) => setActionError(String(e)))}>
              <Share2 size={14} />
              <span>{ar ? 'مشاركة' : 'Share'}</span>
            </button>
          </>
        )}
      </div>

      {/* SHA-256 Checksum */}
      {completed && (
        <div className="checksum-line" style={{ marginTop: 2 }}>
          <ShieldCheck size={14} className="accent-text" />
          {task.checksum?.actual ? (
            <span dir="ltr" className="hash-value">
              SHA-256: {task.checksum.actual.slice(0, 24)}… {task.checksum.verified ? '· Verified' : ''}
            </span>
          ) : (
            <button
              className="text-button"
              disabled={verifying}
              onClick={async () => {
                setVerifying(true);
                await downloadManager.verifyHash(task.id);
                setVerifying(false);
              }}
            >
              {verifying ? (ar ? 'جارٍ الحساب…' : 'Calculating…') : ar ? 'فحص بصمة SHA-256' : 'Verify SHA-256'}
            </button>
          )}
        </div>
      )}

      {actionError && (
        <p className="inline-error" role="alert">
          {actionError}
        </p>
      )}
    </aside>
  );
}
