import { useApp } from '../../contexts/AppContext'
import IconButton from '../ui/IconButton'
import ViewHeader from '../ui/ViewHeader'
import { BackIcon } from '../ui/Icons'
import GeneralTab from './GeneralTab'
import FileTab from './FileTab'
import HeartbeatTab from './HeartbeatTab'
import FrontendTab from './FrontendTab'
import MemoryTab from './MemoryTab'
import CronTab from './CronTab'
import InternetTab from './InternetTab'
import SkillsTab from './SkillsTab'
import UsersTab from './UsersTab'
import ChannelsTab from './ChannelsTab'
import BrowserTab from './BrowserTab'
import CompactionTab from './CompactionTab'
import CostsTab from './CostsTab'
import VaultTab from './VaultTab'

// 14px stroke-based icons for the sidebar
const ico = (d) => <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.75" strokeLinecap="round" strokeLinejoin="round">{d}</svg>

const icons = {
  general:   ico(<><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></>),
  soul:      ico(<><path d="M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78L12 21.23l8.84-8.84a5.5 5.5 0 0 0 0-7.78z"/></>),
  heartbeat: ico(<><polyline points="22 12 18 12 15 21 9 3 6 12 2 12"/></>),
  boot:      ico(<><polyline points="4 17 10 11 4 5"/><line x1="12" y1="19" x2="20" y2="19"/></>),
  bootstrap: ico(<><path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"/></>),
  frontend:  ico(<><rect x="2" y="3" width="20" height="14" rx="2" ry="2"/><line x1="8" y1="21" x2="16" y2="21"/><line x1="12" y1="17" x2="12" y2="21"/></>),
  memory:    ico(<><path d="M4 4h16c1.1 0 2 .9 2 2v12c0 1.1-.9 2-2 2H4c-1.1 0-2-.9-2-2V6c0-1.1.9-2 2-2z"/><line x1="22" y1="10" x2="2" y2="10"/></>),
  internet:  ico(<><circle cx="12" cy="12" r="10"/><line x1="2" y1="12" x2="22" y2="12"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/></>),
  cron:      ico(<><circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/></>),
  compaction: ico(<><polyline points="4 14 10 14 10 20"/><polyline points="20 10 14 10 14 4"/><line x1="14" y1="10" x2="21" y2="3"/><line x1="3" y1="21" x2="10" y2="14"/></>),
  browser:   ico(<><path d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"/><polyline points="15 3 21 3 21 9"/><line x1="10" y1="14" x2="21" y2="3"/></>),
  channels:  ico(<><path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/></>),
  skills:    ico(<><polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"/></>),
  costs:     ico(<><line x1="12" y1="1" x2="12" y2="23"/><path d="M17 5H9.5a3.5 3.5 0 0 0 0 7h5a3.5 3.5 0 0 1 0 7H6"/></>),
  users:     ico(<><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M23 21v-2a4 4 0 0 0-3-3.87"/><path d="M16 3.13a4 4 0 0 1 0 7.75"/></>),
  vault:     ico(<><rect x="3" y="11" width="18" height="11" rx="2" ry="2"/><path d="M7 11V7a5 5 0 0 1 10 0v4"/></>),
}

const tabGroups = [
  {
    label: 'Agent',
    tabs: [
      { id: 'general', label: 'General' },
      { id: 'soul', label: 'Soul' },
      { id: 'heartbeat', label: 'Heartbeat' },
      { id: 'boot', label: 'Boot' },
      { id: 'bootstrap', label: 'Bootstrap' },
    ],
  },
  {
    label: 'Interface',
    tabs: [
      { id: 'frontend', label: 'Frontend' },
    ],
  },
  {
    label: 'System',
    tabs: [
      { id: 'memory', label: 'Memory' },
      { id: 'compaction', label: 'Compaction' },
      { id: 'internet', label: 'Internet' },
      { id: 'cron', label: 'Cron' },
      { id: 'browser', label: 'Browser (beta)' },
      { id: 'channels', label: 'Channels' },
      { id: 'skills', label: 'Skills' },
      { id: 'vault', label: 'Vault' },
      { id: 'costs', label: 'Costs' },
      { id: 'users', label: 'Users' },
    ],
  },
]

const allTabs = tabGroups.flatMap(g => g.tabs)

function TabContent({ tab }) {
  switch (tab) {
    case 'general': return <GeneralTab />
    case 'soul': return <FileTab fileName="SOUL.md" description="Defines the agent's personality, tone, and behavior." rows={24} />
    case 'heartbeat': return <HeartbeatTab />
    case 'boot': return <FileTab fileName="BOOT.md" description="Instructions executed once on agent startup." rows={20} />
    case 'bootstrap': return <FileTab fileName="BOOTSTRAP.md" description="Instructions for initial instance setup." rows={20} />
    case 'frontend': return <FrontendTab />
    case 'memory': return <MemoryTab />
    case 'compaction': return <CompactionTab />
    case 'internet': return <InternetTab />
    case 'cron': return <CronTab />
    case 'browser': return <BrowserTab />
    case 'channels': return <ChannelsTab />
    case 'skills': return <SkillsTab />
    case 'vault': return <VaultTab />
    case 'costs': return <CostsTab />
    case 'users': return <UsersTab />
    default: return null
  }
}

export default function SettingsView() {
  const { state, dispatch } = useApp()
  const { settingsActiveTab } = state
  const activeLabel = allTabs.find(t => t.id === settingsActiveTab)?.label || 'General'
  const activeGroup = tabGroups.find(g => g.tabs.some(t => t.id === settingsActiveTab))?.label

  return (
    <div className="flex flex-1 min-h-0">
      {/* Settings sidebar — hidden on mobile via CSS */}
      <div className="settings-sidebar w-[220px] shrink-0 border-r border-border-subtle flex flex-col">
        <div className="flex items-center gap-3 h-12 px-4 shrink-0 border-b border-border-subtle">
          <IconButton onClick={() => dispatch({ type: 'HIDE_SETTINGS' })} aria-label="Back">
            <BackIcon />
          </IconButton>
          <h1 className="text-primary text-sm font-semibold">Settings</h1>
        </div>

        <nav className="flex-1 overflow-y-auto px-3 py-2">
          {tabGroups.map((group, gi) => (
            <div key={group.label} className={gi > 0 ? 'mt-1 pt-3 border-t border-border-subtle' : ''}>
              <div className="px-2 pb-1.5 text-[11px] font-semibold text-dim tracking-wider uppercase">
                {group.label}
              </div>
              <div className="flex flex-col gap-0.5">
                {group.tabs.map(t => (
                  <button
                    key={t.id}
                    onClick={() => dispatch({ type: 'SET_SETTINGS_TAB', payload: t.id })}
                    className={`settings-tab-btn ${settingsActiveTab === t.id ? 'active' : ''}`}
                  >
                    <span className="settings-tab-icon">{icons[t.id]}</span>
                    {t.label}
                  </button>
                ))}
              </div>
            </div>
          ))}
        </nav>
      </div>

      {/* Main content area */}
      <div className="flex-1 flex flex-col min-w-0">
        {/* Mobile header */}
        <div className="settings-mobile-nav shrink-0">
          <ViewHeader
            title="Settings"
            right={
              <select
                value={settingsActiveTab}
                onChange={e => dispatch({ type: 'SET_SETTINGS_TAB', payload: e.target.value })}
                className="bg-elevated text-secondary text-xs rounded-none px-2 py-1.5 border border-border-main cursor-pointer"
              >
                {tabGroups.map(group => (
                  <optgroup key={group.label} label={group.label}>
                    {group.tabs.map(t => (
                      <option key={t.id} value={t.id}>{t.label}</option>
                    ))}
                  </optgroup>
                ))}
              </select>
            }
          />
        </div>

        {/* Desktop content header */}
        <div className="settings-desktop-header flex items-center h-12 px-6 shrink-0 border-b border-border-subtle">
          {activeGroup && <span className="text-muted text-xs mr-2">{activeGroup} /</span>}
          <h2 className="text-sm font-semibold text-primary tracking-tight">{activeLabel}</h2>
        </div>

        {/* Scrollable content */}
        <div className="flex-1 overflow-y-auto">
          <div className="max-w-[740px] mx-auto px-6 py-6">
            <TabContent tab={settingsActiveTab} />
          </div>
        </div>
      </div>
    </div>
  )
}
