import {
  ArrowDownToLine,
  Send,
  History,
  Sliders
} from 'lucide-react';
import { ArgonMark } from './ArgonMark';
import type { DownloadStats } from '../types/downloader';

export type MainViewTab = 'downloads' | 'sharing' | 'clipboard' | 'history' | 'browser' | 'settings';
export const CURRENT_VERSION = '2.4.0';

interface Props {
  lang: 'ar' | 'en';
  activeTab: MainViewTab;
  onTabChange: (tab: MainViewTab) => void;
  stats: DownloadStats;
  runtime: 'preview' | 'starting' | 'ready' | 'error';
}

export function Header({ lang, activeTab, onTabChange, stats, runtime }: Props) {
  const ar = lang === 'ar';
  const isFlowActive = activeTab === 'sharing' || activeTab === 'clipboard';

  const links: Array<{
    id: MainViewTab;
    label: string;
    icon: typeof ArrowDownToLine;
    count?: number;
    active: boolean;
  }> = [
    {
      id: 'downloads',
      label: ar ? 'التنزيلات' : 'Downloads',
      icon: ArrowDownToLine,
      count: stats.activeCount,
      active: activeTab === 'downloads'
    },
    {
      id: 'sharing',
      label: ar ? 'المشاركة' : 'Share',
      icon: Send,
      active: isFlowActive
    },
    {
      id: 'history',
      label: ar ? 'السجل' : 'History',
      icon: History,
      active: activeTab === 'history'
    },
    {
      id: 'settings',
      label: ar ? 'الإعدادات' : 'Settings',
      icon: Sliders,
      active: activeTab === 'settings' || activeTab === 'browser'
    }
  ];

  return (
    <aside className="argon-sidebar">
      <div className="argon-brand" dir="ltr">
        <div className="argon-emblem-wrap">
          <ArgonMark size={19} />
        </div>
        <div className="argon-brand-text">
          <strong>ARGON</strong>
        </div>
      </div>

      <nav className="sidebar-navigation" aria-label={ar ? 'أقسام التطبيق' : 'Application sections'}>
        {links.map(({ id, label, icon: Icon, count, active }) => (
          <button
            key={id}
            type="button"
            className={active ? 'is-active' : ''}
            aria-current={active ? 'page' : undefined}
            onClick={() => onTabChange(id)}
          >
            <Icon size={17} />
            <span>{label}</span>
            {Boolean(count && count > 0) && <b>{count}</b>}
          </button>
        ))}
      </nav>

      <div className="sidebar-bottom">
        <div className="engine-status-pill">
          <span className={`status-dot ${runtime !== 'ready' ? 'offline' : ''}`} />
          <span>
            {({
              preview: ar ? 'معاينة' : 'Preview',
              starting: ar ? 'جاري التشغيل…' : 'Starting…',
              ready: ar ? 'جاهز' : 'Ready',
              error: ar ? 'متوقف' : 'Offline'
            })[runtime]}
          </span>
        </div>
        <div className="version-tag" dir="ltr">
          <span>v{CURRENT_VERSION}</span>
        </div>
      </div>
    </aside>
  );
}

export default Header;
