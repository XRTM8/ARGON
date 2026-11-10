import { useEffect, useState, useSyncExternalStore } from 'react';
import { ArrowDownToLine, FolderOpen, Play, RefreshCw, RotateCcw, ShieldCheck } from 'lucide-react';
import { updateManager, type UpdateState } from '../core/updates/updateManager';
import { downloadManager } from '../core/downloadManager';
import { useFlow } from '../core/useFlow';
import { formatBytes, formatSpeed } from '../core/segmentation';
import { isAndroidRuntime } from '../core/safeArea';
import { CURRENT_VERSION } from './Header';

function updateError(error: string, ar: boolean, android: boolean) {
  if (/transfers_active/.test(error)) return ar ? 'انتظر اكتمال التنزيلات والمشاركة قبل تثبيت التحديث.' : 'Finish active downloads and sharing before installing.';
  if (/signature|pubkey|public key|verify|verification/i.test(error)) return ar ? 'تعذّر التحقق من توقيع التحديث. لم يُثبّت شيء.' : 'The update signature could not be verified. Nothing was installed.';
  if (/tasks_not_saved|preferences_not_saved/.test(error)) return ar ? 'تعذّر حفظ البيانات محلياً. تحقق من مساحة القرص وأعد المحاولة.' : 'Local data could not be saved. Check available storage and retry.';
  if (/android_apk_unavailable/.test(error)) return ar ? 'تم نشر إصدار أحدث على GitHub لكنه لا يتضمن حزمة Android (APK) بعد.' : 'A newer release exists on GitHub, but it does not include an Android APK package yet.';
  if (/android_install_permission_required/.test(error)) return ar ? 'يرجى تفعيل إذن «تثبيت التطبيقات غير المعروفة» لـ ARGON من شاشة الإعدادات، ثم اضغط تثبيت التحديث مجدداً.' : 'Allow unknown app installs for ARGON in the settings screen that opened, then tap Install update again.';
  if (/incompatible_update_package|unsupported_package/.test(error)) {
    return android
      ? ar ? 'تتوفر حزمة تثبيت غير متوافقة مع جهاز Android هذا.' : 'The published release package is incompatible with this Android device.'
      : ar ? 'لا توجد حزمة تحديث متوافقة مع نوع هذه النسخة. لن يُستخدم مثبت بدل Portable.' : 'No compatible update package is available for this copy. A portable copy will not use an installer.';
  }
  if (/cached_update_changed/.test(error)) return ar ? 'تغير ملف التحديث المحفوظ. اضغط تنزيل التحديث لجلب نسخة موثوقة.' : 'The cached update changed. Download the update again to get a verified copy.';
  if (/download_update_first/.test(error)) return ar ? 'قم بتنزيل التحديث أولاً قبل بدء التثبيت.' : 'Download the update first before installing.';
  if (/update_check_timeout|timed out|timeout/i.test(error)) return ar ? 'انتهت مهلة الاتصال بخادم التحديثات. تحقق من الشبكة وحاول مجدداً.' : 'Update check timed out. Check your connection and retry.';
  if (/update_busy/.test(error)) return ar ? 'توجد عملية تحديث قيد التنفيذ.' : 'An update operation is already running.';
  const detail = error.replace(/^Error:\s*/i, '').trim();
  const base = ar ? 'تعذّر إتمام التحديث:' : 'Update failed:';
  return detail ? `${base} ${detail.slice(0, 140)}` : (ar ? 'تعذّر الوصول إلى تحديث موثوق. تحقق من الاتصال وحاول مجدداً.' : 'Could not reach a verified update. Check your connection and retry.');
}

