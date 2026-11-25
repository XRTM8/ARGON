import { ArrowDownToLine, Maximize2, Moon, Pause, Play, Sun } from 'lucide-react';
import { ArgonMark } from './ArgonMark';
import { ThroughputSparkline } from './ThroughputSparkline';
import { downloadManager } from '../core/downloadManager';
import { formatSpeed } from '../core/segmentation';
import type { DownloadStats, DownloadTask } from '../types/downloader';

interface Props {
  lang: 'ar' | 'en';
  theme: 'dark' | 'light';
  tasks: DownloadTask[];
  stats: DownloadStats;
  speedHistory: number[];
  onToggleTheme: () => void;
  onRestore: () => void;
}

export function MiniWidget({
  lang,
  theme,
  tasks,
  stats,
  speedHistory,
  onToggleTheme,
  onRestore
}: Props) {
  const ar = lang === 'ar';
  const activeTasks = tasks.filter((t) => t.status === 'downloading' || t.status === 'queued');
  const firstActive = activeTasks[0];
  const totalProgress =
    firstActive && firstActive.totalBytes > 0
      ? Math.min(100, Math.max(0, Math.round((firstActive.downloadedBytes / firstActive.totalBytes) * 100)))
      : 0;

  return (
    <div className={`argon-root argon-mini-mode ${theme === 'light' ? 'light-mode' : ''}`} dir={ar ? 'rtl' : 'ltr'}>
      <div className="mini-widget">
        <div className="mini-widget-header">
          <div className="mini-brand">
            <ArgonMark size={18} />
            <strong className="mini-title">ARGON</strong>
            <span className="mini-status-chip">
              {stats.activeCount > 0
                ? ar ? `${stats.activeCount} نشط` : `${stats.activeCount} active`
                : ar ? 'جاهز' : 'Ready'}
            </span>
          </div>
          <div className="mini-widget-actions">
            <button
              type="button"
              className="mini-icon-btn"
              onClick={onToggleTheme}
              title={ar ? 'تبديل المظهر' : 'Toggle theme'}
            >
              {theme === 'dark' ? <Sun size={12} /> : <Moon size={12} />}
            </button>
            <button
              type="button"
              className="mini-icon-btn mini-expand-btn"
              onClick={onRestore}
              title={ar ? 'استعادة الواجهة الكاملة' : 'Restore full window'}
            >
              <Maximize2 size={13} />
            </button>
          </div>
        </div>

        <div className="mini-widget-body">
          {firstActive ? (
            <div className="mini-task-info">
              <span className="mini-filename" title={firstActive.filename} dir="ltr">
                {firstActive.filename}
              </span>
              <div className="mini-stats-line">
                <span className="mini-speed" dir="ltr">
                  <ArrowDownToLine size={12} />
                  {formatSpeed(stats.currentTotalSpeed)}
                </span>
                <span className="mini-percentage" dir="ltr">
                  {totalProgress}%
                </span>
              </div>
              <div className="mini-progress-track">
                <div className="mini-progress-fill" style={{ width: `${totalProgress}%` }} />
              </div>
            </div>
          ) : (
            <div className="mini-idle-info">
              <span>{ar ? 'لا توجد تنزيلات نشطة حالياً' : 'No active downloads'}</span>
            </div>
          )}
        </div>

        <div className="mini-widget-footer">
          {stats.activeCount > 0 ? (
            <button
              type="button"
              className="mini-action-pill"
              onClick={() => downloadManager.pauseAll()}
              title={ar ? 'إيقاف مؤقت للكل' : 'Pause all'}
            >
              <Pause size={12} />
              <span>{ar ? 'إيقاف مؤقت' : 'Pause'}</span>
            </button>
          ) : tasks.some((t) => t.status === 'paused') ? (
            <button
              type="button"
              className="mini-action-pill"
              onClick={() => downloadManager.resumeAll()}
              title={ar ? 'استئناف الكل' : 'Resume all'}
            >
              <Play size={12} />
              <span>{ar ? 'استئناف' : 'Resume'}</span>
            </button>
          ) : (
            <button type="button" className="mini-action-pill" onClick={onRestore}>
              <span>{ar ? 'فتح التنزيلات' : 'Open Downloads'}</span>
            </button>
          )}
          {stats.currentTotalSpeed > 0 && (
            <div className="mini-sparkline-wrap">
              <ThroughputSparkline history={speedHistory} width={48} height={14} />
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
