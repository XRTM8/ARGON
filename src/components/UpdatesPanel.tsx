import { useEffect, useState, useSyncExternalStore } from 'react';
import { ArrowDownToLine, RefreshCw, ShieldCheck } from 'lucide-react';
import { updateManager, type UpdateState } from '../core/updates/updateManager';
import { downloadManager } from '../core/downloadManager';
import { useFlow } from '../core/useFlow';
import { formatBytes } from '../core/segmentation';
import { CURRENT_VERSION } from './Header';

function updateError(error: string, ar: boolean) {
  if (/transfers_active/.test(error)) return ar ? 'انتظر اكتمال التنزيلات والمشاركة قبل تثبيت التحديث.' : 'Finish active downloads and sharing before installing.';
  if (/signature|pubkey|public key|verify|verification/i.test(error)) return ar ? 'تعذّر التحقق من توقيع التحديث. لم يُثبّت شيء.' : 'The update signature could not be verified. Nothing was installed.';
  if (/tasks_not_saved|preferences_not_saved/.test(error)) return ar ? 'تعذّر حفظ البيانات محلياً. تحقق من مساحة القرص وأعد المحاولة.' : 'Local data could not be saved. Check available storage and retry.';
  if (/incompatible_update_package|unsupported_package/.test(error)) return ar ? 'لا توجد حزمة تحديث متوافقة مع نوع هذه النسخة. لن يُستخدم مثبت بدل Portable.' : 'No compatible update package is available for this copy. A portable copy will not use an installer.';
  if (/cached_update_changed/.test(error)) return ar ? 'تغير ملف التحديث المحفوظ. اضغط تنزيل التحديث لجلب نسخة موثوقة.' : 'The cached update changed. Download the update again to get a verified copy.';
  if (/update_busy/.test(error)) return ar ? 'توجد عملية تحديث قيد التنفيذ.' : 'An update operation is already running.';
  return ar ? 'تعذّر الوصول إلى تحديث موثوق. تحقق من الاتصال وحاول مجدداً.' : 'Could not reach a verified update. Check your connection and retry.';
}

export function UpdatesPanel({ lang }: { lang: 'ar' | 'en' }) {
  const ar = lang === 'ar';
  const state = useSyncExternalStore(updateManager.subscribe, updateManager.getState);
  const { snapshot, pending } = useFlow();
  const [active, setActive] = useState(downloadManager.getStats().activeCount);
  useEffect(() => downloadManager.subscribe(() => setActive(downloadManager.getStats().activeCount)), []);
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
    queued: ar ? 'أُضيفت الحزمة المتوافقة إلى التنزيلات. فكّ Portable في مجلد منفصل بعد إغلاق النسخة القديمة.' : 'The compatible package is in Downloads. Extract Portable into a separate folder after closing the old copy.',
    error: ar ? 'تعذّر فحص التحديثات.' : 'Could not check for updates.'
  };
  return <section className="updates-panel glass-panel" aria-label={ar ? 'تحديثات التطبيق' : 'Application updates'}>
    <div className="update-heading"><ShieldCheck size={19} /><div><h2>{ar ? 'تحديثات التطبيق' : 'Application updates'}</h2><span dir="ltr">ARGON {CURRENT_VERSION}{state.mode === 'portable' ? ' · Portable' : installed ? ' · Windows installer' : ''}</span></div><button className="action-button" disabled={busy || state.phase === 'ready' || ['unknown', 'preview'].includes(state.mode)} onClick={() => void updateManager.check()}><RefreshCw size={14} className={state.phase === 'checking' ? 'animate-spin' : ''} />{ar ? 'فحص الآن' : 'Check now'}</button></div>
    <label className="update-preference"><div><strong>{ar ? 'فحص وتنزيل التحديثات تلقائياً' : 'Check and download updates automatically'}</strong><p>{installed ? (ar ? 'اختياري ومطفأ افتراضياً. يفحص عند التشغيل وكل 6 ساعات؛ التثبيت بموافقتك بعد انتهاء العمليات.' : 'Optional and off by default. Checks at startup and every 6 hours; you approve installation after transfers finish.') : state.mode === 'portable' ? (ar ? 'نسخة Portable تُحدّث بحزمة ZIP متوافقة حتى تبقى مستقلة عن المثبّت.' : 'Portable copies use a compatible ZIP to stay separate from the installer.') : (ar ? 'التحديث داخل التطبيق متاح حالياً لمثبّت Windows x64.' : 'In-app installation is currently available for the Windows x64 installer.')}</p></div><input type="checkbox" role="switch" checked={state.automatic} disabled={!installed || state.phase === 'installing'} onChange={event => void updateManager.setAutomatic(event.target.checked)} aria-label={ar ? 'تحديث تلقائي اختياري' : 'Optional automatic updates'} /></label>
    <p className="update-description" role="status">{state.mode === 'preview' ? (ar ? 'هذه معاينة للواجهة؛ التحديث يتطلب تطبيق سطح المكتب.' : 'Interface preview. Updates require the desktop app.') : descriptions[state.phase]}</p>
    {state.phase === 'downloading' && <div className="update-progress"><progress value={state.total ? state.downloaded : undefined} max={state.total || undefined} aria-label={ar ? 'تقدم تنزيل التحديث' : 'Update download progress'} /><span dir="ltr">{formatBytes(state.downloaded)}{state.total ? ` / ${formatBytes(state.total)}` : ''}</span></div>}
    {state.phase === 'available' && <button className="action-button primary" onClick={() => void updateManager.download()}><ArrowDownToLine size={15} />{installed ? (ar ? 'تنزيل التحديث الموقّع' : 'Download signed update') : (ar ? 'تنزيل الحزمة المتوافقة' : 'Download compatible package')}</button>}
    {state.phase === 'ready' && <><button className="action-button primary" disabled={active > 0 || sharing} onClick={() => void updateManager.install()}>{ar ? 'تثبيت وإعادة فتح ARGON' : 'Install and restart ARGON'}</button>{(active > 0 || sharing) && <p className="update-description">{ar ? 'التثبيت ينتظر انتهاء التنزيلات والمشاركة.' : 'Installation waits for downloads and sharing to finish.'}</p>}</>}
    {state.lastChecked && <small className="update-last-check">{ar ? 'آخر فحص: ' : 'Last checked: '}{new Date(state.lastChecked).toLocaleString(ar ? 'ar-IQ' : 'en')}</small>}
    {state.error && <p className="inline-error" role="alert">{updateError(state.error, ar)}</p>}
  </section>;
}
