import {
  ArrowDownToLine,
  Send,
  Clipboard,
  History,
  Sliders,
  Puzzle
} from 'lucide-react';
import { ArgonMark } from './ArgonMark';
import type { DownloadStats } from '../types/downloader';

export type MainViewTab = 'downloads' | 'sharing' | 'clipboard' | 'history' | 'browser' | 'settings';
export const CURRENT_VERSION = '2.3.0';

interface Props {
  lang: 'ar' | 'en';
  activeTab: MainViewTab;
  onTabChange: (tab: MainViewTab) => void;
  stats: DownloadStats;
  runtime: 'preview' | 'starting' | 'ready' | 'error';
}

export function Header({ lang, activeTab, onTabChange, stats, runtime }: Props) {
  const ar = lang === 'ar';

  const links: Array<{
    id: MainViewTab;
    label: string;
    icon: typeof ArrowDownToLine;
    count?: number;
    hideMobile?: boolean;
  }> = [
    {
      id: 'downloads',
      label: ar ? 'التنزيلات' : 'Downloads',
      icon: ArrowDownToLine,
      count: stats.activeCount
    },
    {
      id: 'sharing',
      label: ar ? 'المشاركة' : 'Flow',
      icon: Send
    },
    {
      id: 'clipboard',
      label: ar ? 'الحافظة' : 'Clipboard',
      icon: Clipboard,
      hideMobile: true
    },
    {
      id: 'history',
      label: ar ? 'السجل' : 'Activity',
      icon: History
    },
    {
      id: 'browser',
      label: ar ? 'إضافة المتصفح' : 'Extension',
      icon: Puzzle,
      hideMobile: true
    },
    {
      id: 'settings',
      label: ar ? 'الإعدادات' : 'Settings',
      icon: Sliders
    }
  ];

  const isFlowActive = activeTab === 'sharing' || activeTab === 'clipboard';

  return (
    <aside className="argon-sidebar">
      <div className="argon-brand" dir="ltr">
        <div className="argon-emblem-wrap">
          <ArgonMark size={20} />
        </div>
        <div className="argon-brand-text">
          <strong>ARGON</strong>
          <span>ENTERPRISE TRANSFER</span>
        </div>
      </div>

      <nav className="sidebar-navigation" aria-label={ar ? 'أقسام التطبيق' : 'Application sections'}>
        {links.map(({ id, label, icon: Icon, count, hideMobile }) => {
          const active = activeTab === id || (id === 'sharing' && isFlowActive && typeof window !== 'undefined' && window.innerWidth <= 760);
          return (
            <button
              key={id}
              type="button"
              className={`${active ? 'is-active' : ''} ${hideMobile ? 'hide-on-mobile' : ''}`}
              aria-current={active ? 'page' : undefined}
              onClick={() => onTabChange(id)}
            >
              <Icon size={17} />
              <span>{label}</span>
              {Boolean(count && count > 0) && <b>{count}</b>}
            </button>
          );
        })}
      </nav>

      <div className="sidebar-bottom">
        <div className="engine-status-pill">
          <span className={`status-dot ${runtime !== 'ready' ? 'offline' : ''}`} />
          <span>
            {({
              preview: ar ? 'معاينة الواجهة' : 'Interface preview',
              starting: ar ? 'تهيئة المحرك…' : 'Starting engine…',
              ready: ar ? 'المحرك متصل' : 'Engine connected',
              error: ar ? 'المحرك غير متاح' : 'Engine unavailable'
            })[runtime]}
          </span>
        </div>
        <div className="version-tag" dir="ltr">
          <span>ARGON CORE</span>
          <span>v{CURRENT_VERSION}</span>
        </div>
      </div>
    </aside>
  );
}

export default Header;
