import React, { useEffect, useState } from 'react';
import { FolderOpen, Bell, Volume2, Sliders, Cpu, Clock, Shield } from 'lucide-react';
import { downloadManager } from '../core/downloadManager';
import { UpdatesPanel } from './UpdatesPanel';
import { SharingSettings } from './FlowPanels';
import { CURRENT_VERSION } from './Header';
import { isAndroidRuntime } from '../core/safeArea';

export type SettingsTab = 'general' | 'engine' | 'scheduler' | 'updates';

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
  const [clipboardMonitor, setClipboardMonitor] = useState(s.clipboardMonitorEnabled ?? false);
  const [nightScheduler, setNightScheduler] = useState(s.nightSchedulerEnabled ?? false);
  const [nightStart, setNightStart] = useState(s.nightSchedulerStartHour ?? 2);
  const [nightEnd, setNightEnd] = useState(s.nightSchedulerEndHour ?? 8);
  const [autoShutdown, setAutoShutdown] = useState(s.autoShutdownOnComplete ?? false);

  return (
    <div className="settings-shell">
      <div className="view-header" style={{ marginBottom: 4 }}>
        <div className="view-heading">
          <span className="eyebrow">{ar ? 'إدارة النظام' : 'SYSTEM PREFERENCES'}</span>
          <div className="view-header-title">
            <h1>{ar ? 'الإعدادات' : 'Settings'}</h1>
            <span className="count-badge" dir="ltr">
              v{CURRENT_VERSION}
            </span>
          </div>
          <p className="view-description">
            {ar
              ? 'إدارة التخزين، تسريع الاتصال، الطوابير المجدولة، والتحديثات الموقّعة.'
              : 'Configure storage, multi-segment acceleration, scheduled queues, and signed updates.'}
          </p>
        </div>
      </div>

      <div className="settings-tabs" role="tablist" aria-label={ar ? 'أقسام الإعدادات' : 'Settings sections'}>
        <button type="button" role="tab" aria-selected={tab === 'general'} onClick={() => setTab('general')}>
          <Sliders size={14} style={{ display: 'inline', marginInlineEnd: 6 }} />
          {ar ? 'العام والمظهر' : 'General & UI'}
        </button>
        <button type="button" role="tab" aria-selected={tab === 'engine'} onClick={() => setTab('engine')}>
          <Cpu size={14} style={{ display: 'inline', marginInlineEnd: 6 }} />
          {ar ? 'المحرك والتسريع' : 'Engine & Speed'}
        </button>
        <button type="button" role="tab" aria-selected={tab === 'scheduler'} onClick={() => setTab('scheduler')}>
          <Clock size={14} style={{ display: 'inline', marginInlineEnd: 6 }} />
          {ar ? 'الجدولة والطوابير' : 'Scheduler'}
        </button>
        <button type="button" role="tab" aria-selected={tab === 'updates'} onClick={() => setTab('updates')}>
          <Shield size={14} style={{ display: 'inline', marginInlineEnd: 6 }} />
          {ar ? 'التحديثات والمشاركة' : 'Updates & Flow'}
        </button>
      </div>

      {tab === 'general' && (
        <>
          <section className="settings-card">
            <div className="settings-label">
              <strong style={{ display: 'flex', alignItems: 'center', gap: 7 }}>
                <FolderOpen size={15} className="accent-text" />
                {ar ? 'مجلد الحفظ الافتراضي' : 'Download Destination'}
              </strong>
              <span>
                {android
                  ? ar
                    ? 'على Android يُدار المسار تلقائياً عبر نظام التخزين الآمن للتطبيق.'
                    : 'On Android, destination storage is managed safely by the app sandbox.'
                  : ar
                    ? 'حدد المسار الكامل لحفظ التنزيلات.'
                    : 'Set the full directory path for saved downloads.'}
              </span>
            </div>

            <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
              <input
                aria-label={ar ? 'مسار مجلد الحفظ' : 'Download directory'}
                dir="ltr"
                value={directory}
                disabled={android}
                placeholder={s.defaultDirectory || 'Downloads'}
                onChange={(e) => setDirectory(e.target.value)}
                className="argon-input"
                style={{ flex: 1, minWidth: 200, fontFamily: 'monospace' }}
              />
              {!android && (
                <button
                  className="action-button primary"
                  onClick={async () => {
                    try {
                      await downloadManager.updateSettings({ defaultDirectory: directory });
                      setDirectoryMessage(ar ? 'تم حفظ المجلد والتحقق من صلاحية الكتابة.' : 'Directory saved and verified.');
                    } catch (error) {
                      setDirectoryMessage(String(error));
                    }
                  }}
                >
                  {ar ? 'حفظ المسار' : 'Save Path'}
                </button>
              )}
            </div>
            {directoryMessage && <p style={{ margin: 0, fontSize: 12, color: 'var(--argon-accent)' }}>{directoryMessage}</p>}
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
        </section>
      )}

      {tab === 'scheduler' && (
        <section className="settings-card">
          <div className="settings-row">
            <div className="settings-label">
              <strong>{ar ? 'تفعيل نافذة الجدول الليلي' : 'Enable Night Scheduler Window'}</strong>
              <span>
                {ar
                  ? 'حصر بدء التنزيلات المجدولة خلال الساعات المحددة فقط'
                  : 'Restrict queued downloads to the configured off-peak hours'}
              </span>
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

          <div className="settings-row">
            <div className="settings-label">
              <strong>{ar ? 'ساعات النشاط المجدولة' : 'Active Schedule Window'}</strong>
              <span>{ar ? 'تُطبّق أيضاً على مهام الطابور الليلي' : 'Also governs Night Queue tasks'}</span>
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

          {!android && (
            <div className="settings-row">
              <div className="settings-label">
                <strong>{ar ? 'إيقاف تشغيل الكمبيوتر بعد الانتهاء' : 'Shut Down PC When All Complete'}</strong>
                <span>{ar ? 'إيقاف التشغيل الآمن فور اكتمال جميع المهام في الطابور' : 'Trigger safe OS shutdown once every queued transfer completes'}</span>
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
    </div>
  );
};
