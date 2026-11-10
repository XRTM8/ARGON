import { useState } from 'react';
import {
  Send,
  Smartphone,
  Monitor,
  Power,
  Plus,
  Wifi,
  ShieldCheck,
  ShieldQuestion,
  Link2,
  X,
  FolderOpen,
  FileText,
  Clipboard,
  ClipboardCheck,
  Copy,
  ArrowDownToLine,
  ArrowUpRight,
  ArrowDownLeft,
  Loader2,
  Check,
  AlertCircle,
  Cable,
  Unplug,
  HardDrive
} from 'lucide-react';
import { useFlow } from '../core/useFlow';
import { nativeRuntime } from '../core/flowClient';
import { downloadManager } from '../core/downloadManager';
import { formatBytes, formatSpeed } from '../core/segmentation';
import type { FlowTransfer } from '../types/flow';
import type { MainViewTab } from './Header';

type Lang = 'ar' | 'en';
const run = (promise: Promise<unknown>) => {
  void promise.catch(() => {});
};
const readablePath = (path: string) => path.replace(/^\\\\\?\\/, '');
const android = () => /android/i.test(globalThis.navigator?.userAgent || '');

export function flowStatus(status: string, lang: Lang) {
  const names: Record<string, [string, string]> = {
    queued: ['في الانتظار', 'Queued'],
    hashing: ['حساب البصمة', 'Hashing'],
    waiting: ['بانتظار موافقة الطرف الآخر', 'Waiting for peer'],
    sending: ['جارٍ الإرسال', 'Sending'],
    receiving: ['جارٍ الاستقبال', 'Receiving'],
    verifying: ['التحقق من البصمة', 'Verifying integrity'],
    completed: ['مكتمل', 'Complete'],
    error: ['فشل النقل', 'Failed'],
    cancelled: ['ملغى', 'Cancelled']
  };
  return names[status]?.[lang === 'ar' ? 0 : 1] || status;
}

export function FlowTransferList({ lang, limit = 8, all = false }: { lang: Lang; limit?: number; all?: boolean }) {
  const { snapshot, client } = useFlow();
  const ar = lang === 'ar';
  const transfers = (snapshot?.transfers || []).slice(0, all ? 100 : limit);

  return (
    <div className="flow-transfers">
      {transfers.length === 0 ? (
        <div className="quiet-empty">
          <FileText size={20} />
          <p>{ar ? 'لا توجد عمليات نقل حالية.' : 'No active transfers.'}</p>
        </div>
      ) : (
        transfers.map((transfer) => (
          <TransferRow
            key={transfer.id}
            transfer={transfer}
            lang={lang}
            onCancel={() => run(client.execute('flow_cancel_transfer', { transferId: transfer.id }))}
          />
        ))
      )}
    </div>
  );
}

function TransferRow({
  transfer: t,
  lang,
  onCancel
}: {
  transfer: FlowTransfer;
  lang: Lang;
  onCancel: () => void;
}) {
  const { client } = useFlow();
  const ar = lang === 'ar';
  const active = !['completed', 'error', 'cancelled'].includes(t.status);
  const percent = t.total > 0 ? Math.min(100, (t.bytes / t.total) * 100) : 0;

  return (
    <article className={`flow-transfer ${t.status === 'error' ? 'has-error' : ''}`}>
      <div className="file-symbol">
        {t.kind === 'text' ? (
          <Clipboard size={16} />
        ) : t.direction === 'send' ? (
          <ArrowUpRight size={16} />
        ) : (
          <ArrowDownLeft size={16} />
        )}
      </div>

      <div className="transfer-detail">
        <div className="row-between">
          <strong dir={t.kind === 'file' ? 'auto' : undefined}>
            {t.kind === 'text' ? (ar ? 'نص الحافظة' : 'Clipboard text') : t.name}
          </strong>
          <span className={`transfer-state state-${t.status}`}>{flowStatus(t.status, lang)}</span>
        </div>

        <div className="transfer-meta">
          <span>
            {t.direction === 'send' ? (ar ? 'إلى' : 'To') : ar ? 'من' : 'From'} {t.peerName}
          </span>
          <span dir="ltr">
            {formatBytes(t.bytes)} / {formatBytes(t.total)}
          </span>
          {t.speed > 0 && <span dir="ltr">{formatSpeed(t.speed)}</span>}
        </div>

        {active && t.kind === 'file' && (
          <div
            className="titanium-progress"
            role="progressbar"
            aria-label={ar ? 'تقدم النقل' : 'Transfer progress'}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={Math.round(percent)}
          >
            <div style={{ width: `${percent}%` }} />
          </div>
        )}

        {t.error && (
          <p className="transfer-error" dir="auto">
            {t.error}
          </p>
        )}
      </div>

      <div className="transfer-actions">
        {active ? (
          <button className="icon-button" onClick={onCancel} aria-label={ar ? 'إلغاء النقل' : 'Cancel transfer'}>
            <X size={15} />
          </button>
        ) : t.status === 'completed' && t.path ? (
          <button
            className="icon-button"
            onClick={() =>
              run(
                t.direction === 'receive'
                  ? client.execute('flow_open_received', { transferId: t.id })
                  : android()
                    ? downloadManager.openFile(t.path!)
                    : downloadManager.openFolder(t.path!)
              )
            }
            aria-label={android() ? (ar ? 'فتح الملف' : 'Open file') : ar ? 'فتح مجلد الملف' : 'Open file folder'}
          >
            <FolderOpen size={15} />
          </button>
        ) : null}

        {t.status === 'completed' && t.sha256 && (
          <ShieldCheck size={16} className="status-ok" aria-label="SHA-256 verified" />
        )}
      </div>
    </article>
  );
}

