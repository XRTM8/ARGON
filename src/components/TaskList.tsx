import { memo, useState, useEffect, useRef } from 'react';
import { Search, X, ArrowDownToLine, CheckCircle2, Pause, Play, Trash2, SlidersHorizontal, Copy, Check, CheckSquare } from 'lucide-react';
import type { DownloadTask } from '../types/downloader';
import { TaskCard } from './TaskCard';
import { downloadManager } from '../core/downloadManager';
import { writeClipboardText } from '../core/clipboard';

interface Props {
  tasks: DownloadTask[];
  lang: 'ar' | 'en';
  mode?: 'all' | 'downloads' | 'finished';
  inspectedId?: string | null;
  onInspect?: (id: string) => void;
  onPause: (id: string) => void;
  onResume: (id: string) => void;
  onRemove: (id: string, disk?: boolean) => void;
  onClearCompleted: () => void;
  onStartImmediately?: (id: string) => void;
  onRestart?: (id: string) => void;
  onPauseAll?: () => void;
  onResumeAll?: () => void;
  onSwitchToDownloads?: () => void;
  onShare?: (task: DownloadTask) => void;
}

export const TaskList = memo(function TaskList({
  tasks,
  lang,
  mode = 'all',
  inspectedId,
  onInspect,
  onPause,
  onResume,
  onRemove,
  onStartImmediately,
  onRestart,
  onShare
}: Props) {
  const ar = lang === 'ar';
  const [query, setQuery] = useState('');
  const [status, setStatus] = useState('all');
  const [category, setCategory] = useState('all');
  const [queue, setQueue] = useState('all');
  const [sort, setSort] = useState('newest');
  const [showFilters, setShowFilters] = useState(false);
  const [selectMode, setSelectMode] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [exported, setExported] = useState(false);

  const inMode = mode === 'all'
    ? tasks
    : tasks.filter((task) => (task.status === 'completed') === (mode === 'finished'));
  const hasActiveFilter = status !== 'all' || category !== 'all' || queue !== 'all' || sort !== 'newest';

  const labels: Record<string, string> = ar
    ? {
        all: 'كل الحالات',
        downloading: 'جارٍ التنزيل',
        queued: 'في الانتظار',
        paused: 'متوقف',
        completed: 'مكتمل',
        error: 'يحتاج انتباهك'
      }
    : {
        all: 'All statuses',
        downloading: 'Downloading',
        queued: 'Queued',
        paused: 'Paused',
        completed: 'Completed',
        error: 'Needs attention'
      };

  const filtered = inMode
    .filter(
      (task) =>
        (status === 'all' || task.status === status) &&
        (category === 'all' || task.category === category) &&
        (queue === 'all' || (task.queueId || 'main') === queue) &&
        (!query.trim() ||
          `${task.filename} ${task.originalUrl || task.url}`.toLowerCase().includes(query.trim().toLowerCase()))
    )
    .sort((a, b) => {
      if (sort === 'name') return a.filename.localeCompare(b.filename, lang);
      if (sort === 'size') return b.totalBytes - a.totalBytes;
      if (sort === 'oldest') return a.createdAt - b.createdAt;

      // Default: 'newest'
      // When in 'all' mode, keep active (non-completed) downloads pinned to top
      if (mode === 'all') {
        const aCompleted = a.status === 'completed';
        const bCompleted = b.status === 'completed';
        if (aCompleted !== bCompleted) {
          return aCompleted ? 1 : -1;
        }
        if (!aCompleted && !bCompleted) {
          if (a.status === 'downloading' && b.status !== 'downloading') return -1;
          if (b.status === 'downloading' && a.status !== 'downloading') return 1;
          if (a.status === 'queued' && b.status === 'queued') {
            const pA = a.queuePriority ?? 0;
            const pB = b.queuePriority ?? 0;
            if (pA !== pB) return pB - pA;
            return a.createdAt - b.createdAt;
          }
        }
      }

      if (a.status === 'queued' && b.status === 'queued') {
        const pA = a.queuePriority ?? 0;
        const pB = b.queuePriority ?? 0;
        if (pA !== pB) return pB - pA;
        return a.createdAt - b.createdAt;
      }

      return b.createdAt - a.createdAt;
    });

  const [visibleLimit, setVisibleLimit] = useState(50);
  const sentinelRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    setVisibleLimit(50);
  }, [query, status, category, queue, sort, mode]);

  useEffect(() => {
    if (visibleLimit >= filtered.length) return;
    const el = sentinelRef.current;
    if (!el || typeof IntersectionObserver === 'undefined') return;

    const observer = new IntersectionObserver(
      (entries) => {
        if (entries[0]?.isIntersecting) {
          setVisibleLimit((prev) => prev + 50);
        }
      },
      { rootMargin: '300px' }
    );

    observer.observe(el);
    return () => observer.disconnect();
  }, [visibleLimit, filtered.length]);

  const visibleTasks = filtered.slice(0, visibleLimit);

  const selection = filtered.filter((task) => selected.has(task.id));
  const allSelected = filtered.length > 0 && selection.length === filtered.length;
  const toggle = (id: string) => {
    setConfirmRemove(false);
    setSelected((previous) => {
      const next = new Set(previous);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  return (
    <section className="download-list" aria-label={ar ? 'قائمة التنزيلات' : 'Download list'}>
      {inMode.length > 0 && (
        <div className="list-tools">
          <div className="list-search-row">
            <div className="search-field">
              <Search size={14} />
              <input
                id="download-search"
                aria-label={ar ? 'بحث في التنزيلات' : 'Search downloads'}
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                placeholder={ar ? 'بحث…' : 'Search…'}
              />
              {query && (
                <button
                  className="icon-button"
                  style={{ width: 26, height: 26 }}
                  onClick={() => setQuery('')}
                  aria-label={ar ? 'مسح البحث' : 'Clear search'}
                >
                  <X size={13} />
                </button>
              )}
            </div>

            <button
              type="button"
              className={`action-button filter-toggle ${showFilters || hasActiveFilter ? 'is-active' : ''}`}
              aria-expanded={showFilters}
              onClick={() => setShowFilters((v) => !v)}
              title={ar ? 'تصفية وترتيب' : 'Filter & sort'}
            >
              <SlidersHorizontal size={14} />
              <span>{ar ? 'تصفية' : 'Filter'}</span>
            </button>

            <button
              type="button"
              className={`action-button filter-toggle ${selectMode ? 'is-active' : ''}`}
              aria-pressed={selectMode}
              onClick={() => {
                setSelectMode((v) => {
                  if (v) {
                    setSelected(new Set());
                    setConfirmRemove(false);
                  }
                  return !v;
                });
              }}
              title={ar ? 'تحديد متعدد' : 'Select items'}
            >
              <CheckSquare size={14} />
              <span>{ar ? 'تحديد' : 'Select'}</span>
            </button>
          </div>

          {showFilters && (
            <div className="list-filters">
              {(mode === 'all' || mode === 'downloads') && (
                <select aria-label={ar ? 'الحالة' : 'Status'} value={status} onChange={(event) => setStatus(event.target.value)}>
                  {Object.entries(labels).map(([value, label]) => (
                    <option key={value} value={value}>
                      {label}
                    </option>
                  ))}
                </select>
              )}
              <select aria-label={ar ? 'النوع' : 'Type'} value={category} onChange={(event) => setCategory(event.target.value)}>
                {[
                  ['all', ar ? 'كل الأنواع' : 'All types'],
                  ['programs', ar ? 'برامج' : 'Apps'],
                  ['archives', ar ? 'أرشيف' : 'Archives'],
                  ['media', ar ? 'وسائط' : 'Media'],
                  ['documents', ar ? 'مستندات' : 'Docs'],
                  ['torrents', ar ? 'تورنت' : 'Torrents'],
                  ['other', ar ? 'أخرى' : 'Other']
                ].map(([value, label]) => (
                  <option key={value} value={value}>
                    {label}
                  </option>
                ))}
              </select>
              <select aria-label={ar ? 'الطابور' : 'Queue'} value={queue} onChange={(event) => setQueue(event.target.value)}>
                <option value="all">{ar ? 'كل الطوابير' : 'All queues'}</option>
                <option value="main">{ar ? 'رئيسي' : 'Main'}</option>
                <option value="night">{ar ? 'ليلي' : 'Night'}</option>
                <option value="media">{ar ? 'وسائط' : 'Media'}</option>
              </select>
              <select aria-label={ar ? 'الترتيب' : 'Sort'} value={sort} onChange={(event) => setSort(event.target.value)}>
                {[
                  ['newest', ar ? 'الأحدث' : 'Newest'],
                  ['oldest', ar ? 'الأقدم' : 'Oldest'],
                  ['name', ar ? 'الاسم' : 'Name'],
                  ['size', ar ? 'الحجم' : 'Size']
                ].map(([value, label]) => (
                  <option key={value} value={value}>
                    {label}
                  </option>
                ))}
              </select>
              <button
                type="button"
                className="text-button"
                onClick={() => {
                  const text = downloadManager.exportUrls(mode === 'finished' ? 'completed' : 'active');
                  void writeClipboardText(text)
                    .then(() => {
                      setExported(true);
                      setTimeout(() => setExported(false), 1600);
                    })
                    .catch(() => {});
                }}
              >
                {exported ? <Check size={13} className="status-ok" /> : <Copy size={13} />}
                <span>{exported ? (ar ? 'نُسخت' : 'Copied') : ar ? 'نسخ الروابط' : 'Copy URLs'}</span>
              </button>
            </div>
          )}
        </div>
      )}

      {selection.length > 0 && (
        <div className="selection-bar" role="group" aria-label={ar ? 'إجراءات المحددة' : 'Selected download actions'}>
          <span>
            {selection.length} {ar ? 'محددة' : 'selected'}
          </span>
          {selection.some((task) => ['downloading', 'queued'].includes(task.status)) && (
            <button className="text-button" onClick={() => selection.forEach((task) => onPause(task.id))}>
              <Pause size={14} />
              {ar ? 'إيقاف مؤقت' : 'Pause'}
            </button>
          )}
          {selection.some((task) => ['paused', 'error', 'queued'].includes(task.status)) && (
            <button className="text-button" onClick={() => selection.forEach((task) => onResume(task.id))}>
              <Play size={14} />
              {ar ? 'استئناف' : 'Resume'}
            </button>
          )}
          <button className="text-button danger-text" onClick={() => setConfirmRemove(!confirmRemove)}>
            <Trash2 size={14} />
            {ar ? 'إزالة' : 'Remove'}
          </button>
          <button
            className="text-button"
            onClick={() => {
              setSelected(new Set());
              setConfirmRemove(false);
            }}
          >
            {ar ? 'إلغاء' : 'Clear'}
          </button>
          {confirmRemove && (
            <div className="selection-confirm">
              <span>{ar ? 'ستبقى الملفات على القرص.' : 'Files stay on disk.'}</span>
              <button
                className="action-button"
                onClick={() => {
                  selection.forEach((task) => onRemove(task.id, false));
                  setSelected(new Set());
                  setConfirmRemove(false);
                }}
              >
                {ar ? 'تأكيد الإزالة' : 'Confirm'}
              </button>
            </div>
          )}
        </div>
      )}

      {filtered.length > 0 ? (
        <div className={`download-table ${selectMode ? 'is-select-mode' : ''}`}>
          {selectMode && (
            <div className="list-column-head">
              <label>
                <input
                  type="checkbox"
                  checked={allSelected}
                  onChange={() => {
                    setSelected(allSelected ? new Set() : new Set(filtered.map((task) => task.id)));
                    setConfirmRemove(false);
                  }}
                  aria-label={ar ? 'تحديد الكل' : 'Select all'}
                />
                <span>{ar ? 'تحديد الكل' : 'Select all'}</span>
              </label>
              <span />
              <span />
              <span />
            </div>
          )}
          {visibleTasks.map((task, index) => {
            const showCompletedDivider =
              mode === 'all' &&
              task.status === 'completed' &&
              index > 0 &&
              visibleTasks[index - 1].status !== 'completed';

            return (
              <div key={task.id} style={{ display: 'contents' }}>
                {showCompletedDivider && (
                  <div className="list-section-divider">
                    <CheckCircle2 size={13} />
                    <span>{ar ? 'التنزيلات المكتملة' : 'Completed downloads'}</span>
                  </div>
                )}
                <TaskCard
                  task={task}
                  lang={lang}
                  onPause={onPause}
                  onResume={onResume}
                  onRemove={onRemove}
                  onStartImmediately={onStartImmediately}
                  onRestart={onRestart}
                  onShare={onShare}
                  isSelected={selected.has(task.id)}
                  isInspected={inspectedId === task.id}
                  onInspect={onInspect}
                  onToggleSelect={toggle}
                />
              </div>
            );
          })}
          {visibleLimit < filtered.length && (
            <div ref={sentinelRef} style={{ display: 'flex', justifyContent: 'center', padding: '16px' }}>
              <button
                type="button"
                className="action-button"
                onClick={() => setVisibleLimit((prev) => prev + 50)}
              >
                {ar
                  ? `عرض المزيد (${filtered.length - visibleLimit} متبقية)`
                  : `Show more (${filtered.length - visibleLimit} remaining)`}
              </button>
            </div>
          )}
        </div>
      ) : (
        <div className="download-empty">
          <div className="empty-file" aria-hidden="true">
            {mode === 'finished' ? <CheckCircle2 size={24} /> : <ArrowDownToLine size={24} />}
          </div>
          <h2>
            {inMode.length > 0
              ? ar ? 'لا توجد نتائج مطابقة' : 'No matching downloads'
              : mode === 'finished'
                ? ar ? 'لا توجد تنزيلات مكتملة' : 'No completed downloads yet'
                : ar ? 'لا توجد تنزيلات حالياً' : 'No active downloads'}
          </h2>
          <p className="empty-subtitle">
            {inMode.length > 0
              ? ar ? 'جرب تغيير خيارات التصفية أو البحث' : 'Try changing your search or filter criteria'
              : ar ? 'أدخل رابطاً في الأعلى أو الصقه ليبدأ التنزيل تلقائياً' : 'Enter a link above or paste a URL to begin downloading'}
          </p>
          {hasActiveFilter && (
            <button
              className="action-button"
              style={{ marginTop: 12 }}
              onClick={() => {
                setQuery('');
                setStatus('all');
                setCategory('all');
                setQueue('all');
                setSort('newest');
              }}
            >
              {ar ? 'مسح التصفية' : 'Clear filters'}
            </button>
          )}
        </div>
      )}
    </section>
  );
}, (prev, next) => (
  prev.tasks === next.tasks
  && prev.lang === next.lang
  && prev.mode === next.mode
  && prev.inspectedId === next.inspectedId
));
