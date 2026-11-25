import { useState } from 'react';
import {
  Search,
  ArrowDownToLine,
  ArrowUpRight,
  ArrowDownLeft,
  FolderOpen,
  CheckCircle2
} from 'lucide-react';
import { useFlow } from '../core/useFlow';
import { downloadManager } from '../core/downloadManager';
import { formatBytes } from '../core/segmentation';
import { flowStatus } from './FlowPanels';
import type { DownloadTask } from '../types/downloader';

export function HistoryView({ lang, tasks }: { lang: 'ar' | 'en'; tasks: DownloadTask[] }) {
  const { snapshot } = useFlow();
  const ar = lang === 'ar';
  const [filter, setFilter] = useState('');
  const [actionError, setActionError] = useState('');

  const entries = [
    ...tasks.map((task) => ({
      id: task.id,
      name: task.filename,
      date: task.createdAt,
      kind: 'download' as const,
      peer: '',
      status: task.status,
      path: task.savePath,
      size: task.totalBytes
    })),
    ...(snapshot?.transfers || []).map((transfer) => ({
      id: `flow-${transfer.id}`,
      name: transfer.kind === 'text' ? (ar ? 'نص الحافظة' : 'Clipboard text') : transfer.name,
      date: transfer.createdAt,
      kind: transfer.direction as 'send' | 'receive',
      peer: transfer.peerName,
      status: transfer.status,
      path: transfer.path || undefined,
      size: transfer.total
    }))
  ]
    .sort((a, b) => b.date - a.date)
    .filter((e) => !filter.trim() || e.name.toLowerCase().includes(filter.toLowerCase()));

  return (
    <div>
      <div className="view-header">
        <div className="view-header-title">
          <h1>{ar ? 'سجل العمليات' : 'Activity Log'}</h1>
          <span className="count-badge" dir="ltr">
            {entries.length} {ar ? 'عنصر' : 'items'}
          </span>
        </div>

        <div className="search-field" style={{ maxWidth: 260 }}>
          <Search size={14} />
          <input
            type="text"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder={ar ? 'بحث في السجل…' : 'Filter activity…'}
          />
        </div>
      </div>

      {actionError && (
        <p className="inline-error" role="alert">
          {actionError}
        </p>
      )}
      <div className="glass-panel">
        {entries.length > 0 ? (
          entries.map((entry) => (
            <article key={entry.id} className="history-row">
              <div className="file-symbol">
                {entry.kind === 'download' ? (
                  <ArrowDownToLine size={16} />
                ) : entry.kind === 'send' ? (
                  <ArrowUpRight size={16} />
                ) : (
                  <ArrowDownLeft size={16} />
                )}
              </div>

              <div>
                <strong dir="auto">{entry.name}</strong>
                <span>
                  {entry.kind === 'download'
                    ? ar ? 'تنزيل' : 'Download'
                    : `${entry.kind === 'send' ? (ar ? 'إلى' : 'To') : ar ? 'من' : 'From'} ${entry.peer}`}{' '}
                  · <span dir="ltr">{formatBytes(entry.size)}</span> ·{' '}
                  {new Date(entry.date).toLocaleString(ar ? 'ar-IQ' : 'en', {
                    month: 'short',
                    day: 'numeric',
                    hour: '2-digit',
                    minute: '2-digit'
                  })}
                </span>
              </div>

              <small className={entry.status === 'error' ? 'status-danger' : ''}>
                {entry.kind === 'download'
                  ? {
                      downloading: ar ? 'جارٍ التنزيل' : 'Downloading',
                      paused: ar ? 'متوقف' : 'Paused',
                      queued: ar ? 'في الانتظار' : 'Queued',
                      completed: ar ? 'مكتمل' : 'Complete',
                      error: ar ? 'خطأ' : 'Error'
                    }[entry.status] || entry.status
                  : flowStatus(entry.status, lang)}
              </small>

              {entry.status === 'completed' && entry.path ? (
                <button
                  className="icon-button"
                  onClick={() => {
                    void downloadManager.openFolder(entry.path).catch((error) => setActionError(String(error)));
                  }}
                  aria-label={ar ? 'فتح المجلد' : 'Open folder'}
                >
                  <FolderOpen size={15} />
                </button>
              ) : entry.status === 'completed' ? (
                <CheckCircle2 size={15} className="status-ok" />
              ) : (
                <span />
              )}
            </article>
          ))
        ) : (
          <div className="quiet-empty">
            <p>{ar ? 'لا توجد عمليات مسجلة حتى الآن.' : 'No activity yet.'}</p>
          </div>
        )}
      </div>
    </div>
  );
}
