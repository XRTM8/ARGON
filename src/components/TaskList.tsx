import { memo, useState } from 'react';
import { Search, X, ArrowDownToLine, CheckCircle2, Pause, Play, Trash2, Filter, Copy, Check } from 'lucide-react';
import type { DownloadTask } from '../types/downloader';
import { TaskCard } from './TaskCard';
import { downloadManager } from '../core/downloadManager';

interface Props {
  tasks: DownloadTask[];
  lang: 'ar' | 'en';
  mode: 'downloads' | 'finished';
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
  mode,
  inspectedId,
  onInspect,
  onPause,
  onResume,
  onRemove,
  onStartImmediately,
  onRestart,
  onSwitchToDownloads,
  onShare
}: Props) {
  const ar = lang === 'ar';
  const [query, setQuery] = useState('');
  const [status, setStatus] = useState('all');
  const [category, setCategory] = useState('all');
  const [queue, setQueue] = useState('all');
  const [sort, setSort] = useState('newest');
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [exported, setExported] = useState(false);

  const inMode = tasks.filter((task) => (task.status === 'completed') === (mode === 'finished'));
  const filtered = inMode
    .filter(
      (task) =>
        (status === 'all' || mode === 'finished' || task.status === status) &&
        (category === 'all' || task.category === category) &&
        (queue === 'all' || (task.queueId || 'main') === queue) &&
        (!query.trim() ||
          `${task.filename} ${task.originalUrl || task.url}`.toLowerCase().includes(query.trim().toLowerCase()))
    )
    .sort((a, b) =>
      sort === 'name'
        ? a.filename.localeCompare(b.filename, lang)
        : sort === 'size'
          ? b.totalBytes - a.totalBytes
          : sort === 'oldest'
            ? a.createdAt - b.createdAt
            : b.createdAt - a.createdAt
    );

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

  const labels: Record<string, string> = ar
    ? { all: 'كل الحالات', downloading: 'جارٍ التنزيل', queued: 'في الانتظار', paused: 'متوقف مؤقتاً', error: 'يحتاج انتباهك' }
    : { all: 'All statuses', downloading: 'Downloading', queued: 'Queued', paused: 'Paused', error: 'Needs attention' };

  return (
    <section className="download-list" aria-label={ar ? 'قائمة التنزيلات' : 'Download list'}>
      <div className="list-tools">
        <div className="search-field">
          <Search size={15} />
          <input
            id="download-search"
            aria-label={ar ? 'بحث في التنزيلات' : 'Search downloads'}
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={ar ? 'بحث باسم الملف أو الموقع…' : 'Search files or websites…'}
          />
          {query && (
            <button className="icon-button" style={{ width: 28, height: 28 }} onClick={() => setQuery('')} aria-label={ar ? 'مسح البحث' : 'Clear search'}>
              <X size={13} />
            </button>
          )}
        </div>

        <div className="list-filters">
          <Filter size={14} aria-hidden="true" />
          {mode === 'downloads' && (
            <select aria-label={ar ? 'تصفية حسب الحالة' : 'Filter by status'} value={status} onChange={(event) => setStatus(event.target.value)}>
              {Object.entries(labels).map(([value, label]) => (
                <option key={value} value={value}>
                  {label}
                </option>
              ))}
            </select>
          )}
          <select aria-label={ar ? 'نوع الملف' : 'File type'} value={category} onChange={(event) => setCategory(event.target.value)}>
            {[
              ['all', ar ? 'كل الأنواع' : 'All types'],
              ['programs', ar ? 'برامج' : 'Apps'],
              ['archives', ar ? 'أرشيف' : 'Archives'],
              ['media', ar ? 'وسائط' : 'Media'],
              ['documents', ar ? 'مستندات' : 'Documents'],
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
          <select aria-label={ar ? 'ترتيب التنزيلات' : 'Sort downloads'} value={sort} onChange={(event) => setSort(event.target.value)}>
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
          {inMode.length > 0 && (
            <button
              type="button"
              className="text-button"
              onClick={() => {
                const text = downloadManager.exportUrls(mode === 'finished' ? 'completed' : 'active');
                void navigator.clipboard.writeText(text).then(() => {
                  setExported(true);
                  setTimeout(() => setExported(false), 1600);
                }).catch(() => {});
              }}
              title={ar ? 'نسخ جميع الروابط' : 'Copy all links'}
            >
              {exported ? <Check size={13} className="status-ok" /> : <Copy size={13} />}
              <span>{exported ? (ar ? 'نُسخت' : 'Copied') : ar ? 'تصدير' : 'Export'}</span>
            </button>
          )}
        </div>
      </div>

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
            {ar ? 'إزالة من القائمة' : 'Remove from list'}
          </button>
          <button
            className="text-button"
            onClick={() => {
              setSelected(new Set());
              setConfirmRemove(false);
            }}
          >
            {ar ? 'إلغاء التحديد' : 'Deselect'}
          </button>
          {confirmRemove && (
            <div className="selection-confirm">
              <span>{ar ? 'ستبقى الملفات على القرص.' : 'Files will stay on disk.'}</span>
              <button
                className="action-button"
                onClick={() => {
                  selection.forEach((task) => onRemove(task.id, false));
                  setSelected(new Set());
                  setConfirmRemove(false);
                }}
              >
                {ar ? 'تأكيد الإزالة' : 'Confirm removal'}
              </button>
            </div>
          )}
        </div>
      )}

      {filtered.length > 0 ? (
        <div className="download-table">
          <div className="list-column-head">
            <label>
              <input
                type="checkbox"
                checked={allSelected}
                onChange={() => {
                  setSelected(allSelected ? new Set() : new Set(filtered.map((task) => task.id)));
                  setConfirmRemove(false);
                }}
                aria-label={ar ? 'تحديد كل النتائج' : 'Select all results'}
              />
              <span>{ar ? 'الملف' : 'File'}</span>
            </label>
            <span>{ar ? 'الحالة والحجم' : 'Status & size'}</span>
            <span>{ar ? 'السرعة / التقدم' : 'Speed / progress'}</span>
            <span />
          </div>
          {filtered.map((task) => (
            <TaskCard
              key={task.id}
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
          ))}
          <div className="list-count">
            {filtered.length} / {inMode.length} {ar ? 'ملفات' : 'files'}
          </div>
        </div>
      ) : (
        <div className="download-empty">
          <div className="empty-file" aria-hidden="true">
            {mode === 'finished' ? <CheckCircle2 size={28} /> : <ArrowDownToLine size={28} />}
          </div>
          <h2>
            {inMode.length
              ? ar ? 'لا توجد نتائج مطابقة' : 'No matching downloads'
              : mode === 'finished'
                ? ar ? 'ملفاتك المكتملة ستظهر هنا' : 'Your finished files will appear here'
                : ar ? 'كل شيء جاهز لتنزيلك الأول' : 'Ready for your first download'}
          </h2>
          <p>
            {inMode.length
              ? ar ? 'غيّر البحث أو التصفية لعرض المزيد من الملفات.' : 'Change your search or filters to see more files.'
              : mode === 'finished'
                ? ar ? 'افتح الملفات، اعرض مجلدها، أو أرسلها إلى جهاز آخر.' : 'Open a file, find its folder, or send it to another device.'
                : ar ? 'أضف رابط ملف بالأعلى، أو استخدم الأنماط المتسلسلة والدفعات.' : 'Add a URL above, or use batch and pattern downloads.'}
          </p>
          {inMode.length ? (
            <button
              className="action-button"
              onClick={() => {
                setQuery('');
                setStatus('all');
                setCategory('all');
                setQueue('all');
              }}
            >
              {ar ? 'مسح التصفية' : 'Clear filters'}
            </button>
          ) : mode === 'finished' && onSwitchToDownloads ? (
            <button className="action-button" onClick={onSwitchToDownloads}>
              {ar ? 'إضافة تنزيل' : 'Add a download'}
            </button>
          ) : (
            <button className="action-button primary" onClick={() => document.getElementById('download-url')?.focus()}>
              <ArrowDownToLine size={15} />
              {ar ? 'إضافة رابط' : 'Add a link'}
            </button>
          )}
          {!inMode.length && mode === 'downloads' && (
            <div className="empty-shortcuts">
              <span>
                <kbd>Ctrl</kbd> + <kbd>V</kbd> {ar ? 'لصق رابط' : 'Paste a link'}
              </span>
              <span>
                <kbd>Ctrl</kbd> + <kbd>K</kbd> {ar ? 'الأوامر' : 'Command palette'}
              </span>
            </div>
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