export function SharingView({ lang, onSwitchTab }: { lang: Lang; onSwitchTab?: (tab: MainViewTab) => void }) {
  const { snapshot, selectedFiles, selectedPeerId, pending, client } = useFlow();
  const ar = lang === 'ar';
  const [address, setAddress] = useState('');
  const [copied, setCopied] = useState(false);

  const peer = snapshot?.peers.find((p) => p.id === selectedPeerId);
  const enabled = !!snapshot?.enabled;

  const toggle = () => run(client.execute(enabled ? 'flow_stop' : 'flow_start'));
  const connect = () => {
    const value = address.trim();
    if (!value) return;
    run(client.execute('flow_connect', { address: value.includes(':') ? value : `${value}:54127` }));
  };

  return (
    <div className="sharing-view">
      {onSwitchTab && (
        <div className="mobile-flow-switch">
          <button type="button" className="is-active" onClick={() => onSwitchTab('sharing')}>
            {ar ? 'مشاركة الملفات' : 'Local Sharing'}
          </button>
          <button type="button" onClick={() => onSwitchTab('clipboard')}>
            {ar ? 'الحافظة المشفرة' : 'Clipboard'}
          </button>
        </div>
      )}

      <div className="view-header">
        <div className="view-header-title">
          <h1>{ar ? 'المشاركة المحلية المشفرة' : 'Encrypted Local Sharing'}</h1>
          <span className="count-badge" dir="ltr">
            {snapshot?.peers.length || 0} {ar ? 'أجهزة' : 'devices'}
          </span>
        </div>

        <button
          className={`action-button ${enabled ? '' : 'primary'}`}
          disabled={!nativeRuntime() || pending.includes('flow_start') || pending.includes('flow_stop')}
          onClick={toggle}
        >
          <Power size={15} />
          <span>{enabled ? (ar ? 'إيقاف الخدمة' : 'Stop Service') : ar ? 'تشغيل المشاركة' : 'Enable Sharing'}</span>
        </button>
      </div>

      {!nativeRuntime() && (
        <div className="runtime-notice" role="status">
          <AlertCircle size={16} />
          <span>
            {ar
              ? 'معاينة للواجهة: بروتوكول Flow يتطلب تشغيل تطبيق ARGON المكتبي أو المحمول.'
              : 'Preview mode: Flow protocol operates natively inside ARGON desktop and mobile builds.'}
          </span>
        </div>
      )}

      {enabled && snapshot && (
        <div className="connection-strip glass-panel">
          <div>
            <Wifi size={16} className="accent-text" />
            <span style={{ fontWeight: 600 }}>{snapshot.name}</span>
            <span style={{ color: 'var(--argon-subtle)' }}>·</span>
            <code dir="ltr">{snapshot.addresses[0] || '127.0.0.1:54127'}</code>
          </div>

          <button
            className="text-button"
            onClick={() => {
              void navigator.clipboard
                .writeText(snapshot.addresses[0] || '')
                .then(() => setCopied(true))
                .catch(() => setCopied(false));
              setTimeout(() => setCopied(false), 1600);
            }}
          >
            {copied ? <Check size={13} className="status-ok" /> : <Copy size={13} />}
            <span>{copied ? (ar ? 'تم النسخ' : 'Copied') : ar ? 'نسخ العنوان' : 'Copy Address'}</span>
          </button>
        </div>
      )}

      <div className="sharing-grid">
        <section className="sharing-workspace">
          <div className="section-title">
            <h2>{ar ? 'الأجهزة المتصلة' : 'Local Network Devices'}</h2>
            <span>{enabled ? (ar ? 'شبكة Wi-Fi المحلية' : 'LAN / Hotspot') : ar ? 'الخدمة غير مفعّلة' : 'Offline'}</span>
          </div>

          {snapshot?.peers.length ? (
            <div className="peer-grid">
              {snapshot.peers.map((device) => (
                <article key={device.id} className={`peer-card ${device.id === selectedPeerId ? 'is-selected' : ''}`}>
                  <button
                    className="peer-select"
                    aria-pressed={device.id === selectedPeerId}
                    onClick={() => client.selectPeer(device.id)}
                  >
                    <span className="peer-icon">
                      {device.name.toLowerCase().includes('phone') || device.name.includes('هاتف') ? (
                        <Smartphone size={20} />
                      ) : (
                        <Monitor size={20} />
                      )}
                    </span>
                    <strong dir="auto">{device.name}</strong>
                    <span className="peer-status">
                      {device.trusted ? (
                        <ShieldCheck size={13} className="status-ok" />
                      ) : (
                        <ShieldQuestion size={13} className="status-warn" />
                      )}
                      <span>
                        {device.trusted
                          ? ar ? 'مقترن وموثوق' : 'Paired & Verified'
                          : ar ? 'غير مقترن' : 'Not paired'}
                      </span>
                      <i className={`presence ${device.online ? 'online' : ''}`} />
                    </span>
                    <code dir="ltr">{device.address}</code>
                  </button>

                  <button
                    className="peer-pair text-button"
                    disabled={!enabled || pending.includes('flow_pair_peer')}
                    onClick={() =>
                      run(client.execute(device.trusted ? 'flow_revoke_peer' : 'flow_pair_peer', { peerId: device.id }))
                    }
                  >
                    {device.trusted ? <Unplug size={13} /> : <Link2 size={13} />}
                    <span>{device.trusted ? (ar ? 'إلغاء الاقتران' : 'Unpair') : ar ? 'بدء الاقتران' : 'Pair Device'}</span>
                  </button>
                </article>
              ))}
            </div>
          ) : (
            <div className="device-empty glass-panel">
              <Cable size={24} />
              <strong>{enabled ? (ar ? 'لم يتم العثور على أجهزة بعد' : 'Searching for local devices...') : ar ? 'المشاركة المحلية متوقفة' : 'Local Sharing is Off'}</strong>
              <p>
                {enabled
                  ? ar
                    ? 'تأكد من فتح تطبيق ARGON على الجهاز الآخر والاتصال بنفس الشبكة.'
                    : 'Ensure ARGON is open on the target device and connected to the same LAN.'
                  : ar
                    ? 'اضغط "تشغيل المشاركة" أعلاه لبدء استقبال وإرسال الملفات بأمان.'
                    : 'Click "Enable Sharing" above to securely send and receive files.'}
              </p>
            </div>
          )}

          <div className="manual-connect">
            <label htmlFor="flow-address">{ar ? 'اتصال مباشر بعنوان IP' : 'Direct IP Connect'}</label>
            <div>
              <input
                id="flow-address"
                dir="ltr"
                value={address}
                onChange={(e) => setAddress(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter') connect();
                }}
                placeholder="192.168.1.5:54127"
                autoComplete="off"
              />
              <button
                className="action-button"
                disabled={!enabled || !address.trim() || pending.includes('flow_connect')}
                onClick={connect}
              >
                <Link2 size={14} />
                <span>{ar ? 'اتصال' : 'Connect'}</span>
              </button>
            </div>
          </div>

          <div className="send-drop glass-panel">
            <div>
              <h2>{peer ? (ar ? `إرسال إلى: ${peer.name}` : `Target: ${peer.name}`) : ar ? 'إرسال ملفات' : 'Send Files'}</h2>
              <p>
                {peer?.trusted
                  ? ar
                    ? 'اختر ملفات للإرسال المشفر عبر بروتوكول Noise XX.'
                    : 'Select files for end-to-end encrypted transfer.'
                  : ar
                    ? 'اختر جهازاً مقترناً أولاً للبدء بالإرسال.'
                    : 'Pair with a device above to begin sending.'}
              </p>
            </div>
            <button
              className="action-button primary"
              disabled={!nativeRuntime() || pending.includes('flow_choose_files')}
              onClick={() => run(client.chooseFiles())}
            >
              <Plus size={15} />
              <span>{ar ? 'اختيار ملفات' : 'Select Files'}</span>
            </button>
          </div>

          {selectedFiles.length > 0 && (
            <div className="glass-panel" style={{ marginTop: 14, padding: 16 }}>
              <div className="section-title">
                <h2>{ar ? 'الملفات المحددة' : 'Staged Files'}</h2>
                <span>
                  {selectedFiles.length} · {formatBytes(selectedFiles.reduce((sum, f) => sum + f.size, 0))}
                </span>
              </div>

              <div style={{ display: 'grid', gap: 6, margin: '10px 0' }}>
                {selectedFiles.map((file) => (
                  <div
                    key={file.id}
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      gap: 10,
                      padding: '8px 10px',
                      background: 'var(--argon-surface)',
                      borderRadius: 7
                    }}
                  >
                    <FileText size={15} className="accent-text" />
                    <span dir="auto" style={{ flex: 1, fontSize: 12.5 }}>
                      {file.name}
                    </span>
                    <small dir="ltr" style={{ color: 'var(--argon-subtle)', fontFamily: 'monospace' }}>
                      {formatBytes(file.size)}
                    </small>
                    <button className="icon-button" aria-label={ar ? 'إزالة' : 'Remove'} onClick={() => client.removeSelected(file.id)}>
                      <X size={13} />
                    </button>
                  </div>
                ))}
              </div>

              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', paddingTop: 10, borderTop: '1px solid var(--argon-border)' }}>
                <span style={{ fontSize: 11.5, color: 'var(--argon-subtle)' }}>
                  {peer?.trusted
                    ? ar ? 'سيتلقى الطرف الآخر طلب قبول قبل الحفظ.' : 'Receiver must confirm prompt before transfer.'
                    : ar ? 'يلزم اقتران الجهاز أولاً.' : 'Pair device before sending.'}
                </span>

                <button
                  className="action-button primary"
                  disabled={!enabled || !peer?.trusted || pending.includes('flow_send_files')}
                  onClick={() => run(client.sendFiles())}
                >
                  <Send size={14} />
                  <span>{ar ? 'إرسال الآن' : 'Dispatch Files'}</span>
                </button>
              </div>
            </div>
          )}

          <div className="section-title" style={{ marginTop: 22 }}>
            <h2>{ar ? 'عمليات النقل النشطة' : 'Live Transfers'}</h2>
            <span>Flow Protocol v2</span>
          </div>
          <div className="glass-panel">
            <FlowTransferList lang={lang} />
          </div>
        </section>

        <aside className="sharing-aside glass-panel">
          <ShieldCheck size={20} className="accent-text" />
          <h2>{ar ? 'حماية مشفرة نقطة لنقطة' : 'Zero-Knowledge Security'}</h2>
          <p>
            {ar
              ? 'تتم جميع الاتصالات محلياً عبر بروتوكول Noise XX دون المرور عبر أي خوادم أو طرف ثالث.'
              : 'All communication is direct peer-to-peer over encrypted Noise XX sockets without intermediary servers.'}
          </p>
          <div className="aside-rule" />
          <div className="sharing-feature-list">
            <span>
              <Check size={13} className="status-ok" />
              <span>{ar ? 'تشفير ChaChaPoly مع توثيق BLAKE2s' : 'ChaCha20-Poly1305 + BLAKE2s'}</span>
            </span>
            <span>
              <Check size={13} className="status-ok" />
              <span>{ar ? 'فحص تلقائي لبصمة SHA-256 للملفات' : 'Mandatory SHA-256 integrity'}</span>
            </span>
            <span>
              <Check size={13} className="status-ok" />
              <span>{ar ? 'رمز اقتران بصري 6 أرقام' : '6-digit visual handshake'}</span>
            </span>
            <span>
              <Check size={13} className="status-ok" />
              <span>{ar ? 'استئناف تلقائي للملفات بعد الانقطاع' : 'Resume on reconnection'}</span>
            </span>
          </div>
        </aside>
      </div>
    </div>
  );
}

