import { useMemo, useState } from 'react';
import { Cpu, Download, Loader2, X } from 'lucide-react';
import { downloadManager } from '../core/downloadManager';
import { formatBytes } from '../core/segmentation';
import type { HfRepoSummary } from '../types/downloader';

interface Props {
  open: boolean;
  lang: 'ar' | 'en';
  onClose: () => void;
  onQueueFiles: (files: { url: string; filename: string; sha256?: string }[]) => void;
}

export function HuggingFaceModal({ open, lang, onClose, onQueueFiles }: Props) {
  const ar = lang === 'ar';
  const [input, setInput] = useState('');
  const [useMirror, setUseMirror] = useState(false);
  const [token, setToken] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [repo, setRepo] = useState<HfRepoSummary | null>(null);
  const [selectedQuant, setSelectedQuant] = useState<string>('ALL');
  const [selectedPaths, setSelectedPaths] = useState<Set<string>>(new Set());

  const filteredFiles = useMemo(() => {
    if (!repo) return [];
    if (selectedQuant === 'ALL') return repo.files;
    return repo.files.filter((f) => f.isConfig || f.quant === selectedQuant);
  }, [repo, selectedQuant]);

  if (!open) return null;

  const handleFetch = async () => {
    if (!input.trim()) return;
    setLoading(true);
    setError('');
    try {
      const res = await downloadManager.hfFetchRepo(input.trim(), useMirror, token.trim() || undefined);
      setRepo(res);
      const defaultQuant = res.quants.includes('Q4_K_M')
        ? 'Q4_K_M'
        : res.quants[0] ?? 'ALL';
      setSelectedQuant(defaultQuant);
      const preselected = new Set<string>();
      for (const f of res.files) {
        if (f.isConfig || (defaultQuant !== 'ALL' && f.quant === defaultQuant) || res.files.length <= 8) {
          preselected.add(f.path);
        }
      }
      setSelectedPaths(preselected);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  const togglePath = (path: string) => {
    setSelectedPaths((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  const handleDownload = () => {
    if (!repo) return;
    const chosen = repo.files
      .filter((f) => selectedPaths.has(f.path))
      .sort((a, b) => (a.isConfig === b.isConfig ? a.size - b.size : a.isConfig ? -1 : 1));
    if (!chosen.length) return;
    onQueueFiles(
      chosen.map((f) => ({
        url: f.downloadUrl,
        filename: f.path.split('/').pop() || f.path,
        sha256: f.sha256
      }))
    );
    onClose();
  };

  return (
    <div className="dialog-backdrop" onClick={onClose}>
      <div
        className="glass-panel cookie-modal-panel"
        style={{ maxWidth: 640 }}
        role="dialog"
        aria-modal="true"
        aria-labelledby="hf-modal-title"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="cookie-modal-head">
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <Cpu size={18} className="accent-text" />
            <h3 id="hf-modal-title" style={{ margin: 0, fontSize: 16 }}>
              {ar ? 'تنزيل نماذج Hugging Face' : 'Hugging Face Model Downloader'}
            </h3>
          </div>
          <button type="button" className="icon-button" onClick={onClose} aria-label={ar ? 'إغلاق' : 'Close'}>
            <X size={16} />
          </button>
        </div>

        <div style={{ display: 'flex', gap: 8, marginBottom: 10 }}>
          <input
            dir="ltr"
            className="argon-input"
            style={{ flex: 1 }}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            placeholder="Qwen/Qwen3-8B-GGUF or https://huggingface.co/..."
            onKeyDown={(e) => {
              if (e.key === 'Enter') void handleFetch();
            }}
          />
          <button type="button" className="action-button primary" onClick={() => void handleFetch()} disabled={loading}>
            {loading ? <Loader2 size={14} className="spin" /> : ar ? 'جلب الملفات' : 'Fetch'}
          </button>
        </div>

        <div style={{ display: 'flex', gap: 12, alignItems: 'center', fontSize: 12, marginBottom: 10, flexWrap: 'wrap' }}>
          <label style={{ display: 'flex', alignItems: 'center', gap: 6, cursor: 'pointer' }}>
            <input type="checkbox" checked={useMirror} onChange={(e) => setUseMirror(e.target.checked)} />
            <span>{ar ? 'استخدام مرآة hf-mirror.com' : 'Use hf-mirror.com'}</span>
          </label>
          <input
            dir="ltr"
            type="password"
            className="argon-input"
            style={{ flex: 1, minWidth: 180, padding: '5px 8px', fontSize: 12 }}
            value={token}
            onChange={(e) => setToken(e.target.value)}
            placeholder={ar ? 'رمز الوصول للنماذج المقيدة (اختياري)' : 'HF Access Token for gated models (optional)'}
          />
        </div>

        {error && (
          <div className="cookie-modal-status is-err" role="alert">
            {error}
          </div>
        )}

        {repo && (
          <>
            {repo.quants.length > 0 && (
              <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap', marginBottom: 8 }}>
                <button
                  type="button"
                  className={`action-button ${selectedQuant === 'ALL' ? 'primary' : ''}`}
                  style={{ fontSize: 11, padding: '4px 8px' }}
                  onClick={() => setSelectedQuant('ALL')}
                >
                  {ar ? 'الكل' : 'All'}
                </button>
                {repo.quants.map((q) => (
                  <button
                    key={q}
                    type="button"
                    className={`action-button ${selectedQuant === q ? 'primary' : ''}`}
                    style={{ fontSize: 11, padding: '4px 8px' }}
                    onClick={() => {
                      setSelectedQuant(q);
                      const next = new Set<string>();
                      for (const f of repo.files) {
                        if (f.isConfig || f.quant === q) next.add(f.path);
                      }
                      setSelectedPaths(next);
                    }}
                  >
                    {q}
                  </button>
                ))}
              </div>
            )}

            <div style={{ maxHeight: 220, overflowY: 'auto', border: '1px solid var(--argon-border)', borderRadius: 8, padding: 8, marginBottom: 12 }}>
              {filteredFiles.map((f) => (
                <label
                  key={f.path}
                  style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '4px 2px', fontSize: 12, cursor: 'pointer' }}
                >
                  <input
                    type="checkbox"
                    checked={selectedPaths.has(f.path)}
                    onChange={() => togglePath(f.path)}
                  />
                  <span dir="ltr" className="ellipsis" style={{ flex: 1 }}>{f.path}</span>
                  {f.sha256 && <span style={{ fontSize: 10, opacity: 0.6 }}>SHA-256</span>}
                  <span dir="ltr" style={{ opacity: 0.8 }}>{formatBytes(f.size)}</span>
                </label>
              ))}
            </div>
          </>
        )}

        <div className="cookie-modal-foot">
          <span style={{ fontSize: 12, opacity: 0.75 }}>
            {repo ? `${selectedPaths.size} ${ar ? 'ملفات محددة' : 'files selected'}` : ''}
          </span>
          <div style={{ display: 'flex', gap: 8 }}>
            <button type="button" className="action-button" onClick={onClose}>
              {ar ? 'إلغاء' : 'Cancel'}
            </button>
            <button
              type="button"
              className="action-button primary"
              disabled={!repo || selectedPaths.size === 0}
              onClick={handleDownload}
            >
              <Download size={14} />
              <span>{ar ? 'إضافة للطابور' : 'Queue Model'}</span>
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
