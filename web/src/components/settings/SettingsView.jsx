import { useApp } from '../../contexts/AppContext'
import IconButton from '../ui/IconButton'
import { BackIcon } from '../ui/Icons'
import GeneralTab from './GeneralTab'
import FileTab from './FileTab'
import HeartbeatTab from './HeartbeatTab'
import FrontendTab from './FrontendTab'
import MemoryTab from './MemoryTab'
import CronTab from './CronTab'
import SkillsTab from './SkillsTab'
import UsersTab from './UsersTab'
import ChannelsTab from './ChannelsTab'

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
      { id: 'cron', label: 'Cron' },
      { id: 'channels', label: 'Channels' },
      { id: 'skills', label: 'Skills' },
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
    case 'cron': return <CronTab />
    case 'channels': return <ChannelsTab />
    case 'skills': return <SkillsTab />
    case 'users': return <UsersTab />
    default: return null
  }
}

export default function SettingsView() {
  const { state, dispatch } = useApp()
  const { settingsActiveTab } = state
  const activeLabel = allTabs.find(t => t.id === settingsActiveTab)?.label || 'General'

  return (
    <div className="flex h-[100dvh] bg-bg">
      {/* Settings sidebar — hidden on mobile via CSS */}
      <div className="settings-sidebar w-[220px] shrink-0 border-r border-border-subtle flex flex-col">
        <div className="flex items-center gap-3 h-12 px-4 shrink-0">
          <IconButton onClick={() => dispatch({ type: 'HIDE_SETTINGS' })} aria-label="Back">
            <BackIcon />
          </IconButton>
          <h1 className="text-primary text-sm font-semibold">Settings</h1>
        </div>

        <nav className="flex-1 overflow-y-auto px-3 py-2">
          {tabGroups.map((group, gi) => (
            <div key={group.label} className={gi > 0 ? 'mt-4' : ''}>
              <div className="px-2 pb-1.5 text-[11px] font-semibold text-dim tracking-wider uppercase">
                {group.label}
              </div>
              <div className="flex flex-col gap-0.5">
                {group.tabs.map(t => (
                  <button
                    key={t.id}
                    onClick={() => dispatch({ type: 'SET_SETTINGS_TAB', payload: t.id })}
                    className={`px-2.5 py-1.5 rounded-md text-[13px] text-left cursor-pointer transition-colors ${
                      settingsActiveTab === t.id
                        ? 'text-accent bg-accent-muted font-medium'
                        : 'text-secondary hover:text-primary hover:bg-elevated'
                    }`}
                  >
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
        {/* Mobile header — visible only on mobile via CSS */}
        <div className="settings-mobile-nav hidden shrink-0 border-b border-border-subtle">
          <div className="flex items-center gap-3 h-12 px-4">
            <IconButton onClick={() => dispatch({ type: 'HIDE_SETTINGS' })} aria-label="Back">
              <BackIcon />
            </IconButton>
            <h1 className="text-primary text-sm font-semibold">Settings</h1>
            <div className="ml-auto">
              <select
                value={settingsActiveTab}
                onChange={e => dispatch({ type: 'SET_SETTINGS_TAB', payload: e.target.value })}
                className="bg-elevated text-secondary text-xs rounded-md px-2 py-1.5 border border-border-main cursor-pointer"
              >
                {tabGroups.map(group => (
                  <optgroup key={group.label} label={group.label}>
                    {group.tabs.map(t => (
                      <option key={t.id} value={t.id}>{t.label}</option>
                    ))}
                  </optgroup>
                ))}
              </select>
            </div>
          </div>
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
