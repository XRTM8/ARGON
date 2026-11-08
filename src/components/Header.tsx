import {
  ArrowDownToLine,
  Send,
  Clipboard,
  History,
  Sliders,
  Puzzle
} from 'lucide-react';
import type { DownloadStats } from '../types/downloader';

export type MainViewTab = 'downloads' | 'sharing' | 'clipboard' | 'history' | 'browser' | 'settings';
export const CURRENT_VERSION = '2.1.0';

interface Props {
  lang: 'ar' | 'en';
  activeTab: MainViewTab;
  onTabChange: (tab: MainViewTab) => void;
  stats: DownloadStats;
  runtime: 'preview' | 'starting' | 'ready' | 'error';
}

export function Header({ lang, activeTab, onTabChange, stats, runtime }: Props) {
  const ar = lang === 'ar';

  const links = [
    {
      id: 'downloads' as const,
      label: ar ? 'التنزيلات' : 'Downloads',
      icon: ArrowDownToLine,
      count: stats.activeCount
    },
    {
      id: 'sharing' as const,
      label: ar ? 'المشاركة' : 'Sharing',
      icon: Send
    },
    {
      id: 'clipboard' as const,
      label: ar ? 'الحافظة' : 'Clipboard',
      icon: Clipboard
    },
    {
      id: 'history' as const,
      label: ar ? 'السجل' : 'Activity',
      icon: History
    },
    {
      id: 'browser' as const,
      label: ar ? 'إضافة المتصفح' : 'Browser extension',
      icon: Puzzle
    },
    {
      id: 'settings' as const,
      label: ar ? 'الإعدادات' : 'Settings',
      icon: Sliders
    }
  ];

  return (
    <aside className="argon-sidebar">
      {/* Brand Header */}
      <div className="argon-brand" dir="ltr">
        <div className="argon-emblem-wrap">
          <svg viewBox="0 0 24 24" width="20" height="20" fill="none" xmlns="http://www.w3.org/2000/svg">
            <path
              d="M12 2.5L20.5 7.5V16.5L12 21.5L3.5 16.5V7.5L12 2.5Z"
              stroke="currentColor"
              strokeWidth="1.8"
              strokeLinejoin="round"
            />
            <path
              d="M12 7.5L16.5 16.5H7.5L12 7.5Z"
              fill="currentColor"
              fillOpacity="0.25"
              stroke="currentColor"
              strokeWidth="1.6"
              strokeLinejoin="round"
            />
            <circle cx="12" cy="13.5" r="1.5" fill="currentColor" />
          </svg>
        </div>
        <div className="argon-brand-text">
          <strong>ARGON</strong>
          <span>BY OTIUM</span>
        </div>
      </div>

      {/* Navigation */}
      <nav className="sidebar-navigation" aria-label={ar ? 'أقسام التطبيق' : 'Application sections'}>
        {links.map(({ id, label, icon: Icon, count }) => (
          <button
            key={id}
            type="button"
            className={activeTab === id ? 'is-active' : ''}
            aria-current={activeTab === id ? 'page' : undefined}
            onClick={() => onTabChange(id)}
          >
            <Icon size={17} />
            <span>{label}</span>
            {Boolean(count && count > 0) && <b>{count}</b>}
          </button>
        ))}
      </nav>

      {/* Sidebar Footer */}
      <div className="sidebar-bottom">
        <div className="engine-status-pill">
          <span className={`status-dot ${runtime !== 'ready' ? 'offline' : ''}`} />
          <span>{({ preview: ar ? 'معاينة الواجهة' : 'Interface preview', starting: ar ? 'تهيئة المحرك…' : 'Starting engine…', ready: ar ? 'المحرك متصل' : 'Engine connected', error: ar ? 'المحرك غير متاح' : 'Engine unavailable' })[runtime]}</span>
        </div>
        <div className="version-tag" dir="ltr">
          <span>ARGON 2</span>
          <span>v{CURRENT_VERSION}</span>
        </div>
      </div>
    </aside>
  );
}

export default Header;
