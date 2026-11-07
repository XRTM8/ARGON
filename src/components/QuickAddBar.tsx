import React, { useState, useEffect } from 'react';
import {
  Link2,
  ArrowDownToLine,
  ChevronDown,
  ChevronUp,
  Clipboard,
  AlertCircle,
  Loader2
} from 'lucide-react';

interface QuickAddBarProps {
  lang: 'ar' | 'en';
  onAdd: (url: string, filename?: string, segments?: number) => void;
  detectedClipboardUrl?: string | null;
  onClearClipboardUrl?: () => void;
}

export const QuickAddBar: React.FC<QuickAddBarProps> = ({
  lang,
  onAdd,
  detectedClipboardUrl,
  onClearClipboardUrl
}) => {
  const isAr = lang === 'ar';
  const [url, setUrl] = useState('');
  const [customFilename, setCustomFilename] = useState('');
  const [segments, setSegments] = useState(16);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [isResolving, setIsResolving] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  useEffect(() => {
    if (detectedClipboardUrl && !url) {
      setUrl(detectedClipboardUrl);
    }
  }, [detectedClipboardUrl]);

  const handlePasteClipboard = async () => {
    try {
      const text = await navigator.clipboard?.readText?.();
      if (text && /^https?:\/\/.+/i.test(text.trim())) {
        setUrl(text.trim());
        setErrorMessage(null);
      }
    } catch {
      // Clipboard denied
    }
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const cleanUrl = url.trim();
    if (!cleanUrl) return;

    if (!cleanUrl.startsWith('http://') && !cleanUrl.startsWith('https://') && !cleanUrl.startsWith('magnet:')) {
      setErrorMessage(
        isAr
          ? 'يرجى إدخال رابط صالح يبدأ بـ http:// أو https:// أو magnet:'
          : 'Please enter a valid HTTP, HTTPS, or magnet URL'
      );
      return;
    }

    setErrorMessage(null);
    setIsResolving(true);

    try {
      onAdd(cleanUrl, customFilename.trim() || undefined, segments);
      setUrl('');
      setCustomFilename('');
      setShowAdvanced(false);
      onClearClipboardUrl?.();
    } catch (err) {
      setErrorMessage(err instanceof Error ? err.message : 'حدث خطأ في معالجة الرابط');
    } finally {
      setIsResolving(false);
    }
  };

  return (
    <div className="w-full bg-[#111116] border border-white/[0.08] hover:border-white/[0.14] rounded-2xl p-3 sm:p-4 shadow-xl transition-all">
      <form onSubmit={handleSubmit} className="space-y-3">
        {/* Main Input Row */}
        <div className="flex flex-col sm:flex-row items-stretch gap-2">
          <div className="relative flex-1">
            <Link2 className="w-4 h-4 text-zinc-500 absolute left-3.5 rtl:right-3.5 rtl:left-auto top-1/2 -translate-y-1/2 pointer-events-none" />
            <input
              type="text"
              value={url}
              onChange={(e) => {
                setUrl(e.target.value);
                if (errorMessage) setErrorMessage(null);
              }}
              placeholder={
                isAr
                  ? 'الصق رابط التحميل المباشر هنا (ملفات، وسائط، تيك توك، روابط مضغوطة)...'
                  : 'Paste direct download link here (files, media, streams, archives)...'
              }
              className="w-full bg-[#09090d] border border-white/[0.08] focus:border-cyan-400/60 rounded-xl pl-9 rtl:pr-9 rtl:pl-24 pr-24 py-2.5 text-xs sm:text-sm text-zinc-100 placeholder-zinc-500 focus:outline-none focus:ring-1 focus:ring-cyan-400/30 transition-all font-mono"
            />

            {/* Quick Paste inside Input button */}
            <button
              type="button"
              onClick={handlePasteClipboard}
              className="absolute right-2 rtl:left-2 rtl:right-auto top-1/2 -translate-y-1/2 px-2.5 py-1 rounded-lg bg-white/[0.04] hover:bg-white/[0.08] text-zinc-400 hover:text-cyan-300 text-xs font-mono flex items-center gap-1 transition-all border border-white/[0.05]"
              title={isAr ? 'لصق من الحافظة' : 'Paste from clipboard'}
            >
              <Clipboard className="w-3 h-3 text-cyan-400" />
              <span className="hidden sm:inline text-[11px]">{isAr ? 'لصق' : 'Paste'}</span>
            </button>
          </div>

          {/* Submit Action Button */}
          <button
            type="submit"
            disabled={!url.trim() || isResolving}
            className="px-5 py-2.5 rounded-xl bg-gradient-to-r from-cyan-400 to-cyan-500 text-zinc-950 font-bold text-xs sm:text-sm flex items-center justify-center gap-2 hover:from-cyan-300 hover:to-cyan-400 active:scale-95 transition-all shadow-[0_0_20px_rgba(0,229,255,0.25)] disabled:opacity-40 disabled:pointer-events-none shrink-0"
          >
            {isResolving ? (
              <>
                <Loader2 className="w-4 h-4 animate-spin" />
                <span>{isAr ? 'جاري الفحص...' : 'Resolving...'}</span>
              </>
            ) : (
              <>
                <ArrowDownToLine className="w-4 h-4 stroke-[2.5]" />
                <span>{isAr ? 'بدء التنزيل' : 'Download'}</span>
              </>
            )}
          </button>
        </div>

        {/* Inline Error Notice if any */}
        {errorMessage && (
          <div className="p-2.5 rounded-xl bg-rose-500/10 border border-rose-500/25 flex items-start gap-2 text-rose-300 text-xs font-mono animate-in-fade">
            <AlertCircle className="w-4 h-4 shrink-0 text-rose-400 mt-0.5" />
            <span className="leading-relaxed">{errorMessage}</span>
          </div>
        )}

        {/* Advanced Options Toggle */}
        <div className="flex items-center justify-between text-xs pt-0.5">
          <button
            type="button"
            onClick={() => setShowAdvanced(!showAdvanced)}
            className="text-[11px] font-mono text-zinc-400 hover:text-cyan-300 flex items-center gap-1 transition-colors"
          >
            <span>{isAr ? 'خيارات متقدمة (اسم الملف، المسارات)' : 'Advanced (filename, threads)'}</span>
            {showAdvanced ? <ChevronUp className="w-3.5 h-3.5" /> : <ChevronDown className="w-3.5 h-3.5" />}
          </button>

          <span className="text-[10px] font-mono text-zinc-500">
            {isAr ? `${segments} اتصالات متوازية` : `${segments} parallel streams`}
          </span>
        </div>

        {/* Expandable Advanced Options */}
        {showAdvanced && (
          <div className="pt-2 border-t border-white/[0.04] grid grid-cols-1 sm:grid-cols-2 gap-3 text-xs font-mono animate-in-fade">
            <div>
              <label className="text-[11px] text-zinc-400 block mb-1">
                {isAr ? 'اسم ملف مخصص (اختياري):' : 'Custom Filename (optional):'}
              </label>
              <input
                type="text"
                value={customFilename}
                onChange={(e) => setCustomFilename(e.target.value)}
                placeholder="myfile.ext"
                className="w-full bg-[#09090d] border border-white/[0.08] rounded-xl px-3 py-1.5 text-xs text-zinc-200 placeholder-zinc-600 focus:outline-none focus:border-cyan-400/40"
              />
            </div>

            <div>
              <label className="text-[11px] text-zinc-400 block mb-1">
                {isAr ? 'مسارات التسريع المتوازية:' : 'Parallel Streams:'}
              </label>
              <div className="grid grid-cols-4 gap-1.5">
                {[4, 8, 16, 32].map((s) => (
                  <button
                    key={s}
                    type="button"
                    onClick={() => setSegments(s)}
                    className={`py-1.5 rounded-lg border text-center font-bold text-xs transition-all ${
                      segments === s
                        ? 'border-cyan-400 text-zinc-950 bg-cyan-400 shadow-[0_0_12px_rgba(0,229,255,0.3)]'
                        : 'border-white/[0.06] text-zinc-400 hover:text-zinc-200 bg-white/[0.02]'
                    }`}
                  >
                    {s}x
                  </button>
                ))}
              </div>
            </div>
          </div>
        )}
      </form>
    </div>
  );
};