export function ClipboardView({
  lang,
  onDownload,
  onSwitchTab
}: {
  lang: Lang;
  onDownload: (url: string) => void;
  onSwitchTab?: (tab: MainViewTab) => void;
}) {
  const { snapshot, selectedPeerId, pending, client } = useFlow();
  const ar = lang === 'ar';
  const [text, setText] = useState('');
  const [clean, setClean] = useState<string | null>(null);
  const [copied, setCopied] = useState('');

  const peer = snapshot?.peers.find((p) => p.id === selectedPeerId);
  const size = new TextEncoder().encode(text).length;

  const read = async () => {
    const value = await client.execute<string>('flow_read_clipboard');
    setText(value);
    setClean(null);
  };

  const preview = async () => {
    const [value] = await client.execute<[string, boolean]>('flow_clean_text', { text });
    setClean(value);
  };

  const extractUrl = (value: string) => {
    try {
      const parsed = new URL(value.trim());
      return ['http:', 'https:'].includes(parsed.protocol) ? parsed.toString() : null;
    } catch {
      return null;
    }
  };

  return (
    <div className="clipboard-view">
      {onSwitchTab && (
        <div className="mobile-flow-switch">
          <button type="button" onClick={() => onSwitchTab('sharing')}>
            {ar ? 'مشاركة الملفات' : 'Local Sharing'}
          </button>
          <button type="button" className="is-active" onClick={() => onSwitchTab('clipboard')}>
            {ar ? 'الحافظة المشفرة' : 'Clipboard'}
          </button>
        </div>
      )}

      <div className="view-header">
        <div className="view-header-title">
          <h1>{ar ? 'الحافظة المشفرة' : 'Secure Clipboard'}</h1>
          <span className="count-badge" dir="ltr">
            {formatBytes(size)} / 16 KB
          </span>
        </div>

        <button
          className="action-button"
          disabled={!nativeRuntime() || pending.includes('flow_read_clipboard')}
          onClick={() => run(read())}
        >
          <Clipboard size={14} />
          <span>{ar ? 'قراءة الحافظة' : 'Read Clipboard'}</span>
        </button>
      </div>

      <div className="clipboard-compose glass-panel">
        <div className="section-title">
          <label htmlFor="flow-clipboard-text">{ar ? 'محتوى النص للمشاركة' : 'Text to Transfer'}</label>
          <span>{peer ? `${ar ? 'الوجهة:' : 'Destination:'} ${peer.name}` : ''}</span>
        </div>

        <textarea
          id="flow-clipboard-text"
          value={text}
          onChange={(e) => {
            setText(e.target.value);
            setClean(null);
          }}
          placeholder={ar ? 'الصق نصاً أو رابطاً هنا لإرساله إلى جهازك الآخر...' : 'Paste text or links here to dispatch to a paired device...'}
          dir="auto"
        />

        <div className="compose-controls">
          <select aria-label={ar ? 'جهاز استقبال النص' : 'Recipient device'} value={selectedPeerId} onChange={(e) => client.selectPeer(e.target.value)}>
            <option value="">{ar ? 'اختر الجهاز المستقبل...' : 'Select Target Device...'}</option>
            {snapshot?.peers
              .filter((p) => p.trusted)
              .map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
          </select>

          <button
            className="action-button"
            disabled={!text.trim() || size > 16 * 1024 || pending.includes('flow_clean_text')}
            onClick={() => run(preview())}
            title={ar ? 'إزالة معلمات التتبع من الروابط' : 'Strip tracking parameters from URLs'}
          >
            <span>{ar ? 'تنظيف الروابط' : 'Clean URLs'}</span>
          </button>

          <button
            className="action-button primary"
            disabled={!snapshot?.enabled || !peer?.trusted || !text.trim() || size > 16 * 1024 || pending.includes('flow_send_text')}
            onClick={() => run(client.execute('flow_send_text', { peerId: selectedPeerId, text }))}
          >
            <Send size={14} />
            <span>{ar ? 'إرسال النص' : 'Send Text'}</span>
          </button>
        </div>

        {clean !== null && (
          <div className="clean-preview">
            <span style={{ fontSize: 11.5, color: 'var(--argon-accent)', fontWeight: 600 }}>
              {ar ? 'معاينة النص بعد إزالة التتبع:' : 'Cleaned Text Preview:'}
            </span>
            <pre dir="auto">{clean}</pre>
            <button
              className="text-button"
              onClick={() => {
                setText(clean);
                setClean(null);
              }}
            >
              <Check size={13} className="status-ok" />
              <span>{ar ? 'تطبيق النص المنظف' : 'Apply Cleaned Text'}</span>
            </button>
          </div>
        )}

        <p className="privacy-caption">
          {ar
            ? 'محتويات الحافظة مشفرة في الذاكرة المؤقتة ولا يتم حفظها على القرص.'
            : 'Clipboard payloads are transmitted in volatile memory and never retained on disk.'}
        </p>
      </div>

      <div className="section-title" style={{ marginTop: 22 }}>
        <h2>{ar ? 'النصوص المستلمة' : 'Received Snippets'}</h2>
        <span>{snapshot?.clipboard.length || 0}</span>
      </div>

      {snapshot?.clipboard.length ? (
        <div className="received-clips">
          {snapshot.clipboard.map((item) => (
            <article className="received-clip glass-panel" key={item.id}>
              <div className="section-title">
                <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6 }}>
                  <ArrowDownLeft size={13} className="accent-text" /> {item.peerName}
                </span>
                <time style={{ fontFamily: 'monospace', fontSize: 10.5 }}>
                  {new Date(item.receivedAt).toLocaleTimeString(ar ? 'ar-IQ' : 'en', {
                    hour: '2-digit',
                    minute: '2-digit'
                  })}
                </time>
              </div>

              <pre dir="auto">{item.text}</pre>

              <div className="clip-actions">
                <button
                  className="action-button"
                  onClick={() => run(client.execute('flow_copy_received', { itemId: item.id }).then(() => setCopied(item.id)))}
                >
                  {copied === item.id ? <ClipboardCheck size={14} className="status-ok" /> : <Copy size={14} />}
                  <span>{copied === item.id ? (ar ? 'تم النسخ' : 'Copied') : ar ? 'نسخ للحافظة' : 'Copy'}</span>
                </button>

                {extractUrl(item.text) && (
                  <button className="action-button primary" onClick={() => onDownload(extractUrl(item.text)!)}>
                    <ArrowDownToLine size={14} />
                    <span>{ar ? 'تنزيل الرابط' : 'Download Link'}</span>
                  </button>
                )}
              </div>
            </article>
          ))}
        </div>
      ) : (
        <div className="quiet-empty glass-panel">
          <Clipboard size={20} />
          <p>{ar ? 'لا توجد نصوص مستلمة في هذه الجلسة.' : 'No snippets received in this session.'}</p>
        </div>
      )}
    </div>
  );
}

