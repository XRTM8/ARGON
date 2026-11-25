import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import {
  Globe,
  AlertCircle,
  X,
  ArrowDownToLine,
  Puzzle,
  Pause,
  Play,
  Trash2,
  Sun,
  Moon,
  Command,
  Minimize2
} from 'lucide-react';
import { Header, type MainViewTab } from './components/Header';
import { ArgonMark } from './components/ArgonMark';
import { QuickAddBar } from './components/QuickAddBar';
import { TaskList } from './components/TaskList';
import { TaskInspector } from './components/TaskInspector';
import { CommandPalette } from './components/CommandPalette';
import { SettingsView, type SettingsTab } from './components/SettingsView';
import { BrowserView } from './components/BrowserView';
import { HistoryView } from './components/HistoryView';
import { MiniWidget } from './components/MiniWidget';
import { SharingView, ClipboardView, FlowDialogs, SendDownloadDialog } from './components/FlowPanels';
import { ThroughputSparkline } from './components/ThroughputSparkline';
import { downloadManager } from './core/downloadManager';
import { updateManager } from './core/updates/updateManager';
import { useFlow } from './core/useFlow';
import { extractSharedUrl } from './core/downloadInput';
import { formatSpeed } from './core/segmentation';
import { consumeInitialAndroidSharedUrl, syncSystemBarTheme } from './core/safeArea';
import type { DownloadTask, MediaMetadata, QueueId } from './types/downloader';
import { parseTorrentFile, type TorrentMetadata } from './core/torrent';

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
  const [inspectedId, setInspectedId] = useState<string | null>(null);
  const [clipboardUrl, setClipboardUrl] = useState<{ text: string; key: number } | null>(null);
  const [dragging, setDragging] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [runtime, setRuntime] = useState(downloadManager.getRuntimeStatus());
  const [theme, setTheme] = useState<'dark' | 'light'>(() => downloadManager.getSettings().theme ?? 'dark');
  const [settingsTabRequest, setSettingsTabRequest] = useState<{ tab: SettingsTab; key: number } | null>(null);
  const [miniMode, setMiniMode] = useState(false);

  const toggleMiniMode = async (enable: boolean) => {
    setMiniMode(enable);
    const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
    if (inTauri) {
      try {
        const { getCurrentWindow, LogicalSize } = await import('@tauri-apps/api/window');
        const appWindow = getCurrentWindow();
        if (enable) {
          await appWindow.setSize(new LogicalSize(380, 180));
          await appWindow.setAlwaysOnTop(true);
        } else {
          await appWindow.setSize(new LogicalSize(1100, 720));
          await appWindow.setAlwaysOnTop(false);
        }
      } catch {
        // safe fallback
      }
    }
  };

  const openSettingsTab = (tab: SettingsTab) => {
    setSettingsTabRequest({ tab, key: performance.now() });
    setActiveView('settings');
  };

  const flow = useFlow();
  const [shareTask, setShareTask] = useState<DownloadTask | null>(null);
  const inspectedIdRef = useRef<string | null>(inspectedId);
  inspectedIdRef.current = inspectedId;
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
    const computeSig = (list: DownloadTask[]) => {
      let out = '';
      for (let i = 0; i < list.length; i++) {
        const t = list[i];
        const phase =
          t.status !== 'downloading'
            ? '0'
            : t.downloadedBytes === 0
              ? '1'
              : t.totalBytes > 0 && t.downloadedBytes >= t.totalBytes
                ? '3'
                : '2';
        out += `${t.id}:${t.status}:${phase}:${t.probed ? t.chunks.length : 0}:${t.filename}:${t.errorMessage || ''}:${t.resumeNotice || ''}:${t.mediaFinalizing ? 1 : 0};`;
      }
      return out;
    };

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
    }
    const onShared = (event: Event) => {
      const text = (event as CustomEvent<string>).detail?.trim();
      if (text) {
        setClipboardUrl({ text: extractSharedUrl(text), key: performance.now() });
        setActiveView('downloads');
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

    const handleDrop = async (event: DragEvent) => {
      event.preventDefault();
      setDragging(false);
      const files = event.dataTransfer?.files;
      if (files && files.length > 0) {
        const file = files[0];
        if (file.name.toLowerCase().endsWith('.torrent') || file.type === 'application/x-bittorrent') {
          try {
            const buffer = await file.arrayBuffer();
            const meta = await parseTorrentFile(new Uint8Array(buffer));
            const magnet = `magnet:?xt=urn:btih:${meta.infoHash}&dn=${encodeURIComponent(meta.name)}`;
            setClipboardUrl({ text: magnet, key: performance.now() });
            setActiveView('downloads');
            return;
          } catch {
            // fallback to text extraction
          }
        }
      }
      const rawText = (event.dataTransfer?.getData('text/uri-list') || event.dataTransfer?.getData('text/plain') || '').trim();
      if (/^magnet:\?[\s\S]*xt=urn:btih:/i.test(rawText)) {
        setClipboardUrl({ text: rawText, key: performance.now() });
        setActiveView('downloads');
        return;
      }
      const urls = rawText
        .split(/\r?\n/)
        .filter((line) => /^https?:\/\//i.test(line))
        .join('\n');
      if (urls) {
        setClipboardUrl({ text: urls, key: performance.now() });
        setActiveView('downloads');
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
    mirrors?: string[],
    torrentMeta?: TorrentMetadata
  ) => {
    setActiveView('downloads');
    void downloadManager.addTaskAsync(url, filename, segments, expectedSha256, undefined, mediaInfo, queueId, mirrors, torrentMeta);
  };

  const share = (task: DownloadTask) => {
    setShareTask(task);
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

  if (miniMode) {
    return (
      <MiniWidget
        lang={lang}
        theme={theme}
        tasks={tasks}
        stats={stats}
        speedHistory={speedHistory}
        onToggleTheme={toggleTheme}
        onRestore={() => void toggleMiniMode(false)}
      />
    );
  }

  return (
    <div className="argon-app">
      <Header lang={lang} activeTab={activeView} onTabChange={setActiveView} stats={stats} runtime={runtime} />

      <div className="argon-content">
        <header className="argon-topbar">
          <div className="topbar-leading">
            <div className="mobile-brand-pill" dir={ar ? 'rtl' : 'ltr'}>
              <ArgonMark size={22} />
              <span className="brand-title">{ar ? 'آرغون' : 'ARGON'}</span>
            </div>
            <div className="breadcrumb">
              <span>{ar ? 'آرغون' : 'ARGON'}</span>
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
              className="topbar-chip hide-on-mobile"
              onClick={() => void toggleMiniMode(true)}
              aria-label={ar ? 'الوضع المصغر' : 'Mini Mode'}
              title={ar ? 'الوضع المصغر' : 'Mini Mode'}
            >
              <Minimize2 size={13} />
              <span>{ar ? 'مصغر' : 'Mini'}</span>
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
              <div className="view-header downloads-view-header">
                <div className="view-header-title">
                  <h1>{ar ? 'التنزيلات' : 'Downloads'}</h1>
                  {tasks.length > 0 && (
                    <span className="count-badge" dir="ltr">
                      {tasks.length}
                    </span>
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
                    <div className="stream-badge">
                      <ArrowDownToLine size={14} />
                      <span>
                        {stats.activeCount > 0
                          ? (ar ? `${stats.activeCount} تنزيل نشط` : `${stats.activeCount} active`)
                          : (ar ? `التنزيلات (${tasks.length})` : `Downloads (${tasks.length})`)}
                      </span>
                      {stats.completedCount > 0 && stats.activeCount > 0 && (
                        <span className="stream-completed-count">
                          · {ar ? `${stats.completedCount} مكتمل` : `${stats.completedCount} completed`}
                        </span>
                      )}
                    </div>

                    <div className="download-toolbar-actions">
                      {stats.activeCount > 0 ? (
                        <button
                          type="button"
                          className="text-button"
                          onClick={() => downloadManager.pauseAll()}
                          title={ar ? 'إيقاف مؤقت للكل' : 'Pause all'}
                        >
                          <Pause size={13} />
                          <span>{ar ? 'إيقاف الكل' : 'Pause all'}</span>
                        </button>
                      ) : (
                        tasks.some((t) => t.status === 'paused') && (
                          <button
                            type="button"
                            className="text-button"
                            onClick={() => downloadManager.resumeAll()}
                            title={ar ? 'استئناف الكل' : 'Resume all'}
                          >
                            <Play size={13} />
                            <span>{ar ? 'استئناف' : 'Resume'}</span>
                          </button>
                        )
                      )}

                      {stats.completedCount > 0 && (
                        <button
                          type="button"
                          className="text-button text-danger"
                          onClick={() => downloadManager.clearCompleted()}
                          title={ar ? 'مسح التنزيلات المكتملة من القائمة' : 'Clear completed from list'}
                        >
                          <Trash2 size={13} />
                          <span>{ar ? 'مسح المكتملة' : 'Clear'}</span>
                        </button>
                      )}
                    </div>
                  </div>

                  <TaskList {...taskProps} mode="all" />
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
          {activeView === 'history' && <HistoryView lang={lang} tasks={tasks} />}
          {activeView === 'browser' && <BrowserView lang={lang} />}
          {activeView === 'settings' && <SettingsView lang={lang} requestedTab={settingsTabRequest} />}
        </main>
      </div>

      <FlowDialogs lang={lang} />
      {shareTask && !flow.snapshot?.pairings.length && !flow.snapshot?.incoming.length && (
        <SendDownloadDialog
          key={shareTask.id}
          lang={lang}
          task={shareTask}
          onClose={() => setShareTask(null)}
          onOpenSharing={() => { setShareTask(null); setActiveView('sharing'); }}
        />
      )}

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

export default App;
