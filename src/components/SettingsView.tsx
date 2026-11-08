import React, { useState } from 'react';
import {
  FolderOpen,
  Bell,
  Volume2,
  RefreshCw,
  CheckCircle2,
  Sliders,
  Shield,
  } from 'lucide-react';
import { downloadManager } from '../core/downloadManager';
import { fetchReleaseInfo, isNewerVersion, type ReleaseInfo } from '../core/updates/releaseFeed';
import { CURRENT_VERSION } from './Header';

interface SettingsViewProps {
  lang: 'ar' | 'en';
}

export const SettingsView: React.FC<SettingsViewProps> = ({ lang }) => {
  const isAr = lang === 'ar';
  const currentSettings = downloadManager.getSettings();

  const [maxConcurrent, setMaxConcurrent] = useState(currentSettings.maxConcurrentDownloads);
  const [speedLimit, setSpeedLimit] = useState(currentSettings.globalSpeedLimitBytes);
  const [notifications, setNotifications] = useState(currentSettings.enableNotifications);
  const [sound, setSound] = useState(currentSettings.enableSound ?? true);
  const [soundVolume, setSoundVolume] = useState(currentSettings.soundVolume ?? 0.5);
  const [isCheckingUpdate, setIsCheckingUpdate] = useState(false);
  const [updateInfo, setUpdateInfo] = useState<{ hasUpdate: boolean; version?: string } | null>(null);
  const [releaseInfo, setReleaseInfo] = useState<ReleaseInfo | null>(null);
  const [updateError, setUpdateError] = useState<string | null>(null);
  const [directory, setDirectory] = useState(currentSettings.defaultDirectory);
  const [directoryMessage, setDirectoryMessage] = useState<string | null>(null);

  const handleMaxConcurrentChange = (limit: number) => {
    setMaxConcurrent(limit);
    downloadManager.updateSettings({ maxConcurrentDownloads: limit });
  };

  const handleSpeedLimitChange = (bytesPerSec: number) => {
    setSpeedLimit(bytesPerSec);
    downloadManager.updateSettings({ globalSpeedLimitBytes: bytesPerSec });
  };

  const handleNotificationToggle = () => {
    const next = !notifications;
    setNotifications(next);
    downloadManager.updateSettings({ enableNotifications: next });
  };

  const handleSoundToggle = () => {
    const next = !sound;
    setSound(next);
    downloadManager.updateSettings({ enableSound: next });
  };

  const handleVolumeChange = (vol: number) => {
    setSoundVolume(vol);
    downloadManager.updateSettings({ soundVolume: vol });
  };

  const handleCheckUpdates = async () => {
    setIsCheckingUpdate(true);
    setUpdateInfo(null);
    setUpdateError(null);
    setReleaseInfo(null);
    try {
      const release = await fetchReleaseInfo();
      if (release && isNewerVersion(release.version, CURRENT_VERSION)) {
        setUpdateInfo({ hasUpdate: true, version: release.version });
        setReleaseInfo(release);
      } else {
        setUpdateInfo({ hasUpdate: false });
      }
    } catch (error) {
      setUpdateError(isAr ? String(error) : 'Could not check published releases. Check your connection and try again.');
    } finally {
      setIsCheckingUpdate(false);
    }
  };

  return (
    <div className="max-w-2xl mx-auto space-y-4 py-2 animate-in-fade">
      <div className="text-center py-4" dir="ltr">
        <div className="text-xl font-bold tracking-widest text-zinc-100">ARGON</div>
        <div className="mt-1 text-xs font-semibold tracking-[0.2em] text-[#b6d99c]">BY Otium</div>
      </div>
      {/* Title */}
      <div className="flex items-center justify-between pb-3 border-b border-white/[0.06]">
        <div>
          <h2 className="text-base sm:text-lg font-bold text-zinc-100 flex items-center gap-2">
            <Sliders className="w-5 h-5 text-[#b6d99c]" />
            <span>{isAr ? 'إعدادات التطبيق' : 'Settings'}</span>
          </h2>
          <p className="text-xs text-zinc-400 mt-0.5">
            {isAr ? 'تخصيص مسار الحفظ، سرعة التنزيل، والمؤثرات' : 'Customize storage, download speed, and audio'}
          </p>
        </div>
        <span className="text-xs font-mono text-zinc-400 px-2 py-1 rounded-lg bg-white/[0.03] border border-white/[0.06]">
          v{CURRENT_VERSION}
        </span>
      </div>

      {/* 1. Storage Location */}
      <div className="bg-[#121212] border border-white/[0.06] rounded-lg p-4 transition-all">
        <div className="flex items-center justify-between mb-2">
          <div className="flex items-center gap-2 text-xs font-semibold text-zinc-200">
            <FolderOpen className="w-4 h-4 text-[#b6d99c]" />
            <span>{isAr ? 'مجلد الحفظ الافتراضي' : 'Download Destination'}</span>
          </div>
        </div>
        <div className="flex items-center gap-2">
          <input aria-label={isAr ? 'مسار مجلد الحفظ' : 'Download directory'} dir="ltr" value={directory}
            placeholder={currentSettings.defaultDirectory || 'Downloads'} onChange={(event) => setDirectory(event.target.value)}
            className="min-w-0 flex-1 bg-[#080808] border border-white/[0.06] rounded-xl px-3 py-2 text-xs font-mono text-zinc-300" />
          <button className="px-3 py-2 rounded-xl bg-white/[0.06] text-xs" onClick={async () => {
            try { await downloadManager.updateSettings({ defaultDirectory: directory }); setDirectoryMessage(isAr ? 'تم حفظ المجلد' : 'Directory saved'); }
            catch (error) { setDirectoryMessage(String(error)); }
          }}>{isAr ? 'حفظ' : 'Save'}</button>
        </div>
        {directoryMessage && <p className="mt-2 text-xs text-zinc-400">{directoryMessage}</p>}
        <button className="mt-3 text-xs text-[#b6d99c]" onClick={async () => { try { await downloadManager.openBrowserExtensionDir(); } catch (error) { setDirectoryMessage(String(error)); } }}>
          {isAr ? 'فتح مجلد إضافة المتصفح' : 'Open browser extension folder'}
        </button>
      </div>

      {/* 2. Concurrency & Speed Limit */}
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
        {/* Max Concurrent */}
        <div className="bg-[#121212] border border-white/[0.06] rounded-lg p-4">
          <div className="flex items-center justify-between mb-2.5">
            <span className="text-xs font-semibold text-zinc-200">
              {isAr ? 'التحميلات المتزامنة' : 'Max Concurrent'}
            </span>
            <span className="text-xs font-bold text-[#b6d99c] font-mono">{maxConcurrent}</span>
          </div>
          <div className="grid grid-cols-4 gap-1.5 font-mono text-xs">
            {[1, 2, 3, 5].map((count) => (
              <button
                key={count}
                onClick={() => handleMaxConcurrentChange(count)}
                className={`py-2 rounded-xl border text-center font-bold transition-all active:scale-95 ${
                  maxConcurrent === count
                    ? 'border-[#b6d99c] text-zinc-950 bg-[#b6d99c] '
                    : 'border-white/[0.06] text-zinc-400 hover:text-zinc-200 hover:bg-white/[0.04]'
                }`}
              >
                {count}
              </button>
            ))}
          </div>
        </div>

        {/* Speed Limit */}
        <div className="bg-[#121212] border border-white/[0.06] rounded-lg p-4">
          <div className="flex items-center justify-between mb-2.5">
            <span className="text-xs font-semibold text-zinc-200">
              {isAr ? 'محدد السرعة' : 'Speed Limit'}
            </span>
            <span className="text-xs font-bold text-[#b6d99c] font-mono">
              {speedLimit === 0 ? (isAr ? 'أقصى سرعة ⚡' : 'Unlimited ⚡') : `${speedLimit / (1024 * 1024)} MB/s`}
            </span>
          </div>
          <div className="grid grid-cols-4 gap-1.5 font-mono text-xs">
            {[
              { label: isAr ? 'حر' : 'MAX', val: 0 },
              { label: '2M', val: 2 * 1024 * 1024 },
              { label: '5M', val: 5 * 1024 * 1024 },
              { label: '10M', val: 10 * 1024 * 1024 }
            ].map((opt) => (
              <button
                key={opt.val}
                onClick={() => handleSpeedLimitChange(opt.val)}
                className={`py-2 rounded-xl border text-center font-bold transition-all active:scale-95 ${
                  speedLimit === opt.val
                    ? 'border-[#b6d99c] text-zinc-950 bg-[#b6d99c] '
                    : 'border-white/[0.06] text-zinc-400 hover:text-zinc-200 hover:bg-white/[0.04]'
                }`}
              >
                {opt.label}
              </button>
            ))}
          </div>
        </div>
      </div>

      {/* 3. Notifications & Sounds */}
      <div className="bg-[#121212] border border-white/[0.06] rounded-lg p-4 space-y-3.5">
        {/* Notifications */}
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2.5">
            <Bell className="w-4 h-4 text-zinc-400" />
            <div>
              <span className="text-xs font-semibold text-zinc-200 block">
                {isAr ? 'إشعارات النظام' : 'System Notifications'}
              </span>
              <span className="text-[11px] text-zinc-500">
                {isAr ? 'تنبيه فوري عند اكتمال التحميل' : 'Notify when downloads finish'}
              </span>
            </div>
          </div>
          <button
            onClick={handleNotificationToggle}
            className={`w-11 h-6 rounded-full transition-colors p-0.5 ${
              notifications ? 'bg-[#b6d99c]' : 'bg-white/[0.1]'
            }`}
          >
            <div
              className={`w-5 h-5 rounded-full bg-[#080808] transition-transform ${
                notifications ? 'translate-x-5 rtl:-translate-x-5' : 'translate-x-0'
              }`}
            />
          </button>
        </div>

        <div className="h-px bg-white/[0.04]" />

        {/* Sound Effects */}
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2.5">
            <Volume2 className="w-4 h-4 text-zinc-400" />
            <div>
              <span className="text-xs font-semibold text-zinc-200 block">
                {isAr ? 'المؤثرات الصوتية' : 'Sound Effects'}
              </span>
              <span className="text-[11px] text-zinc-500">
                {isAr ? 'نغمات صوتية عند البدء والانتهاء' : 'Audio chimes on start and complete'}
              </span>
            </div>
          </div>
          <button
            onClick={handleSoundToggle}
            className={`w-11 h-6 rounded-full transition-colors p-0.5 ${
              sound ? 'bg-[#b6d99c]' : 'bg-white/[0.1]'
            }`}
          >
            <div
              className={`w-5 h-5 rounded-full bg-[#080808] transition-transform ${
                sound ? 'translate-x-5 rtl:-translate-x-5' : 'translate-x-0'
              }`}
            />
          </button>
        </div>

        {sound && (
          <div className="pt-1 flex items-center gap-3">
            <span className="text-[11px] text-zinc-400 font-mono w-16">
              {Math.round(soundVolume * 100)}%
            </span>
            <input
              type="range"
              min="0"
              max="1"
              step="0.05"
              value={soundVolume}
              onChange={(e) => handleVolumeChange(parseFloat(e.target.value))}
              className="flex-1 accent-[#b6d99c] h-1.5 bg-black/50 rounded-lg cursor-pointer"
            />
          </div>
        )}
      </div>

      {/* 4. Updates & Verification */}
      <div className="bg-[#121212] border border-white/[0.06] rounded-lg p-4 flex flex-col sm:flex-row items-start sm:items-center justify-between gap-3">
        <div className="flex items-center gap-2.5">
          <Shield className="w-4 h-4 text-[#b6d99c]" />
          <div>
            <span className="text-xs font-semibold text-zinc-200 block">
              {isAr ? 'تحديثات التطبيق' : 'Application Updates'}
            </span>
            <span className="text-[11px] text-zinc-500 font-mono">
              ARGON v{CURRENT_VERSION}
            </span>
          </div>
        </div>

        <button
          onClick={handleCheckUpdates}
          disabled={isCheckingUpdate}
          className="px-4 py-2 rounded-xl bg-white/[0.06] hover:bg-white/[0.1] text-xs font-semibold text-zinc-200 transition-all flex items-center gap-2 active:scale-95 disabled:opacity-50"
        >
          <RefreshCw className={`w-3.5 h-3.5 ${isCheckingUpdate ? 'animate-spin text-[#b6d99c]' : ''}`} />
          <span>{isCheckingUpdate ? (isAr ? 'جاري الفحص...' : 'Checking...') : (isAr ? 'فحص التحديثات' : 'Check for Updates')}</span>
        </button>
      </div>

      {updateInfo && (
        <div
          className={`p-3 rounded-xl border text-xs font-mono flex items-center gap-2 animate-in-fade ${
            updateInfo.hasUpdate
              ? 'bg-[#b6d99c]/10 border-[#b6d99c]/30 text-[#b6d99c]'
              : 'bg-white/[0.02] border-white/[0.06] text-zinc-400'
          }`}
        >
          {updateInfo.hasUpdate ? (
            <>
              <CheckCircle2 className="w-4 h-4 shrink-0 text-[#b6d99c]" />
              <span>
                {isAr
                  ? `يتوفر تحديث جديد (v${updateInfo.version})!`
                  : `New version available (v${updateInfo.version})!`}
              </span>
            </>
          ) : (
            <>
              <CheckCircle2 className="w-4 h-4 shrink-0 text-[#b6d99c]" />
              <span>
                {isAr ? 'أنت تستخدم أحدث إصدار متوفر ✓' : 'You are running the latest version ✓'}
              </span>
            </>
          )}
        </div>
      )}
      {releaseInfo && updateInfo?.hasUpdate && (
        <button className="px-4 py-2 rounded-xl bg-[#b6d99c] text-zinc-950 text-xs font-bold" onClick={() => {
          const isAndroid = /android/i.test(navigator.userAgent);
          const asset = isAndroid ? releaseInfo.assets.androidApk : releaseInfo.assets.windowsInstaller;
          if (!asset) { setUpdateError(isAr ? 'لا يتوفر مثبت لهذا النظام في الإصدار المنشور.' : 'No installer is published for this platform.'); return; }
          downloadManager.addTask(asset.url, undefined, 4, asset.sha256);
          setUpdateError(isAr ? 'أُضيف التحديث إلى التنزيلات.' : 'Update added to downloads.');
        }}>{isAr ? 'تنزيل التحديث' : 'Download update'}</button>
      )}
      {updateError && <p role="status" className="text-xs text-amber-300 p-3 border border-amber-500/20 rounded-xl">{updateError}</p>}
    </div>
  );
};