export function FlowDialogs({ lang }: { lang: Lang }) {
  const { snapshot, pending, client } = useFlow();
  const ar = lang === 'ar';
  const pairing = snapshot?.pairings[0];
  const incoming = snapshot?.incoming[0];

  if (!pairing && !incoming) return null;

  const pair = !!pairing;
  const busy = pending.includes(pair ? 'flow_confirm_pairing' : 'flow_respond_incoming');
  const respond = (accepted: boolean) =>
    run(client.execute(pair ? 'flow_confirm_pairing' : 'flow_respond_incoming', { requestId: pairing?.id || incoming!.id, accepted }));

  return (
    <div className="dialog-backdrop">
      <section className="flow-dialog glass-panel" role="dialog" aria-modal="true" aria-labelledby="flow-dialog-title">
        <div className="dialog-symbol">
          {pair ? <ShieldCheck size={24} /> : incoming?.kind === 'text' ? <Clipboard size={24} /> : <ArrowDownLeft size={24} />}
        </div>

        <h2 id="flow-dialog-title">
          {pair ? (ar ? 'تأكيد اقتران الجهاز' : 'Device Pairing Verification') : ar ? 'طلب استقبال وارد' : 'Incoming Transfer Request'}
        </h2>

        <p className="dialog-peer" dir="auto">
          {pairing?.peerName || incoming?.peerName}
        </p>

        {pairing ? (
          <>
            <p className="dialog-caption">
              {ar ? 'قارن الرمز المكوّن من 6 أرقام على كلا الجهازين:' : 'Verify that this 6-digit code matches on both screens:'}
            </p>
            <div className="pairing-code" dir="ltr">
              {pairing.code.slice(0, 3)} <span>{pairing.code.slice(3)}</span>
            </div>
            <p className="fingerprint" dir="ltr">
              {pairing.peerId.match(/.{1,8}/g)?.join(' ')}
            </p>
          </>
        ) : (
          <div style={{ padding: '12px 0' }}>
            <strong dir="auto" style={{ fontSize: 13.5, display: 'block' }}>
              {incoming!.kind === 'text' ? (ar ? 'نص حافظة' : 'Clipboard payload') : incoming!.name}
            </strong>
            <span dir="ltr" style={{ fontFamily: 'monospace', color: 'var(--argon-accent)', fontSize: 12 }}>
              {formatBytes(incoming!.size)}
            </span>
          </div>
        )}

        <div className="dialog-buttons">
          <button className="action-button" disabled={busy} onClick={() => respond(false)}>
            {ar ? 'رفض' : 'Decline'}
          </button>
          <button className="action-button primary" disabled={busy} onClick={() => respond(true)}>
            {busy ? <Loader2 size={15} className="animate-spin" /> : <Check size={15} />}
            <span>{pair ? (ar ? 'الرمز متطابق (موافقة)' : 'Code Matches') : ar ? 'قبول الاستقبال' : 'Accept'}</span>
          </button>
        </div>
      </section>
    </div>
  );
}

