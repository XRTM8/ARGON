import { useState } from 'react';
import { Search, X, ArrowDownToLine, CheckCircle2, Pause, Play, Trash2, Filter } from 'lucide-react';
import type { DownloadTask } from '../types/downloader';
import { TaskCard } from './TaskCard';

interface Props {
  tasks: DownloadTask[]; lang: 'ar' | 'en'; mode: 'downloads' | 'finished';
  onPause: (id: string) => void; onResume: (id: string) => void; onRemove: (id: string, disk?: boolean) => void;
  onClearCompleted: () => void; onStartImmediately?: (id: string) => void; onRestart?: (id: string) => void;
  onPauseAll?: () => void; onResumeAll?: () => void; onSwitchToDownloads?: () => void; onShare?: (task: DownloadTask) => void;
}

export function TaskList({ tasks, lang, mode, onPause, onResume, onRemove, onStartImmediately, onRestart, onSwitchToDownloads, onShare }: Props) {
  const ar = lang === 'ar';
  const [query, setQuery] = useState('');
  const [status, setStatus] = useState('all');
  const [category, setCategory] = useState('all');
  const [sort, setSort] = useState('newest');
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [confirmRemove, setConfirmRemove] = useState(false);
  const inMode = tasks.filter((task) => (task.status === 'completed') === (mode === 'finished'));
  const filtered = inMode.filter((task) => (status === 'all' || mode === 'finished' || task.status === status) && (category === 'all' || task.category === category) && (!query.trim() || `${task.filename} ${task.originalUrl || task.url}`.toLowerCase().includes(query.trim().toLowerCase())))
    .sort((a, b) => sort === 'name' ? a.filename.localeCompare(b.filename, lang) : sort === 'size' ? b.totalBytes - a.totalBytes : sort === 'oldest' ? a.createdAt - b.createdAt : b.createdAt - a.createdAt);
  const selection = filtered.filter((task) => selected.has(task.id));
  const allSelected = filtered.length > 0 && selection.length === filtered.length;
  const toggle = (id: string) => { setConfirmRemove(false); setSelected((previous) => { const next = new Set(previous); if (next.has(id)) next.delete(id); else next.add(id); return next; }); };
  const labels: Record<string, string> = ar ? { all: 'كل الحالات', downloading: 'جارٍ التنزيل', queued: 'في الانتظار', paused: 'متوقف مؤقتاً', error: 'يحتاج انتباهك' } : { all: 'All statuses', downloading: 'Downloading', queued: 'Queued', paused: 'Paused', error: 'Needs attention' };

  return <section className="download-list" aria-label={ar ? 'قائمة التنزيلات' : 'Download list'}>
    <div className="list-tools">
      <div className="search-field"><Search size={16} /><input id="download-search" aria-label={ar ? 'بحث في التنزيلات' : 'Search downloads'} value={query} onChange={(event) => setQuery(event.target.value)} placeholder={ar ? 'بحث باسم الملف أو الموقع…' : 'Search files or websites…'} />{query && <button className="icon-button" onClick={() => setQuery('')} aria-label={ar ? 'مسح البحث' : 'Clear search'}><X size={14} /></button>}</div>
      <div className="list-filters"><Filter size={14} aria-hidden="true" />
        {mode === 'downloads' && <select aria-label={ar ? 'تصفية حسب الحالة' : 'Filter by status'} value={status} onChange={(event) => setStatus(event.target.value)}>{Object.entries(labels).map(([value, label]) => <option key={value} value={value}>{label}</option>)}</select>}
        <select aria-label={ar ? 'نوع الملف' : 'File type'} value={category} onChange={(event) => setCategory(event.target.value)}>{[['all', ar ? 'كل الأنواع' : 'All file types'], ['programs', ar ? 'برامج' : 'Applications'], ['archives', ar ? 'أرشيف' : 'Archives'], ['media', ar ? 'وسائط' : 'Media'], ['documents', ar ? 'مستندات' : 'Documents'], ['other', ar ? 'أخرى' : 'Other']].map(([value, label]) => <option key={value} value={value}>{label}</option>)}</select>
        <select aria-label={ar ? 'ترتيب التنزيلات' : 'Sort downloads'} value={sort} onChange={(event) => setSort(event.target.value)}>{[['newest', ar ? 'الأحدث' : 'Newest first'], ['oldest', ar ? 'الأقدم' : 'Oldest first'], ['name', ar ? 'الاسم' : 'Filename'], ['size', ar ? 'الحجم' : 'Largest first']].map(([value, label]) => <option key={value} value={value}>{label}</option>)}</select>
      </div>
    </div>
    {selection.length > 0 && <div className="selection-bar" role="group" aria-label={ar ? 'إجراءات المحددة' : 'Selected download actions'}><span>{selection.length} {ar ? 'محددة' : 'selected'}</span>
      {selection.some((task) => ['downloading', 'queued'].includes(task.status)) && <button className="text-button" onClick={() => selection.forEach((task) => onPause(task.id))}><Pause size={14} />{ar ? 'إيقاف مؤقت' : 'Pause'}</button>}
      {selection.some((task) => ['paused', 'error', 'queued'].includes(task.status)) && <button className="text-button" onClick={() => selection.forEach((task) => onResume(task.id))}><Play size={14} />{ar ? 'استئناف' : 'Resume'}</button>}
      <button className="text-button danger-text" onClick={() => setConfirmRemove(!confirmRemove)}><Trash2 size={14} />{ar ? 'إزالة من القائمة' : 'Remove from list'}</button>
      <button className="text-button" onClick={() => { setSelected(new Set()); setConfirmRemove(false); }}>{ar ? 'إلغاء التحديد' : 'Deselect'}</button>
      {confirmRemove && <div className="selection-confirm"><span>{ar ? 'ستبقى الملفات على القرص.' : 'Files will stay on disk.'}</span><button className="action-button" onClick={() => { selection.forEach((task) => onRemove(task.id, false)); setSelected(new Set()); setConfirmRemove(false); }}>{ar ? 'تأكيد الإزالة' : 'Confirm removal'}</button></div>}
    </div>}
    {filtered.length > 0 ? <div className="download-table">
      <div className="list-column-head"><label><input type="checkbox" checked={allSelected} onChange={() => { setSelected(allSelected ? new Set() : new Set(filtered.map((task) => task.id))); setConfirmRemove(false); }} aria-label={ar ? 'تحديد كل النتائج' : 'Select all results'} /><span>{ar ? 'الملف' : 'File'}</span></label><span>{ar ? 'الحالة والحجم' : 'Status & size'}</span><span>{ar ? 'السرعة / التقدم' : 'Speed / progress'}</span><span /></div>
      {filtered.map((task) => <TaskCard key={task.id} task={task} lang={lang} onPause={onPause} onResume={onResume} onRemove={onRemove} onStartImmediately={onStartImmediately} onRestart={onRestart} onShare={onShare} isSelected={selected.has(task.id)} onToggleSelect={toggle} />)}
      <div className="list-count">{filtered.length} / {inMode.length} {ar ? 'ملفات' : 'files'}</div>
    </div> : <div className="download-empty">
      <div className="empty-file" aria-hidden="true">{mode === 'finished' ? <CheckCircle2 size={27} /> : <ArrowDownToLine size={27} />}</div>
      <h2>{inMode.length ? (ar ? 'لا توجد نتائج مطابقة' : 'No matching downloads') : mode === 'finished' ? (ar ? 'ملفاتك المكتملة ستظهر هنا' : 'Your finished files will appear here') : (ar ? 'كل شيء جاهز لتنزيلك الأول' : 'Ready for your first download')}</h2>
      <p>{inMode.length ? (ar ? 'غيّر البحث أو التصفية لعرض المزيد من الملفات.' : 'Change your search or filters to see more files.') : mode === 'finished' ? (ar ? 'افتح الملفات، اعرض مجلدها، أو أرسلها إلى جهاز آخر.' : 'Open a file, find its folder, or send it to another device.') : (ar ? 'أضف رابط ملف بالأعلى. يمكنك إضافة عدة روابط دفعة واحدة.' : 'Add a file link above. You can also add several links at once.')}</p>
      {inMode.length ? <button className="action-button" onClick={() => { setQuery(''); setStatus('all'); setCategory('all'); }}>{ar ? 'مسح التصفية' : 'Clear filters'}</button> : mode === 'finished' && onSwitchToDownloads ? <button className="action-button" onClick={onSwitchToDownloads}>{ar ? 'إضافة تنزيل' : 'Add a download'}</button> : <button className="action-button" onClick={() => document.getElementById('download-url')?.focus()}><LinkIcon />{ar ? 'إضافة رابط' : 'Add a link'}</button>}
      {!inMode.length && mode === 'downloads' && <div className="empty-shortcuts"><span><kbd>Ctrl</kbd> + <kbd>V</kbd> {ar ? 'لصق رابط' : 'Paste a link'}</span><span>{ar ? 'أو اسحب الرابط إلى النافذة' : 'Or drop a link into this window'}</span></div>}
    </div>}
  </section>;
}
function LinkIcon() { return <ArrowDownToLine size={15} />; }
