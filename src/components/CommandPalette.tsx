import { useEffect, useMemo, useRef, useState } from 'react';
import {
  Search,
  Pause,
  Play,
  Sun,
  Moon,
  Globe,
  Gauge,
  ArrowDownToLine,
  Send,
  Sliders,
  History,
  Puzzle
} from 'lucide-react';
import type { MainViewTab } from './Header';

interface CommandItem {
  id: string;
  label: string;
  hint?: string;
  icon: typeof Search;
  run: () => void;
}

interface Props {
  open: boolean;
  onClose: () => void;
  lang: 'ar' | 'en';
  theme: 'dark' | 'light';
  onNavigate: (tab: MainViewTab) => void;
  onPauseAll: () => void;
  onResumeAll: () => void;
  onToggleTheme: () => void;
  onToggleLang: () => void;
  onSetSpeedLimit: (bytesPerSec: number) => void;
  onFocusIntake: () => void;
}

export function CommandPalette({
  open,
  onClose,
  lang,
  theme,
  onNavigate,
  onPauseAll,
  onResumeAll,
  onToggleTheme,
  onToggleLang,
  onSetSpeedLimit,
  onFocusIntake
}: Props) {
  const ar = lang === 'ar';
  const [query, setQuery] = useState('');
  const [activeIndex, setActiveIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const items: CommandItem[] = useMemo(
    () => [
      {
        id: 'new-download',
        label: ar ? 'إضافة رابط تنزيل جديد' : 'New download link',
        hint: 'Ctrl+N',
        icon: ArrowDownToLine,
        run: () => { onNavigate('downloads'); onFocusIntake(); }
      },
      {
        id: 'pause-all',
        label: ar ? 'إيقاف جميع التنزيلات مؤقتاً' : 'Pause all active downloads',
        icon: Pause,
        run: onPauseAll
      },
      {
        id: 'resume-all',
        label: ar ? 'استئناف جميع التنزيلات' : 'Resume all paused downloads',
        icon: Play,
        run: onResumeAll
      },
      {
        id: 'speed-unlimited',
        label: ar ? 'سرعة التنزيل: غير محدودة' : 'Speed limit: Unlimited',
        icon: Gauge,
        run: () => onSetSpeedLimit(0)
      },
      {
        id: 'speed-2m',
        label: ar ? 'تقييد السرعة إلى 2 MB/s' : 'Throttle speed to 2 MB/s',
        icon: Gauge,
        run: () => onSetSpeedLimit(2 * 1024 * 1024)
      },
      {
        id: 'speed-5m',
        label: ar ? 'تقييد السرعة إلى 5 MB/s' : 'Throttle speed to 5 MB/s',
        icon: Gauge,
        run: () => onSetSpeedLimit(5 * 1024 * 1024)
      },
      {
        id: 'nav-sharing',
        label: ar ? 'الانتقال إلى المشاركة المحلية (Flow)' : 'Open Local Sharing (Flow)',
        icon: Send,
        run: () => onNavigate('sharing')
      },
      {
        id: 'nav-history',
        label: ar ? 'الانتقال إلى سجل العمليات' : 'Open Activity Log',
        icon: History,
        run: () => onNavigate('history')
      },
      {
        id: 'nav-browser',
        label: ar ? 'إعداد إضافة المتصفح' : 'Open Browser Extension Setup',
        icon: Puzzle,
        run: () => onNavigate('browser')
      },
      {
        id: 'nav-settings',
        label: ar ? 'فتح الإعدادات المتقدمة' : 'Open Settings',
        icon: Sliders,
        run: () => onNavigate('settings')
      },
      {
        id: 'toggle-theme',
        label:
          theme === 'dark'
            ? ar ? 'التبديل إلى المظهر الفاتح' : 'Switch to Light theme'
            : ar ? 'التبديل إلى المظهر الداكن' : 'Switch to Dark theme',
        icon: theme === 'dark' ? Sun : Moon,
        run: onToggleTheme
      },
      {
        id: 'toggle-lang',
        label: ar ? 'تغيير اللغة إلى English' : 'Switch language to العربية',
        icon: Globe,
        run: onToggleLang
      }
    ],
    [ar, theme, onNavigate, onFocusIntake, onPauseAll, onResumeAll, onSetSpeedLimit, onToggleTheme, onToggleLang]
  );

  const filtered = items.filter((item) =>
    !query.trim() || item.label.toLowerCase().includes(query.trim().toLowerCase())
  );

  useEffect(() => {
    if (open) {
      setQuery('');
      setActiveIndex(0);
      setTimeout(() => inputRef.current?.focus(), 20);
    }
  }, [open]);

  if (!open) return null;

  return (
    <div className="dialog-backdrop" onClick={onClose}>
      <div
        className="command-palette"
        role="dialog"
        aria-modal="true"
        aria-label={ar ? 'لوحة الأوامر السريعة' : 'Command Palette'}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="command-search">
          <Search size={16} />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setActiveIndex(0);
            }}
            onKeyDown={(e) => {
              if (e.key === 'Escape') onClose();
              else if (e.key === 'ArrowDown') {
                e.preventDefault();
                setActiveIndex((i) => Math.min(filtered.length - 1, i + 1));
              } else if (e.key === 'ArrowUp') {
                e.preventDefault();
                setActiveIndex((i) => Math.max(0, i - 1));
              } else if (e.key === 'Enter' && filtered[activeIndex]) {
                e.preventDefault();
                filtered[activeIndex].run();
                onClose();
              }
            }}
            placeholder={ar ? 'اكتب أمراً أو انتقل إلى قسم…' : 'Type a command or jump to a view…'}
          />
          <kbd>ESC</kbd>
        </div>

        <div className="command-list">
          {filtered.length > 0 ? (
            filtered.map((item, idx) => {
              const Icon = item.icon;
              return (
                <button
                  key={item.id}
                  type="button"
                  className={`command-item ${idx === activeIndex ? 'is-focused' : ''}`}
                  onMouseEnter={() => setActiveIndex(idx)}
                  onClick={() => {
                    item.run();
                    onClose();
                  }}
                >
                  <span className="command-item-meta">
                    <Icon size={15} className="accent-text" />
                    <span>{item.label}</span>
                  </span>
                  {item.hint && <kbd>{item.hint}</kbd>}
                </button>
              );
            })
          ) : (
            <div className="quiet-empty" style={{ padding: 24 }}>
              <p>{ar ? 'لا توجد أوامر مطابقة.' : 'No matching commands.'}</p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