export function SharingSettings({ lang }: { lang: Lang }) {
  const { snapshot, pending, client } = useFlow();
  const ar = lang === 'ar';

  return (
    <section className="glass-panel" style={{ padding: 18 }}>
      <div className="section-title">
        <h2>
          <HardDrive size={16} className="accent-text" />
          <span>{ar ? 'مسار استلام ملفات المشاركة' : 'Sharing Storage'}</span>
        </h2>
        <span>Flow v2.0</span>
      </div>

      <label style={{ fontSize: 12, color: 'var(--argon-muted)', display: 'block', marginBottom: 8 }}>
        {ar ? 'مجلد حفظ الملفات المستلمة عبر الشبكة المحلية:' : 'Directory for files received over local network:'}
      </label>

      <div style={{ display: 'flex', gap: 10, alignItems: 'center', flexWrap: 'wrap' }}>
        <code
          dir="ltr"
          style={{
            flex: 1,
            minWidth: 180,
            padding: '8px 11px',
            background: 'var(--argon-input-bg)',
            borderRadius: 7,
            border: '1px solid var(--argon-border)',
            fontSize: 11.5,
            color: 'var(--argon-bright)',
            fontFamily: 'monospace'
          }}
        >
          {readablePath(snapshot?.receiveDirectory || '—')}
        </code>

        {!android() && (
          <button
            className="action-button"
            disabled={!nativeRuntime() || pending.includes('flow_choose_receive_directory')}
            onClick={() => run(client.execute('flow_choose_receive_directory'))}
          >
            <FolderOpen size={14} />
            <span>{ar ? 'تغيير المجلد' : 'Browse'}</span>
          </button>
        )}
      </div>

      <p className="privacy-caption">
        {android()
          ? ar
            ? 'على أندرويد يتم حفظ الملفات المستلمة داخل مساحة التطبيق لضمان الأمان.'
            : 'On Android, incoming transfers are stored within application sandbox for security.'
          : ar
            ? 'تُحفظ الملفات مع التأكد من عدم استبدال أي ملف قائم يحمل الاسم نفسه.'
            : 'Files are safely committed without overwriting pre-existing files.'}
      </p>
    </section>
  );
}
