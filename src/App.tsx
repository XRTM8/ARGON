import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import {
  Globe,
  AlertCircle,
  X,
  ArrowDownToLine,
  ArrowUpRight,
  ArrowDownLeft,
  FolderOpen,
  CheckCircle2,
  Puzzle,
  Pause,
  Play,
  Trash2,
  Search,
  Sun,
  Moon,
  Command
} from 'lucide-react';
import { Header, type MainViewTab } from './components/Header';
import { ArgonMark } from './components/ArgonMark';
import { QuickAddBar } from './components/QuickAddBar';
import { TaskList } from './components/TaskList';
import { TaskInspector } from './components/TaskInspector';
import { CommandPalette } from './components/CommandPalette';
import { SettingsView, type SettingsTab } from './components/SettingsView';
import { BrowserView } from './components/BrowserView';
import { SharingView, ClipboardView, FlowDialogs, flowStatus } from './components/FlowPanels';
import { ThroughputSparkline } from './components/ThroughputSparkline';
import { downloadManager } from './core/downloadManager';
import { updateManager } from './core/updates/updateManager';
import { useFlow } from './core/useFlow';
import { extractSharedUrl } from './core/downloadInput';
import { formatBytes, formatSpeed } from './core/segmentation';
import { consumeInitialAndroidSharedUrl, syncSystemBarTheme } from './core/safeArea';
import type { DownloadTask, MediaMetadata, QueueId } from './types/downloader';

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
  const [speedHistory, setSpeedHistory] = useState(downloadManager.getSpeedHistory());
  const [finished, setFinished] = useState(false);
  const [inspectedId, setInspectedId] = useState<string | null>(null);
  const [clipboardUrl, setClipboardUrl] = useState<{ text: string; key: number } | null>(null);
  const [dragging, setDragging] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [runtime, setRuntime] = useState(downloadManager.getRuntimeStatus());
  const [theme, setTheme] = useState<'dark' | 'light'>(() => downloadManager.getSettings().theme ?? 'dark');
  const [settingsTabRequest, setSettingsTabRequest] = useState<{ tab: SettingsTab; key: number } | null>(null);
  const openSettingsTab = (tab: SettingsTab) => {
    setSettingsTabRequest({ tab, key: performance.now() });
    setActiveView('settings');
  };

  const flow = useFlow();
  const inspectedIdRef = useRef<string | null>(inspectedId);
  inspectedIdRef.current = inspectedId;
  const flowClientRef = useRef(flow.client);
  flowClientRef.current = flow.client;
  const lastStructSigRef = useRef('');
  const update = useSyncExternalStore(updateManager.subscribe, updateManager.getState);
  const ar = lang === 'ar';

  useEffect(() => updateManager.start(), []);

  useEffect(() => {
    syncSystemBarTheme(theme);
  }, [theme]);

  const viewNames: Record<MainViewTab, string> = {
    downloads: ar ? 'التنزيلات' : 'Downloads',
    sharing: ar ? 'المشاركة المحلية' : 'Local Sharing',
    clipboard: ar ? 'الحافظة المشفرة' : 'Secure Clipboard',
    history: ar ? 'سجل العمليات' : 'Activity Log',
    browser: ar ? 'إضافة المتصفح' : 'Browser extension',
    settings: ar ? 'الإعدادات' : 'Settings'
  };

  useEffect(() => {
    const computeSig = (list: DownloadTask[]) =>
      list
        .map((t) => {
          const phase =
            t.status !== 'downloading'
              ? ''
              : t.downloadedBytes === 0
                ? '0'
                : t.totalBytes > 0 && t.downloadedBytes >= t.totalBytes
                  ? '2'
                  : '1';
          return `${t.id}:${t.status}:${phase}:${t.probed ? t.chunks.length : 0}:${t.filename}:${t.errorMessage || ''}`;
        })
        .join('|');

    const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

    return downloadManager.subscribe(() => {
      const nextTasks = downloadManager.getTasks();
      const sig = computeSig(nextTasks);
      if (!inTauri || inspectedIdRef.current !== null || sig !== lastStructSigRef.current) {
        lastStructSigRef.current = sig;
        setTasks([...nextTasks]);
      }
      setStats({ ...downloadManager.getStats() });
      setSpeedHistory(downloadManager.getSpeedHistory());
      setRuntime(downloadManager.getRuntimeStatus());
      const nextTheme = downloadManager.getSettings().theme ?? 'dark';
      setTheme((prev) => (prev !== nextTheme ? nextTheme : prev));
    });
  }, []);

  useEffect(() => {
    if (inspectedId) {
      setTasks([...downloadManager.getTasks()]);
    }
  }, [inspectedId]);

  // Android Share Sheet support (cold start & warm onNewIntent)
  useEffect(() => {
    const initial = consumeInitialAndroidSharedUrl();
    if (initial) {
      setClipboardUrl({ text: extractSharedUrl(initial), key: performance.now() });
      setActiveView('downloads');
      setFinished(false);
    }
    const onShared = (event: Event) => {
      const text = (event as CustomEvent<string>).detail?.trim();
      if (text) {
        setClipboardUrl({ text: extractSharedUrl(text), key: performance.now() });
        setActiveView('downloads');
        setFinished(false);
      }
    };
    window.addEventListener('argon-share-url', onShared);
    return () => window.removeEventListener('argon-share-url', onShared);
  }, []);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'k') {
        event.preventDefault();
        setPaletteOpen((v) => !v);
      } else if (activeView === 'downloads' && (event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'f') {
        event.preventDefault();
        document.getElementById('download-search')?.focus();
      }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
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
      const raw = event.clipboardData?.getData('text/plain')?.trim();
      const text = raw ? extractSharedUrl(raw) : '';
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
        .filter((line) => /^https?:\/\//i.test(line))
        .join('\n');
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

  const add = (
    url: string,
    filename?: string,
    segments?: number,
    expectedSha256?: string,
    mediaInfo?: MediaMetadata,
    queueId?: QueueId,
    mirrors?: string[]
  ) => {
    setFinished(false);
    setActiveView('downloads');
    void downloadManager.addTaskAsync(url, filename, segments, expectedSha256, undefined, mediaInfo, queueId, mirrors);
  };

  const share = (task: DownloadTask) => {
    setActiveView('sharing');
    void flowClientRef.current.chooseFiles(task.savePath).catch(() => {});
  };

  const toggleTheme = () => {
    const next = theme === 'dark' ? 'light' : 'dark';
    setTheme(next);
    void downloadManager.updateSettings({ theme: next });
  };

  const inspectedTask = inspectedId ? tasks.find((t) => t.id === inspectedId) || null : null;

  const taskProps = {
    tasks,
    lang,
    inspectedId,
    onInspect: (id: string) => setInspectedId((prev) => (prev === id ? null : id)),
    onPause: (id: string) => downloadManager.pauseTask(id),
    onResume: (id: string) => downloadManager.resumeTask(id),
    onStartImmediately: (id: string) => downloadManager.startTaskImmediately(id),
    onRestart: (id: string) => downloadManager.restartTask(id),
    onRemove: (id: string, disk?: boolean) => {
      if (inspectedId === id) setInspectedId(null);
      void downloadManager.removeTask(id, disk);
    },
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
        <header className="argon-topbar">
          <div className="topbar-leading">
            <div className="mobile-brand-pill" dir="ltr">
              <ArgonMark size={18} />
              <span>ARGON</span>
            </div>
            <div className="breadcrumb">
              <span>ARGON</span>
              <span>/</span>
              <strong>{viewNames[activeView]}</strong>
            </div>
          </div>

          <div className="topbar-actions">
            {stats.currentTotalSpeed > 0 && (
              <div className="topbar-sparkline-pill" dir="ltr">
                <ThroughputSparkline history={speedHistory} width={56} height={18} />
                <span className="download-speed">
                  <ArrowDownToLine size={13} />
                  {formatSpeed(stats.currentTotalSpeed)}
                </span>
              </div>
            )}

            <button
              type="button"
              className="topbar-chip hide-on-mobile"
              onClick={() => setActiveView('browser')}
              aria-label={ar ? 'إضافة المتصفح' : 'Browser extension'}
              title={ar ? 'إضافة المتصفح' : 'Browser extension'}
            >
              <Puzzle size={14} />
              <span>{ar ? 'المتصفح' : 'Extension'}</span>
            </button>

            <button
              type="button"
              className="topbar-chip command-trigger"
              onClick={() => setPaletteOpen(true)}
              aria-label={ar ? 'لوحة الأوامر' : 'Command Palette'}
            >
              <Command size={13} />
              <span>{ar ? 'أوامر' : 'Commands'}</span>
              <kbd dir="ltr">Ctrl+K</kbd>
            </button>

            <button
              type="button"
              className="topbar-chip"
              onClick={toggleTheme}
              aria-label={ar ? 'تبديل المظهر' : 'Toggle theme'}
              title={ar ? 'تبديل المظهر' : 'Toggle theme'}
            >
              {theme === 'dark' ? <Sun size={14} /> : <Moon size={14} />}
            </button>

            <button
              type="button"
              className="language-button"
              onClick={() => setLang(ar ? 'en' : 'ar')}
              aria-label={ar ? 'Switch to English' : 'التبديل إلى العربية'}
            >
              <Globe size={14} />
              <span>{ar ? 'EN' : 'عربي'}</span>
            </button>
          </div>
        </header>

        {(update.phase === 'ready' || update.phase === 'available') && activeView !== 'settings' && (
          <div className="update-ready-notice" role="status">
            <span>
              {update.phase === 'ready'
                ? ar
                  ? `تحديث ${update.version} جاهز للتثبيت عندما تنتهي عملياتك.`
                  : `Update ${update.version} is ready to install after your transfers finish.`
                : ar
                  ? `يتوفر إصدار جديد (${update.version}) للتنزيل والتثبيت.`
                  : `A new update (${update.version}) is available to download.`}
            </span>
            <button className="text-button" onClick={() => openSettingsTab('updates')}>
              {ar ? 'عرض التحديث' : 'View update'}
            </button>
          </div>
        )}

        {flow.error && (
          <div className="global-error" role="alert">
            <AlertCircle size={16} />
            <span dir="auto">{flow.error}</span>
            <button className="icon-button" aria-label={ar ? 'إغلاق التنبيه' : 'Dismiss notice'} onClick={flow.client.clearError}>
              <X size={14} />
            </button>
          </div>
        )}

        <main className="argon-main" id="main-content">
          {activeView === 'downloads' && (
            <>
              <div className="view-header">
                <div className="view-header-title">
                  <h1>{ar ? 'التنزيلات' : 'Downloads'}</h1>
                  {tasks.length > 0 && (
                    <span className="count-badge" dir="ltr">
                      {tasks.length}
                    </span>
                  )}
                </div>

                <div className="view-header-actions">
                  {stats.activeCount > 0 ? (
                    <button className="action-button" onClick={() => downloadManager.pauseAll()} title={ar ? 'إيقاف مؤقت للكل' : 'Pause all downloads'}>
                      <Pause size={13} />
                      <span>{ar ? 'إيقاف الكل' : 'Pause All'}</span>
                    </button>
                  ) : (
                    tasks.some((t) => t.status === 'paused') && (
                      <button className="action-button" onClick={() => downloadManager.resumeAll()} title={ar ? 'استئناف الكل' : 'Resume all downloads'}>
                        <Play size={13} />
                        <span>{ar ? 'استئناف الكل' : 'Resume All'}</span>
                      </button>
                    )
                  )}

                  {stats.completedCount > 0 && (
                    <button className="action-button" onClick={() => downloadManager.clearCompleted()} title={ar ? 'مسح التنزيلات المكتملة من القائمة' : 'Clear completed from list'}>
                      <Trash2 size={13} />
                      <span>{ar ? 'مسح المكتملة' : 'Clear Completed'}</span>
                    </button>
                  )}
                </div>
              </div>

              {runtime !== 'ready' && (
                <div className="runtime-notice" role="status">
                  <AlertCircle size={16} />
                  <span>
                    {runtime === 'preview'
                      ? ar
                        ? 'معاينة الواجهة فقط — التنزيل يتطلب تشغيل التطبيق.'
                        : 'Interface preview only — open ARGON app to download.'
                      : runtime === 'error'
                        ? ar
                          ? 'تعذّرت تهيئة المحرك. أعد فتح التطبيق وتحقق من مجلد الحفظ.'
                          : 'Engine could not start. Check download folder.'
                        : ar
                          ? 'جارٍ تهيئة المحرك…'
                          : 'Preparing engine…'}
                  </span>
                </div>
              )}

              <QuickAddBar
                lang={lang}
                onAdd={add}
                detectedClipboardUrl={clipboardUrl?.text}
                intakeKey={clipboardUrl?.key}
                onClearClipboardUrl={() => setClipboardUrl(null)}
              />

              <div className={`workspace-split ${inspectedTask ? 'has-inspector' : ''}`}>
                <div className="workspace-primary">
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
                    <button className="text-button hide-on-mobile" onClick={() => setActiveView('browser')}>
                      <Puzzle size={14} />
                      {ar ? 'ربط المتصفح' : 'Connect browser'}
                      <ArrowUpRight size={13} />
                    </button>
                  </div>

                  <TaskList {...taskProps} mode={finished ? 'finished' : 'downloads'} onSwitchToDownloads={() => setFinished(false)} />
                </div>

                {inspectedTask && (
                  <div className="desktop-inspector-wrap">
                    <TaskInspector task={inspectedTask} lang={lang} onClose={() => setInspectedId(null)} />
                  </div>
                )}
              </div>
            </>
          )}

          {activeView === 'sharing' && <SharingView lang={lang} onSwitchTab={setActiveView} />}
          {activeView === 'clipboard' && <ClipboardView lang={lang} onDownload={(url) => add(url)} onSwitchTab={setActiveView} />}
          {activeView === 'history' && <UnifiedHistory lang={lang} tasks={tasks} />}
          {activeView === 'browser' && <BrowserView lang={lang} />}
          {activeView === 'settings' && <SettingsView lang={lang} requestedTab={settingsTabRequest} />}
        </main>
      </div>

      <FlowDialogs lang={lang} />

      <CommandPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
        lang={lang}
        theme={theme}
        onNavigate={setActiveView}
        onPauseAll={() => downloadManager.pauseAll()}
        onResumeAll={() => downloadManager.resumeAll()}
        onToggleTheme={toggleTheme}
        onToggleLang={() => setLang(ar ? 'en' : 'ar')}
        onSetSpeedLimit={(limit) => void downloadManager.updateSettings({ globalSpeedLimitBytes: limit })}
        onFocusIntake={() => setTimeout(() => document.getElementById('download-url')?.focus(), 40)}
      />

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

export default App;