export function UpdatesPanel({ lang }: { lang: 'ar' | 'en' }) {
  const ar = lang === 'ar';
  const android = isAndroidRuntime();
  const state = useSyncExternalStore(updateManager.subscribe, updateManager.getState);
  const { snapshot, pending } = useFlow();
  const [tasks, setTasks] = useState(() => downloadManager.getTasks());
  const [active, setActive] = useState(downloadManager.getStats().activeCount);
  const [localError, setLocalError] = useState<string | null>(null);
  useEffect(() => downloadManager.subscribe(() => {
    setTasks([...downloadManager.getTasks()]);
    setActive(downloadManager.getStats().activeCount);
  }), []);
  const queuedTask = state.taskId ? tasks.find(t => t.id === state.taskId) : undefined;
  const sharing = !!snapshot?.transfers.some(transfer => ['queued', 'hashing', 'waiting', 'sending', 'receiving', 'verifying'].includes(transfer.status)) || !!snapshot?.incoming.length || !!snapshot?.pairings.length || pending.length > 0;
  const busy = ['checking', 'downloading', 'installing'].includes(state.phase);
  const installed = state.mode === 'installed';
  const descriptions: Record<UpdateState['phase'], string> = {
    idle: ar ? 'افحص التحديثات متى تريد، أو فعّل الفحص والتنزيل التلقائي.' : 'Check when you want, or enable automatic checking and downloading.',
    checking: ar ? 'جارٍ فحص الإصدار والحزمة المتوافقة…' : 'Checking the version and compatible package…',
    available: ar ? `يتوفر الإصدار ${state.version}.` : `Version ${state.version} is available.`,
    downloading: ar ? 'جارٍ تنزيل التحديث والتحقق من توقيعه…' : 'Downloading the update and verifying its signature…',
    ready: ar ? `الإصدار ${state.version} جاهز. التثبيت يعيد فتح التطبيق.` : `Version ${state.version} is ready. Installation restarts ARGON.`,
    current: ar ? 'هذه أحدث نسخة مستقرة منشورة.' : 'You have the latest published stable version.',
    installing: ar ? 'جارٍ تشغيل مثبت التحديث…' : 'Starting the update installer…',
    queued: queuedTask?.status === 'completed'
      ? android
        ? ar ? `اكتمل تنزيل تحديث ${state.version}. اضغط تثبيت التحديث للمتابعة.` : `ARGON ${state.version} APK is downloaded. Tap Install update to continue.`
        : ar ? `اكتمل تنزيل حزمة ${state.version}. افتحها لاستخراج النسخة في مجلد جديد بعد إغلاق التطبيق.` : `ARGON ${state.version} Portable ZIP is downloaded. Extract it after closing this copy.`
      : android
        ? ar ? 'أُضيف ملف التحديث (APK) إلى التنزيلات مع التحقق من بصمة SHA-256.' : 'The Android APK update was added to Downloads with SHA-256 verification.'
        : ar ? 'أُضيفت الحزمة المتوافقة إلى التنزيلات. فكّ Portable في مجلد منفصل بعد إغلاق النسخة القديمة.' : 'The compatible package is in Downloads. Extract Portable into a separate folder after closing the old copy.',
    error: ar ? 'تعذّر فحص أو تثبيت التحديث.' : 'Could not check or install updates.'
  };
  const openQueuedPackage = async (revealFolder = false) => {
    if (!queuedTask?.savePath) return;
    setLocalError(null);
    try {
      if (revealFolder) await downloadManager.openFolder(queuedTask.savePath);
      else await downloadManager.openFile(queuedTask.savePath);
    } catch (error) {
      setLocalError(String(error));
    }
  };
  const displayError = localError || state.error;
  return <section className="updates-panel glass-panel" aria-label={ar ? 'تحديثات التطبيق' : 'Application updates'}>
    <div className="update-heading"><ShieldCheck size={19} /><div><h2>{ar ? 'تحديثات التطبيق' : 'Application updates'}</h2><span dir="ltr">ARGON {CURRENT_VERSION}{state.mode === 'portable' ? ' · Portable' : installed ? ' · Windows installer' : android ? ' · Android' : ''}</span></div><button className="action-button" disabled={busy || state.phase === 'ready' || ['unknown', 'preview'].includes(state.mode)} onClick={() => { setLocalError(null); void updateManager.check(); }}><RefreshCw size={14} className={state.phase === 'checking' ? 'animate-spin' : ''} />{ar ? 'فحص الآن' : 'Check now'}</button></div>
    <label className="update-preference"><div><strong>{ar ? 'فحص وتنزيل التحديثات تلقائياً' : 'Check and download updates automatically'}</strong><p>{installed ? (ar ? 'اختياري ومطفأ افتراضياً. يفحص عند التشغيل وكل 6 ساعات؛ التثبيت بموافقتك بعد انتهاء العمليات.' : 'Optional and off by default. Checks at startup and every 6 hours; you approve installation after transfers finish.') : state.mode === 'portable' ? (ar ? 'نسخة Portable تُحدّث بحزمة ZIP متوافقة حتى تبقى مستقلة عن المثبّت.' : 'Portable copies use a compatible ZIP to stay separate from the installer.') : android ? (ar ? 'على Android يتم تنزيل حزمة APK والتحقق من SHA-256 قبل تشغيل المثبّت.' : 'On Android, compatible APK releases are downloaded and SHA-256 verified before launching Android package installer.') : (ar ? 'التحديث التلقائي متاح حالياً لمثبّت Windows x64.' : 'Automatic background installation is currently available for the Windows x64 installer.')}</p></div><input type="checkbox" role="switch" checked={state.automatic} disabled={!installed || state.phase === 'installing'} onChange={event => void updateManager.setAutomatic(event.target.checked)} aria-label={ar ? 'تحديث تلقائي اختياري' : 'Optional automatic updates'} /></label>
    <p className="update-description" role="status">{state.mode === 'preview' ? (ar ? 'هذه معاينة للواجهة؛ التحديث يتطلب تطبيق سطح المكتب.' : 'Interface preview. Updates require the desktop app.') : descriptions[state.phase]}</p>
    {state.phase === 'downloading' && <div className="update-progress"><progress value={state.total ? state.downloaded : undefined} max={state.total || undefined} aria-label={ar ? 'تقدم تنزيل التحديث' : 'Update download progress'} /><span dir="ltr">{formatBytes(state.downloaded)}{state.total ? ` / ${formatBytes(state.total)}` : ''}</span></div>}
    {state.phase === 'queued' && queuedTask && queuedTask.status !== 'completed' && (
      <div className="update-progress">
        <progress value={queuedTask.totalBytes > 0 ? queuedTask.downloadedBytes : undefined} max={queuedTask.totalBytes > 0 ? queuedTask.totalBytes : undefined} aria-label={ar ? 'تقدم تنزيل حزمة التحديث' : 'Update package download progress'} />
        <span dir="ltr">
          {formatBytes(queuedTask.downloadedBytes)}
          {queuedTask.totalBytes > 0 ? ` / ${formatBytes(queuedTask.totalBytes)}` : ''}
          {queuedTask.speedBytesPerSec > 0 ? ` · ${formatSpeed(queuedTask.speedBytesPerSec)}` : ''}
        </span>
      </div>
    )}
    {state.phase === 'available' && <button className="action-button primary" onClick={() => { setLocalError(null); void updateManager.download(); }}><ArrowDownToLine size={15} />{installed ? (ar ? 'تنزيل التحديث الموقّع' : 'Download signed update') : (ar ? 'تنزيل التحديث' : 'Download update')}</button>}
    {state.phase === 'queued' && queuedTask?.status === 'paused' && (
      <button className="action-button primary" onClick={() => downloadManager.resumeTask(queuedTask.id)}>
        <Play size={15} />
        {ar ? 'استئناف تنزيل التحديث' : 'Resume update download'}
      </button>
    )}
    {state.phase === 'queued' && queuedTask?.status === 'error' && (
      <button className="action-button primary" onClick={() => downloadManager.resumeTask(queuedTask.id)}>
        <RotateCcw size={15} />
        {ar ? 'إعادة محاولة التنزيل' : 'Retry update download'}
      </button>
    )}
    {state.phase === 'queued' && queuedTask?.status === 'completed' && (
      <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
        <button className="action-button primary" onClick={() => void openQueuedPackage(false)}>
          <ArrowDownToLine size={15} />
          {android ? (ar ? 'تثبيت التحديث (APK)' : 'Install update (APK)') : (ar ? 'فتح حزمة التحديث' : 'Open update package')}
        </button>
        {!android && (
          <button className="action-button" onClick={() => void openQueuedPackage(true)}>
            <FolderOpen size={15} />
            {ar ? 'فتح المجلد' : 'Open folder'}
          </button>
        )}
      </div>
    )}
    {state.phase === 'ready' && <><button className="action-button primary" disabled={active > 0 || sharing} onClick={() => void updateManager.install()}>{ar ? 'تثبيت وإعادة فتح ARGON' : 'Install and restart ARGON'}</button>{(active > 0 || sharing) && <p className="update-description">{ar ? 'التثبيت ينتظر انتهاء التنزيلات والمشاركة.' : 'Installation waits for downloads and sharing to finish.'}</p>}</>}
    {state.lastChecked && <small className="update-last-check">{ar ? 'آخر فحص: ' : 'Last checked: '}{new Date(state.lastChecked).toLocaleString(ar ? 'ar-IQ' : 'en')}</small>}
    {displayError && <p className="inline-error" role="alert">{updateError(displayError, ar, android)}</p>}
  </section>;
}
