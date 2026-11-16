import React, { useEffect, useState } from 'react';
import { FolderOpen, Bell, Volume2, Sliders, Cpu, Shield, Film, Download, RefreshCw, Key, X, ExternalLink } from 'lucide-react';
import { downloadManager } from '../core/downloadManager';
import { UpdatesPanel } from './UpdatesPanel';
import { SharingSettings } from './FlowPanels';
import { isAndroidRuntime } from '../core/safeArea';
import type { MediaToolsStatus } from '../types/downloader';

export type SettingsTab = 'general' | 'engine' | 'updates';

interface SettingsViewProps {
  lang: 'ar' | 'en';
  requestedTab?: { tab: SettingsTab; key: number } | null;
}

export const SettingsView: React.FC<SettingsViewProps> = ({ lang, requestedTab }) => {
  const ar = lang === 'ar';
  const android = isAndroidRuntime();
  const s = downloadManager.getSettings();

  const [tab, setTab] = useState<SettingsTab>(requestedTab?.tab ?? 'general');
  useEffect(() => {
    if (requestedTab) setTab(requestedTab.tab);
  }, [requestedTab]);
  const [maxConcurrent, setMaxConcurrent] = useState(s.maxConcurrentDownloads);
  const [maxPerHost, setMaxPerHost] = useState(s.maxConnectionsPerHost ?? 4);
  const [defaultSegments, setDefaultSegments] = useState(s.defaultSegmentCount ?? 8);
  const [speedLimit, setSpeedLimit] = useState(s.globalSpeedLimitBytes);
  const [notifications, setNotifications] = useState(s.enableNotifications);
  const [sound, setSound] = useState(s.enableSound ?? true);
  const [soundVolume, setSoundVolume] = useState(s.soundVolume ?? 0.5);
  const [directory, setDirectory] = useState(s.defaultDirectory);
  const [directoryMessage, setDirectoryMessage] = useState<string | null>(null);
  const [theme, setTheme] = useState<'dark' | 'light'>(s.theme ?? 'dark');
  const [workStealing, setWorkStealing] = useState(s.dynamicWorkStealing ?? true);
  const [autoVerify, setAutoVerify] = useState(s.autoVerifyIntegrity ?? false);
  const [autoRetry, setAutoRetry] = useState(s.autoRetryEnabled ?? true);
  const [minimizeToTray, setMinimizeToTray] = useState(s.minimizeToTray ?? true);
  const [startWithWindows, setStartWithWindows] = useState(s.startWithWindows ?? false);
  const [clipboardMonitor, setClipboardMonitor] = useState(s.clipboardMonitorEnabled ?? false);
  const [nightScheduler, setNightScheduler] = useState(s.nightSchedulerEnabled ?? false);
  const [nightStart, setNightStart] = useState(s.nightSchedulerStartHour ?? 2);
  const [nightEnd, setNightEnd] = useState(s.nightSchedulerEndHour ?? 8);
  const [autoShutdown, setAutoShutdown] = useState(s.autoShutdownOnComplete ?? false);
  const [debridEnabled, setDebridEnabled] = useState(s.debridEnabled ?? false);
  const [debridApiKey, setDebridApiKey] = useState(s.debridApiKey ?? '');
  const [mediaTools, setMediaTools] = useState<MediaToolsStatus | null>(null);
  const [toolBusy, setToolBusy] = useState<'ffmpeg' | 'yt-dlp' | null>(null);
  const [toolProgress, setToolProgress] = useState<{ downloaded: number; total?: number } | null>(null);
  const [toolMessage, setToolMessage] = useState<string | null>(null);
  const [showCookieModal, setShowCookieModal] = useState(false);
  const [cookieText, setCookieText] = useState('');
  const [cookieLoading, setCookieLoading] = useState(false);
  const [cookieStatus, setCookieStatus] = useState<string | null>(null);

  const handleOpenCookieModal = async () => {
    setShowCookieModal(true);
    setCookieLoading(true);
    setCookieStatus(null);
    try {
      const existing = await downloadManager.loadCookiesText();
      setCookieText(existing);
    } catch (err) {
      setCookieStatus(String(err));
    } finally {
      setCookieLoading(false);
    }
  };

  const handleSaveCookies = async () => {
    setCookieLoading(true);
    setCookieStatus(null);
    try {
      let clean = cookieText.trim();
      if (clean && !clean.includes('\t') && !clean.startsWith('#')) {
        if (clean.includes('sessionid=')) {
          const match = clean.match(/sessionid=([^;\s]+)/);
          if (match) clean = match[1];
        }
        if (/^[a-zA-Z0-9%_-]{15,}$/.test(clean)) {
          clean = `# Netscape HTTP Cookie File\n# Converted by ARGON\n.instagram.com\tTRUE\t/\tTRUE\t2147483647\tsessionid\t${clean}\n`;
        }
      }
      await downloadManager.saveCookiesText(clean);
      setCookieStatus(ar ? `تم حفظ ملف الكوكيز بنجاح (${clean.length} بايت).` : `Cookies saved successfully (${clean.length} bytes).`);
    } catch (err) {
      setCookieStatus(String(err));
    } finally {
      setCookieLoading(false);
    }
  };

  useEffect(() => {
    if (android || typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return;
    void downloadManager.mediaToolsStatus().then(setMediaTools).catch(() => {});
    let unlisten: (() => void) | undefined;
    void import('@tauri-apps/api/event').then(({ listen }) =>
      listen<{ tool: string; phase: string; downloaded: number; total?: number }>('media-tool-progress', (ev) => {
        setToolProgress({ downloaded: ev.payload.downloaded, total: ev.payload.total });
      }).then((fn) => {
        unlisten = fn;
      })
    );
    return () => {
      unlisten?.();
    };
  }, [android]);

  return (
    <div className="settings-shell">
      <div className="view-header" style={{ marginBottom: 4 }}>
        <div className="view-header-title">
          <h1>{ar ? 'الإعدادات' : 'Settings'}</h1>
        </div>
      </div>

      <div className="settings-tabs" role="tablist" aria-label={ar ? 'أقسام الإعدادات' : 'Settings sections'}>
        <button type="button" role="tab" aria-selected={tab === 'general'} onClick={() => setTab('general')}>
          <Sliders size={14} style={{ display: 'inline', marginInlineEnd: 6 }} />
          {ar ? 'عام' : 'General'}
        </button>
        <button type="button" role="tab" aria-selected={tab === 'engine'} onClick={() => setTab('engine')}>
          <Cpu size={14} style={{ display: 'inline', marginInlineEnd: 6 }} />
          {ar ? 'التحميل' : 'Downloads'}
        </button>
        <button type="button" role="tab" aria-selected={tab === 'updates'} onClick={() => setTab('updates')}>
          <Shield size={14} style={{ display: 'inline', marginInlineEnd: 6 }} />
          {ar ? 'التحديثات' : 'Updates'}
        </button>
      </div>

      {tab === 'general' && (
        <>
          <section className="settings-card">
            <div className="settings-label">
              <strong style={{ display: 'flex', alignItems: 'center', gap: 7 }}>
                <FolderOpen size={15} className="accent-text" />
                {ar ? 'مجلد الحفظ' : 'Download Folder'}
              </strong>
              <span>
                {android
                  ? ar
                    ? 'تُحفظ التنزيلات وتُصدَّر تلقائياً إلى التنزيلات (Downloads/ARGON) وإلى المعرض للمقاطع والصور.'
                    : 'Downloads are published automatically to Downloads/ARGON and Gallery for media.'
                  : ar
                    ? 'المسار الكامل لحفظ التنزيلات على جهازك.'
                    : 'Directory path for saved downloads.'}
              </span>
            </div>

            {!android && (
              <>
                <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
                  <input
                    aria-label={ar ? 'مسار مجلد الحفظ' : 'Download directory'}
                    dir="ltr"
                    value={directory}
                    placeholder={s.defaultDirectory || 'Downloads'}
                    onChange={(e) => setDirectory(e.target.value)}
                    className="argon-input"
                    style={{ flex: 1, minWidth: 200, fontFamily: 'monospace' }}
                  />
                  <button
                    className="action-button primary"
                    onClick={async () => {
                      try {
                        await downloadManager.updateSettings({ defaultDirectory: directory });
                        setDirectoryMessage(ar ? 'تم حفظ المجلد والتحقق من صلاحية الكتابة.' : 'Directory saved.');
                      } catch (error) {
                        setDirectoryMessage(String(error));
                      }
                    }}
                  >
                    {ar ? 'حفظ' : 'Save'}
                  </button>
                </div>
                {directoryMessage && <p style={{ margin: 0, fontSize: 12, color: 'var(--argon-accent)' }}>{directoryMessage}</p>}
              </>
            )}
          </section>

          <section className="settings-card">
            <div className="settings-row">
              <div className="settings-label">
                <strong>{ar ? 'مظهر الواجهة' : 'Interface Theme'}</strong>
                <span>{ar ? 'وضع داكن عميق أو وضع نهاري فاتح' : 'Deep Slate Dark or Crisp Porcelain Light'}</span>
              </div>
              <div className="intake-mode-tabs">
                <button
                  type="button"
                  aria-pressed={theme === 'dark'}
                  onClick={() => {
                    setTheme('dark');
                    void downloadManager.updateSettings({ theme: 'dark' });
                  }}
                >
                  {ar ? 'داكن' : 'Dark'}
                </button>
                <button
                  type="button"
                  aria-pressed={theme === 'light'}
                  onClick={() => {
                    setTheme('light');
                    void downloadManager.updateSettings({ theme: 'light' });
                  }}
                >
                  {ar ? 'فاتح' : 'Light'}
                </button>
              </div>
            </div>

            <div className="settings-row">
              <div className="settings-label">
                <strong style={{ display: 'flex', alignItems: 'center', gap: 7 }}>
                  <Bell size={14} className="accent-text" />
                  {ar ? 'إشعارات النظام' : 'System Notifications'}
                </strong>
                <span>{ar ? 'إشعار فوري عند اكتمال التحميل' : 'Notify when downloads complete'}</span>
              </div>
              <button
                type="button"
                role="switch"
                aria-checked={notifications}
                className="argon-switch"
                onClick={() => {
                  const next = !notifications;
                  setNotifications(next);
                  void downloadManager.updateSettings({ enableNotifications: next });
                }}
              >
                <span className="argon-switch-thumb" />
              </button>
            </div>

            <div className="settings-row">
              <div className="settings-label">
                <strong style={{ display: 'flex', alignItems: 'center', gap: 7 }}>
                  <Volume2 size={14} className="accent-text" />
                  {ar ? 'المؤثرات الصوتية' : 'Sound Effects'}
                </strong>
                <span>{ar ? 'نغمات رصينة عند البدء والانتهاء' : 'Subtle audio cues on start and finish'}</span>
              </div>
              <button
                type="button"
                role="switch"
                aria-checked={sound}
                className="argon-switch"
                onClick={() => {
                  const next = !sound;
                  setSound(next);
                  void downloadManager.updateSettings({ enableSound: next });
                }}
              >
                <span className="argon-switch-thumb" />
              </button>
            </div>

            {sound && (
              <div className="settings-row">
                <span style={{ fontSize: 12, color: 'var(--argon-muted)', fontFamily: 'monospace', minWidth: 42 }}>
                  {Math.round(soundVolume * 100)}%
                </span>
                <input
                  type="range"
                  min="0"
                  max="1"
                  step="0.05"
                  value={soundVolume}
                  onChange={(e) => {
                    const vol = parseFloat(e.target.value);
                    setSoundVolume(vol);
                    void downloadManager.updateSettings({ soundVolume: vol });
                  }}
                  style={{ flex: 1, accentColor: 'var(--argon-accent)' }}
                />
              </div>
            )}

            {!android && (
              <>
                <div className="settings-row">
                  <div className="settings-label">
                    <strong>{ar ? 'البقاء في شريط النظام (Tray) عند الإغلاق' : 'Minimize to System Tray on Close'}</strong>
                    <span>{ar ? 'استمرار التقاط روابط المتصفح وإدارة الطوابير بالخلفية عند إغلاق النافذة' : 'Keep browser capture and downloads running in background'}</span>
                  </div>
                  <button
                    type="button"
                    role="switch"
                    aria-checked={minimizeToTray}
                    className="argon-switch"
                    onClick={() => {
                      const next = !minimizeToTray;
                      setMinimizeToTray(next);
                      void downloadManager.updateSettings({ minimizeToTray: next });
                    }}
                  >
                    <span className="argon-switch-thumb" />
                  </button>
                </div>

                <div className="settings-row">
                  <div className="settings-label">
                    <strong>{ar ? 'التشغيل التلقائي مع بدء Windows' : 'Start ARGON with Windows'}</strong>
                    <span>{ar ? 'تشغيل ARGON مصغراً في شريط النظام عند تسجيل الدخول' : 'Launch ARGON minimized to tray on Windows sign-in'}</span>
                  </div>
                  <button
                    type="button"
                    role="switch"
                    aria-checked={startWithWindows}
                    className="argon-switch"
                    onClick={() => {
                      const next = !startWithWindows;
                      setStartWithWindows(next);
                      void downloadManager.updateSettings({ startWithWindows: next }).catch((err) => {
                        setStartWithWindows(!next);
                        setDirectoryMessage(String(err));
                      });
                    }}
                  >
                    <span className="argon-switch-thumb" />
                  </button>
                </div>
              </>
            )}
          </section>
        </>
      )}

      {tab === 'engine' && (
        <section className="settings-card">
          <div className="settings-row">
            <div className="settings-label">
              <strong>{ar ? 'التحميلات المتزامنة' : 'Max Concurrent Downloads'}</strong>
              <span>{ar ? 'عدد الملفات التي تُنزّل في وقت واحد' : 'Simultaneous active downloads'}</span>
            </div>
            <div className="segment-pills" style={{ width: 'min(260px, 100%)' }}>
              {[1, 2, 3, 5].map((count) => (
                <button
                  key={count}
                  type="button"
                  className={`segment-pill-btn ${maxConcurrent === count ? 'is-active' : ''}`}
                  onClick={() => {
                    setMaxConcurrent(count);
                    void downloadManager.updateSettings({ maxConcurrentDownloads: count });
                  }}
                >
                  {count}
                </button>
              ))}
            </div>
          </div>

          <div className="settings-row">
            <div className="settings-label">
              <strong>{ar ? 'عدد المسارات الافتراضي' : 'Default Connections per File'}</strong>
              <span>{ar ? 'تقسيم الملف إلى اتصالات متوازية لتسريع التنزيل' : 'Parallel byte-range streams per download'}</span>
            </div>
            <div className="segment-pills" style={{ width: 'min(280px, 100%)' }}>
              {[1, 4, 8, 16].map((seg) => (
                <button
                  key={seg}
                  type="button"
                  className={`segment-pill-btn ${defaultSegments === seg ? 'is-active' : ''}`}
                  onClick={() => {
                    setDefaultSegments(seg);
                    void downloadManager.updateSettings({ defaultSegmentCount: seg });
                  }}
                >
                  {seg}
                </button>
              ))}
            </div>
          </div>

          <div className="settings-row">
            <div className="settings-label">
              <strong>{ar ? 'سقف السرعة العام' : 'Global Speed Limit'}</strong>
              <span>{ar ? 'تقييد سرعة الشبكة الكلية للبرنامج' : 'Cap aggregate bandwidth used by ARGON'}</span>
            </div>
            <div className="segment-pills" style={{ width: 'min(300px, 100%)' }}>
              {[
                { label: ar ? 'بلا حد' : 'MAX', val: 0 },
                { label: '2 MB/s', val: 2 * 1024 * 1024 },
                { label: '5 MB/s', val: 5 * 1024 * 1024 },
                { label: '10 MB/s', val: 10 * 1024 * 1024 }
              ].map((opt) => (
                <button
                  key={opt.val}
                  type="button"
                  className={`segment-pill-btn ${speedLimit === opt.val ? 'is-active' : ''}`}
                  onClick={() => {
                    setSpeedLimit(opt.val);
                    void downloadManager.updateSettings({ globalSpeedLimitBytes: opt.val });
                  }}
                >
                  {opt.label}
                </button>
              ))}
            </div>
          </div>

          <div className="settings-row">
            <div className="settings-label">
              <strong>{ar ? 'التقسيم الديناميكي الذكي (Work-Stealing)' : 'Dynamic Segment Splitting'}</strong>
              <span>
                {ar
                  ? 'إعادة توزيع الأجزاء المتبقية تلقائياً عند انتهاء المسارات الأسرع (تقنية IDM)'
                  : 'Automatically split remaining slow ranges among idle fast connections'}
              </span>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={workStealing}
              className="argon-switch"
              onClick={() => {
                const next = !workStealing;
                setWorkStealing(next);
                void downloadManager.updateSettings({ dynamicWorkStealing: next });
              }}
            >
              <span className="argon-switch-thumb" />
            </button>
          </div>

          <div className="settings-row">
            <div className="settings-label">
              <strong>{ar ? 'فحص SHA-256 التلقائي عند الاكتمال' : 'Auto-Verify SHA-256 on Complete'}</strong>
              <span>{ar ? 'حساب البصمة التشفيرية لكل ملف فور انتهائه' : 'Automatically compute file SHA-256 checksum on completion'}</span>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={autoVerify}
              className="argon-switch"
              onClick={() => {
                const next = !autoVerify;
                setAutoVerify(next);
                void downloadManager.updateSettings({ autoVerifyIntegrity: next });
              }}
            >
              <span className="argon-switch-thumb" />
            </button>
          </div>

          <div className="settings-row">
            <div className="settings-label">
              <strong>{ar ? 'الحد الأقصى للمهام المتزامنة لكل موقع' : 'Max Downloads per Domain'}</strong>
              <span>{ar ? 'حماية من الحظر المؤقت (429) عند تحميل عدة ملفات من نفس الخادم' : 'Prevent rate-limiting (HTTP 429) on the same host'}</span>
            </div>
            <div className="segment-pills" style={{ width: 'min(260px, 100%)' }}>
              {[1, 2, 4, 8].map((count) => (
                <button
                  key={count}
                  type="button"
                  className={`segment-pill-btn ${maxPerHost === count ? 'is-active' : ''}`}
                  onClick={() => {
                    setMaxPerHost(count);
                    void downloadManager.updateSettings({ maxConnectionsPerHost: count });
                  }}
                >
                  {count}
                </button>
              ))}
            </div>
          </div>

          <div className="settings-row">
            <div className="settings-label">
              <strong>{ar ? 'إعادة المحاولة التلقائية عند الانقطاع' : 'Auto-Retry Failed Transfers'}</strong>
              <span>{ar ? 'إعادة جدولة التنزيل تلقائياً مع تراجع أسي عند انقطاع الشبكة' : 'Automatically retry transient network drops with exponential backoff'}</span>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={autoRetry}
              className="argon-switch"
              onClick={() => {
                const next = !autoRetry;
                setAutoRetry(next);
                void downloadManager.updateSettings({ autoRetryEnabled: next });
              }}
            >
              <span className="argon-switch-thumb" />
            </button>
          </div>

          <div className="settings-row">
            <div className="settings-label">
              <strong>{ar ? 'مراقبة الحافظة عند التركيز' : 'Smart Clipboard Detection'}</strong>
              <span>{ar ? 'التقاط روابط الملفات المنسوخة تلقائياً عند فتح التطبيق' : 'Suggest direct download URLs from clipboard when focused'}</span>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={clipboardMonitor}
              className="argon-switch"
              onClick={() => {
                const next = !clipboardMonitor;
                setClipboardMonitor(next);
                void downloadManager.updateSettings({ clipboardMonitorEnabled: next });
              }}
            >
              <span className="argon-switch-thumb" />
            </button>
          </div>

          {!android && (
            <div style={{ marginTop: 14, paddingTop: 14, borderTop: '1px solid var(--argon-line)', display: 'grid', gap: 10 }}>
              <div className="settings-row">
                <div className="settings-label">
                  <strong style={{ display: 'flex', alignItems: 'center', gap: 7 }}>
                    <Film size={15} className="accent-text" />
                    {ar ? 'محرك الوسائط و FFmpeg (1080p/4K و HLS/DASH)' : 'Media Engine & FFmpeg (HD Mux & HLS/DASH)'}
                  </strong>
                  <span>
                    {mediaTools
                      ? `yt-dlp: ${mediaTools.hasYtdlp ? '✓' : '✗'} · Node: ${mediaTools.hasNode ? '✓' : '✗'} · FFmpeg: ${mediaTools.hasFfmpeg ? '✓' : '✗'}`
                      : ar ? 'تفعيل دمج الصوت والصورة للجودات العالية وقوائم البث' : 'Enable audio/video muxing for high resolutions and HLS/DASH playlists'}
                  </span>
                </div>
                <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
                  <button
                    type="button"
                    className="action-button primary"
                    disabled={toolBusy !== null}
                    onClick={async () => {
                      setToolBusy('ffmpeg');
                      setToolMessage(null);
                      setToolProgress(null);
                      try {
                        const status = await downloadManager.installFfmpegTool();
                        setMediaTools(status);
                        setToolMessage(ar ? 'تم تنزيل وتفعيل FFmpeg بنجاح بعد التحقق من SHA-256.' : 'FFmpeg verified and installed.');
                      } catch (err) {
                        setToolMessage(String(err));
                      } finally {
                        setToolBusy(null);
                        setToolProgress(null);
                      }
                    }}
                  >
                    <Download size={14} />
                    <span>
                      {toolBusy === 'ffmpeg'
                        ? ar ? 'جارٍ التنزيل…' : 'Installing…'
                        : mediaTools?.hasFfmpeg
                          ? ar ? 'تحديث FFmpeg' : 'Reinstall FFmpeg'
                          : ar ? 'تنزيل وتفعيل FFmpeg' : 'Install FFmpeg'}
                    </span>
                  </button>
                  <button
                    type="button"
                    className="action-button"
                    disabled={toolBusy !== null}
                    onClick={async () => {
                      setToolBusy('yt-dlp');
                      setToolMessage(null);
                      setToolProgress(null);
                      try {
                        const status = await downloadManager.updateYtdlpTool();
                        setMediaTools(status);
                        setToolMessage(ar ? 'تم تحديث yt-dlp والتحقق من SHA-256.' : 'yt-dlp updated and verified.');
                      } catch (err) {
                        setToolMessage(String(err));
                      } finally {
                        setToolBusy(null);
                        setToolProgress(null);
                      }
                    }}
                  >
                    <RefreshCw size={14} className={toolBusy === 'yt-dlp' ? 'animate-spin' : ''} />
                    <span>{ar ? 'تحديث yt-dlp' : 'Update yt-dlp'}</span>
                  </button>
                </div>
              </div>
              {toolProgress && (
                <progress
                  value={toolProgress.total ? toolProgress.downloaded : undefined}
                  max={toolProgress.total || undefined}
                  style={{ width: '100%' }}
                />
              )}
              {toolMessage && <p style={{ margin: 0, fontSize: 12, color: 'var(--argon-accent)' }}>{toolMessage}</p>}
              <div style={{ marginTop: 10, padding: '12px 14px', background: 'var(--argon-card-bg)', border: '1px solid var(--argon-border)', borderRadius: 8, fontSize: 12.5, lineHeight: 1.5, display: 'flex', flexDirection: 'column', gap: 10 }}>
                <div style={{ display: 'flex', alignItems: 'flex-start', gap: 8 }}>
                  <Key size={15} style={{ flexShrink: 0, marginTop: 2, color: 'var(--argon-accent)' }} />
                  <div>
                    <strong style={{ display: 'block', marginBottom: 2 }}>
                      {ar ? 'كوكيز الحسابات الخاصة والمقيدة (Instagram & TikTok)' : 'Account Cookies (Instagram & TikTok)'}
                    </strong>
                    <span style={{ opacity: 0.85 }}>
                      {ar
                        ? 'لتنزيل القصص والريلز وحسابات إنستغرام الخاصة التي تتابعها، يمكنك حفظ ملف cookies.txt أو رمز sessionid هنا لتخطي فحص الحساب الخاص بأمان.'
                        : 'To download private Instagram stories, restricted reels, or locked TikTok videos, save your cookies.txt or sessionid here.'}
                    </span>
                  </div>
                </div>
                <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
                  <button
                    type="button"
                    className="action-button primary"
                    onClick={() => void handleOpenCookieModal()}
                    style={{ fontSize: 12, padding: '5px 12px' }}
                  >
                    <Key size={13} />
                    <span>{ar ? 'إدارة كوكيز cookies.txt' : 'Manage cookies.txt'}</span>
                  </button>
                  <button
                    type="button"
                    className="action-button"
                    onClick={() => void downloadManager.openMediaSniffer('https://www.instagram.com')}
                    style={{ fontSize: 12, padding: '5px 12px' }}
                  >
                    <ExternalLink size={13} />
                    <span>{ar ? 'فتح متصفح الاقتناص المدمج' : 'Open Media Sniffer Browser'}</span>
                  </button>
                </div>
              </div>
            </div>
          )}

          <div className="settings-row">
            <div className="settings-label">
              <strong>{ar ? 'تكامل Real-Debrid لفك الروابط المقيدة والتورنت' : 'Real-Debrid Unrestriction'}</strong>
              <span>
                {ar
                  ? 'تحويل روابط Rapidgator و 1fichier و Mega والتورنت إلى روابط مباشرة فائقة السرعة'
                  : 'Convert locked filehosts and torrents into direct high-speed downloads'}
              </span>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={debridEnabled}
              className="argon-switch"
              onClick={() => {
                const next = !debridEnabled;
                setDebridEnabled(next);
                void downloadManager.updateSettings({ debridEnabled: next });
              }}
            >
              <span className="argon-switch-thumb" />
            </button>
          </div>

          {debridEnabled && (
            <div className="settings-row">
              <div className="settings-label">
                <strong>{ar ? 'مفتاح API الخاص بـ Real-Debrid' : 'Real-Debrid API Token'}</strong>
                <span>{ar ? 'من إعدادات حسابك في real-debrid.com/apitoken' : 'From real-debrid.com/apitoken'}</span>
              </div>
              <input
                type="password"
                className="argon-input"
                style={{ width: 'min(300px, 100%)', padding: '6px 10px', fontSize: '0.85rem' }}
                placeholder={ar ? 'أدخل مفتاح API هنا...' : 'Enter API token...'}
                value={debridApiKey}
                onChange={(e) => {
                  setDebridApiKey(e.target.value);
                  void downloadManager.updateSettings({ debridApiKey: e.target.value.trim() });
                }}
              />
            </div>
          )}

          <div className="settings-row">
            <div className="settings-label">
              <strong>{ar ? 'الجدول الليلي للتنزيلات' : 'Night Scheduler Window'}</strong>
              <span>{ar ? 'حصر التنزيلات المجدولة خلال الساعات المحددة' : 'Restrict queued downloads to configured hours'}</span>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={nightScheduler}
              className="argon-switch"
              onClick={() => {
                const next = !nightScheduler;
                setNightScheduler(next);
                void downloadManager.updateSettings({ nightSchedulerEnabled: next });
              }}
            >
              <span className="argon-switch-thumb" />
            </button>
          </div>

          {nightScheduler && (
            <div className="settings-row">
              <div className="settings-label">
                <strong>{ar ? 'ساعات النشاط' : 'Active Hours'}</strong>
              </div>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <select
                  className="argon-select"
                  value={nightStart}
                  onChange={(e) => {
                    const v = Number(e.target.value);
                    setNightStart(v);
                    void downloadManager.updateSettings({ nightSchedulerStartHour: v });
                  }}
                >
                  {Array.from({ length: 24 }, (_, h) => (
                    <option key={h} value={h}>
                      {String(h).padStart(2, '0')}:00
                    </option>
                  ))}
                </select>
                <span>→</span>
                <select
                  className="argon-select"
                  value={nightEnd}
                  onChange={(e) => {
                    const v = Number(e.target.value);
                    setNightEnd(v);
                    void downloadManager.updateSettings({ nightSchedulerEndHour: v });
                  }}
                >
                  {Array.from({ length: 24 }, (_, h) => (
                    <option key={h} value={h}>
                      {String(h).padStart(2, '0')}:00
                    </option>
                  ))}
                </select>
              </div>
            </div>
          )}

          {!android && (
            <div className="settings-row">
              <div className="settings-label">
                <strong>{ar ? 'إيقاف تشغيل الكمبيوتر بعد الانتهاء' : 'Shut Down PC When Complete'}</strong>
              </div>
              <button
                type="button"
                role="switch"
                aria-checked={autoShutdown}
                className="argon-switch"
                onClick={() => {
                  const next = !autoShutdown;
                  setAutoShutdown(next);
                  void downloadManager.updateSettings({ autoShutdownOnComplete: next });
                }}
              >
                <span className="argon-switch-thumb" />
              </button>
            </div>
          )}
        </section>
      )}

      {tab === 'updates' && (
        <div style={{ display: 'grid', gap: 16 }}>
          <UpdatesPanel lang={lang} />
          <SharingSettings lang={lang} />
        </div>
      )}

      {showCookieModal && (
        <div
          className="dialog-backdrop"
          onClick={() => setShowCookieModal(false)}
          style={{
            position: 'fixed',
            inset: 0,
            background: 'rgba(0,0,0,0.6)',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            zIndex: 9999,
            padding: 16
          }}
        >
          <div
            className="glass-panel"
            role="dialog"
            aria-modal="true"
            onClick={(e) => e.stopPropagation()}
            style={{
              width: 'min(620px, 95vw)',
              maxHeight: '90vh',
              display: 'flex',
              flexDirection: 'column',
              padding: 24,
              borderRadius: 14,
              background: 'var(--argon-card-bg, #1a1e28)',
              border: '1px solid var(--argon-border, #333)',
              boxShadow: '0 25px 50px -12px rgba(0, 0, 0, 0.65)'
            }}
          >
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16 }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <Key size={18} className="accent-text" />
                <h3 style={{ margin: 0, fontSize: 16 }}>
                  {ar ? 'إدارة كوكيز الحسابات (cookies.txt)' : 'Account Cookies Manager'}
                </h3>
              </div>
              <button
                type="button"
                className="action-button icon-only"
                onClick={() => setShowCookieModal(false)}
                title={ar ? 'إغلاق' : 'Close'}
              >
                <X size={16} />
              </button>
            </div>

            <p style={{ margin: '0 0 12px', fontSize: 12.5, opacity: 0.85, lineHeight: 1.5 }}>
              {ar
                ? 'تتيح الكوكيز لـ ARGON تنزيل القصص (Stories)، والريلز المقيدة، وحسابات إنستغرام الخاصة التي تتابعها، بالإضافة لمقاطع تيك توك المقفلة بدون أي مشاكل. الصق محتوى ملف cookies.txt بصيغة Netscape أو الصق رمز sessionid الخاص بإنستغرام مباشرة:'
                : 'Cookies allow downloading Stories, restricted Reels, private accounts you follow, and locked TikTok videos. Paste your Netscape cookies.txt content or paste your Instagram sessionid token directly:'}
            </p>

            <textarea
              dir="ltr"
              value={cookieText}
              onChange={(e) => setCookieText(e.target.value)}
              placeholder={ar ? '# الصق كوكيز Netscape هنا أو الصق sessionid الخاص بـ Instagram...' : '# Paste Netscape cookies here or your Instagram sessionid...'}
              className="argon-input"
              rows={9}
              style={{
                width: '100%',
                fontFamily: 'Consolas, monospace',
                fontSize: 12,
                lineHeight: 1.4,
                resize: 'vertical',
                minHeight: 160,
                maxHeight: 300,
                marginBottom: 12,
                background: 'rgba(0,0,0,0.25)',
                color: 'inherit'
              }}
            />

            {cookieStatus && (
              <div
                style={{
                  marginBottom: 12,
                  padding: '8px 12px',
                  borderRadius: 6,
                  fontSize: 12,
                  background: cookieStatus.includes('نجاح') || cookieStatus.includes('successfully') ? 'rgba(34,197,94,0.15)' : 'rgba(239,68,68,0.15)',
                  color: cookieStatus.includes('نجاح') || cookieStatus.includes('successfully') ? '#4ade80' : '#f87171'
                }}
              >
                {cookieStatus}
              </div>
            )}

            <div style={{ display: 'flex', gap: 8, justifyContent: 'space-between', flexWrap: 'wrap', alignItems: 'center' }}>
              <button
                type="button"
                className="action-button"
                onClick={() => {
                  setCookieText('');
                  void downloadManager.saveCookiesText('');
                  setCookieStatus(ar ? 'تم مسح الكوكيز.' : 'Cookies cleared.');
                }}
                disabled={cookieLoading}
                style={{ fontSize: 12 }}
              >
                {ar ? 'مسح الكل' : 'Clear'}
              </button>
              <div style={{ display: 'flex', gap: 8 }}>
                <button
                  type="button"
                  className="action-button"
                  onClick={() => setShowCookieModal(false)}
                  style={{ fontSize: 12 }}
                >
                  {ar ? 'إلغاء' : 'Cancel'}
                </button>
                <button
                  type="button"
                  className="action-button primary"
                  onClick={() => void handleSaveCookies()}
                  disabled={cookieLoading}
                  style={{ fontSize: 12 }}
                >
                  {cookieLoading ? (ar ? 'جارٍ الحفظ...' : 'Saving...') : (ar ? 'حفظ وتطبيق الكوكيز' : 'Save & Apply Cookies')}
                </button>
              </div>
            </div>
          </div>
        </div>
      )}
    </div>
  );
};
