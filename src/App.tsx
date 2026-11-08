import { useEffect, useState } from 'react';
import {
  Globe,
  Wifi,
  AlertCircle,
  X,
  ArrowDownToLine,
  ArrowUpRight,
  ArrowDownLeft,
  FolderOpen,
  CheckCircle2,
  Power,
  HardDrive,
  Puzzle,
  Pause,
  Play,
  Trash2,
  Search
} from 'lucide-react';
import { Header, CURRENT_VERSION, type MainViewTab } from './components/Header';
import { QuickAddBar } from './components/QuickAddBar';
import { TaskList } from './components/TaskList';
import { SettingsView } from './components/SettingsView';
import { BrowserView } from './components/BrowserView';
import { SharingView, ClipboardView, FlowDialogs, SharingSettings, flowStatus } from './components/FlowPanels';
import { downloadManager } from './core/downloadManager';
import { useFlow } from './core/useFlow';
import { nativeRuntime } from './core/flowClient';
import { formatBytes, formatSpeed } from './core/segmentation';
import type { DownloadTask } from './types/downloader';

export function App() {
  const [lang, setLang] = useState<'ar' | 'en'>(() => {
    try {
      return localStorage.getItem('argon_language') === 'en' ? 'en' : 'ar';
    } catch {
      return 'ar';
    }
  });

  const [activeView, setActiveView] = useState<MainViewTab>('downloads');
  const [tasks, setTasks] = useState(downloadManager.getTasks());
  const [stats, setStats] = useState(downloadManager.getStats());
  const [finished, setFinished] = useState(false);
  const [clipboardUrl, setClipboardUrl] = useState<{ text: string; key: number } | null>(null);
  const [dragging, setDragging] = useState(false);
  const [runtime, setRuntime] = useState(downloadManager.getRuntimeStatus());

  const flow = useFlow();
  const ar = lang === 'ar';

  const viewNames: Record<MainViewTab, string> = {
    downloads: ar ? 'التنزيلات' : 'Downloads',
    sharing: ar ? 'المشاركة المحلية' : 'Local Sharing',
    clipboard: ar ? 'الحافظة المشفرة' : 'Secure Clipboard',
    history: ar ? 'سجل العمليات' : 'Activity Log',
    browser: ar ? 'إضافة المتصفح' : 'Browser extension',
    settings: ar ? 'الإعدادات' : 'Settings'
  };

  useEffect(() => {
    return downloadManager.subscribe(() => {
      setTasks([...downloadManager.getTasks()]);
      setStats({ ...downloadManager.getStats() });
      setRuntime(downloadManager.getRuntimeStatus());
    });
  }, []);

  useEffect(() => {
    const search = (event: KeyboardEvent) => {
      if (activeView === 'downloads' && (event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'f') {
        event.preventDefault(); document.getElementById('download-search')?.focus();
      }
    };
    window.addEventListener('keydown', search);
    return () => window.removeEventListener('keydown', search);
  }, [activeView]);

  useEffect(() => {
    document.documentElement.lang = lang;
    document.documentElement.dir = ar ? 'rtl' : 'ltr';
    try {
      localStorage.setItem('argon_language', lang);
    } catch {}
  }, [lang, ar]);

  useEffect(() => {
    const handlePaste = (event: ClipboardEvent) => {
      if ((event.target as HTMLElement)?.closest('input,textarea,[contenteditable="true"]')) return;
      const text = event.clipboardData?.getData('text/plain')?.trim();
      if (text && /^https?:\/\//i.test(text)) {
        event.preventDefault();
        setClipboardUrl({ text, key: performance.now() });
        setActiveView('downloads');
        setFinished(false);
      }
    };

    const handleDragOver = (event: DragEvent) => {
      if (event.dataTransfer?.types.includes('text/plain') || event.dataTransfer?.types.includes('text/uri-list')) {
        event.preventDefault();
        setDragging(true);
      }
    };

    const handleDragLeave = (event: DragEvent) => {
      if (!event.relatedTarget) setDragging(false);
    };

    const handleDrop = (event: DragEvent) => {
      event.preventDefault();
      setDragging(false);
      const urls = (event.dataTransfer?.getData('text/uri-list') || event.dataTransfer?.getData('text/plain') || '')
        .trim()
        .split(/\r?\n/)
        .filter((line) => /^https?:\/\//i.test(line)).join('\n');
      if (urls) {
        setClipboardUrl({ text: urls, key: performance.now() });
        setActiveView('downloads');
        setFinished(false);
      }
    };

    window.addEventListener('paste', handlePaste);
    window.addEventListener('dragover', handleDragOver);
    window.addEventListener('dragleave', handleDragLeave);
    window.addEventListener('drop', handleDrop);

    return () => {
      window.removeEventListener('paste', handlePaste);
      window.removeEventListener('dragover', handleDragOver);
      window.removeEventListener('dragleave', handleDragLeave);
      window.removeEventListener('drop', handleDrop);
    };
  }, []);

  const add = (url: string, filename?: string, segments?: number) => {
    downloadManager.addTask(url, filename, segments);
    setFinished(false);
    setActiveView('downloads');
  };

  const share = (task: DownloadTask) => {
    setActiveView('sharing');
    void flow.client.chooseFiles(task.savePath).catch(() => {});
  };

  const taskProps = {
    tasks,
    lang,
    onPause: (id: string) => downloadManager.pauseTask(id),
    onResume: (id: string) => downloadManager.resumeTask(id),
    onStartImmediately: (id: string) => downloadManager.startTaskImmediately(id),
    onRestart: (id: string) => downloadManager.restartTask(id),
    onRemove: (id: string, disk?: boolean) => downloadManager.removeTask(id, disk),
    onClearCompleted: () => downloadManager.clearCompleted(),
    onPauseAll: () => downloadManager.pauseAll(),
    onResumeAll: () => downloadManager.resumeAll(),
    onShare: share
  };

  const activeTasksCount = tasks.filter((t) => t.status !== 'completed').length;

  return (
    <div className="argon-app">
      <Header lang={lang} activeTab={activeView} onTabChange={setActiveView} stats={stats} runtime={runtime} />

      <div className="argon-content">
        {/* Topbar */}
        <header className="argon-topbar">
          <div className="breadcrumb">
            <span>ARGON</span>
            <span>/</span>
            <strong>{viewNames[activeView]}</strong>
          </div>

          <div className="topbar-actions">
            {stats.currentTotalSpeed > 0 && (
              <span className="download-speed" dir="ltr">
                <ArrowDownToLine size={13} />
                {formatSpeed(stats.currentTotalSpeed)}
              </span>
            )}

            <button
              className={`sharing-indicator ${flow.snapshot?.enabled ? 'is-enabled' : ''}`}
              disabled={
                !nativeRuntime() ||
                flow.pending.includes('flow_start') ||
                flow.pending.includes('flow_stop')
              }
              onClick={() => {
                void flow.client.execute(flow.snapshot?.enabled ? 'flow_stop' : 'flow_start').catch(() => {});
              }}
              title={ar ? 'مفتاح المشاركة المحلية' : 'Local Sharing Switch'}
            >
              <span className="presence" />
              <span>
                {flow.snapshot?.enabled
                  ? ar
                    ? 'المشاركة مفعّلة'
                    : 'Sharing active'
                  : ar
                    ? 'المشاركة متوقفة'
                    : 'Sharing inactive'}
              </span>
              <Power size={12} />
            </button>

            <button
              className="language-button"
              onClick={() => setLang(ar ? 'en' : 'ar')}
              aria-label={ar ? 'Switch to English' : 'التبديل إلى العربية'}
            >
              <Globe size={14} />
              <span>{ar ? 'EN' : 'عربي'}</span>
            </button>
          </div>
        </header>

        {/* Global Error Banner */}
        {flow.error && (
          <div className="global-error" role="alert">
            <AlertCircle size={16} />
            <span dir="auto">{flow.error}</span>
            <button
              className="icon-button"
              aria-label={ar ? 'إغلاق التنبيه' : 'Dismiss notice'}
              onClick={flow.client.clearError}
            >
              <X size={14} />
            </button>
          </div>
        )}

        {/* Main Content Workspace */}
        <main className="argon-main" id="main-content">
          {activeView === 'downloads' && (
            <>
              {/* Functional View Header */}
              <div className="view-header">
                <div className="view-heading">
                <span className="eyebrow">{ar ? 'مساحة العمل' : 'YOUR WORKSPACE'}</span>
                <div className="view-header-title">
                  <h1>{ar ? 'التنزيلات' : 'Downloads'}</h1>
                  <span className="count-badge" dir="ltr">
                    {tasks.length}
                  </span>
                </div>
                <p className="view-description">{ar ? 'ملفاتك، من أول رابط لآخر بايت.' : 'Your files, from the first link to the last byte.'}</p>
                </div>

                <div className="view-header-actions">
                  {stats.activeCount > 0 ? (
                    <button
                      className="action-button"
                      onClick={() => downloadManager.pauseAll()}
                      title={ar ? 'إيقاف مؤقت للكل' : 'Pause all downloads'}
                    >
                      <Pause size={13} />
                      <span>{ar ? 'إيقاف الكل' : 'Pause All'}</span>
                    </button>
                  ) : (
                    tasks.some((t) => t.status === 'paused') && (
                      <button
                        className="action-button"
                        onClick={() => downloadManager.resumeAll()}
                        title={ar ? 'استئناف الكل' : 'Resume all downloads'}
                      >
                        <Play size={13} />
                        <span>{ar ? 'استئناف الكل' : 'Resume All'}</span>
                      </button>
                    )
                  )}

                  {stats.completedCount > 0 && (
                    <button
                      className="action-button"
                      onClick={() => downloadManager.clearCompleted()}
                      title={ar ? 'مسح التنزيلات المكتملة من القائمة' : 'Clear completed from list'}
                    >
                      <Trash2 size={13} />
                      <span>{ar ? 'مسح المكتملة' : 'Clear Completed'}</span>
                    </button>
                  )}
                </div>
              </div>

              {runtime !== 'ready' && <div className="runtime-notice" role="status"><AlertCircle size={16} /><span>{runtime === 'preview' ? (ar ? 'هذه معاينة للواجهة. افتح تطبيق ARGON لتنزيل الملفات والاتصال بإضافة المتصفح.' : 'Interface preview. Open the ARGON desktop app to download files and connect your browser.') : runtime === 'error' ? (ar ? 'تعذّرت تهيئة المحرك. أعد فتح التطبيق وتحقق من مجلد الحفظ.' : 'The engine could not start. Reopen the app and check the download directory.') : (ar ? 'جارٍ تهيئة المحرك ومجلد الحفظ…' : 'Preparing the engine and download folder…')}</span></div>}
              <div className="transfer-overview" aria-label={ar ? 'ملخص التنزيلات' : 'Transfer overview'}>
                <div><span>{ar ? 'السرعة الحالية' : 'TRANSFER SPEED'}</span><strong dir="ltr">{formatSpeed(stats.currentTotalSpeed)}</strong></div>
                <div><span>{ar ? 'جارٍ التنزيل' : 'DOWNLOADING'}</span><strong>{stats.activeCount}<small>{ar ? 'ملفات' : 'files'}</small></strong></div>
                <div><span>{ar ? 'في الانتظار' : 'IN QUEUE'}</span><strong>{stats.queuedCount}<small>{ar ? 'ملفات' : 'files'}</small></strong></div>
                <div><span>{ar ? 'البيانات المحمّلة' : 'DATA DOWNLOADED'}</span><strong dir="ltr">{formatBytes(stats.totalBytesDownloaded)}</strong></div>
              </div>
              <QuickAddBar
                lang={lang}
                onAdd={add}
                detectedClipboardUrl={clipboardUrl?.text}
                intakeKey={clipboardUrl?.key}
                onClearClipboardUrl={() => setClipboardUrl(null)}
              />

              {/* Tabs */}
              <div className="download-toolbar">
                <div className="download-tabs">
                  <button aria-pressed={!finished} onClick={() => setFinished(false)}>
                    <span>{ar ? 'قائمة التنزيل' : 'In progress'}</span>
                    <span>{activeTasksCount}</span>
                  </button>
                  <button aria-pressed={finished} onClick={() => setFinished(true)}>
                    <span>{ar ? 'المكتملة' : 'Completed'}</span>
                    <span>{stats.completedCount}</span>
                  </button>
                </div>
                <button className="text-button" onClick={() => setActiveView('browser')}><Puzzle size={14} />{ar ? 'ربط المتصفح' : 'Connect browser'}<ArrowUpRight size={13} /></button>
              </div>

              {/* Task List */}
              <TaskList
                {...taskProps}
                mode={finished ? 'finished' : 'downloads'}
                onSwitchToDownloads={() => setFinished(false)}
              />
            </>
          )}

          {activeView === 'sharing' && <SharingView lang={lang} />}
          {activeView === 'clipboard' && <ClipboardView lang={lang} onDownload={(url) => add(url)} />}
          {activeView === 'history' && <UnifiedHistory lang={lang} tasks={tasks} />}
          {activeView === 'browser' && <BrowserView lang={lang} />}
          {activeView === 'settings' && (
            <div className="settings-layout">
              <SettingsView lang={lang} />
              <SharingSettings lang={lang} />
            </div>
          )}
        </main>

        {/* Clean Footer Status Bar */}
        <footer className="workspace-footer">
          <span>
            <HardDrive size={13} />
            <span>{ar ? 'الملفات تُحفظ على جهازك' : 'Files are saved on your device'}</span>
          </span>
          <span dir="ltr">ARGON ENGINE v{CURRENT_VERSION}</span>
        </footer>
      </div>

      <FlowDialogs lang={lang} />

      {dragging && (
        <div className="url-drop-overlay">
          <ArrowDownToLine size={32} />
          <strong>{ar ? 'أفلت الروابط لمراجعتها وإضافتها' : 'Drop links to review and add them'}</strong>
        </div>
      )}
    </div>
  );
}

function UnifiedHistory({ lang, tasks }: { lang: 'ar' | 'en'; tasks: DownloadTask[] }) {
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

        <div className="flex items-center gap-2">
          <div className="relative">
            <Search className="w-3.5 h-3.5 text-zinc-400 absolute left-2.5 rtl:right-2.5 rtl:left-auto top-1/2 -translate-y-1/2 pointer-events-none" />
            <input
              type="text"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              placeholder={ar ? 'بحث في السجل...' : 'Filter activity...'}
              className="text-xs bg-zinc-900 border border-white/[0.08] rounded-lg pl-8 pr-3 rtl:pr-8 rtl:pl-3 py-1.5 text-zinc-200 placeholder:text-zinc-500 focus:outline-none focus:border-cyan-500 w-44 sm:w-56"
            />
          </div>
        </div>
      </div>

      {actionError && <p className="inline-error" role="alert">{actionError}</p>}
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
                    ? ar
                      ? 'تنزيل'
                      : 'Download'
                    : `${entry.kind === 'send' ? (ar ? 'إلى' : 'To') : ar ? 'من' : 'From'} ${entry.peer}`}{' '}
                  · {formatBytes(entry.size)} ·{' '}
                  {new Date(entry.date).toLocaleString(ar ? 'ar-IQ' : 'en', {
                    month: 'short',
                    day: 'numeric',
                    hour: '2-digit',
                    minute: '2-digit'
                  })}
                </span>
              </div>

              <small className={entry.status === 'error' ? 'text-rose-400' : ''}>
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
                <CheckCircle2 size={15} className="text-emerald-400" />
              ) : (
                <span />
              )}
            </article>
          ))
        ) : (
          <div className="quiet-empty">
            <Wifi size={22} />
            <p>{ar ? 'لا توجد عمليات مسجلة حتى الآن.' : 'No recorded activity yet.'}</p>
          </div>
        )}
      </div>
    </div>
  );
}

export default App;
