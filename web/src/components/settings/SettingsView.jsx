import { useApp } from '../../contexts/AppContext'
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

  const activeTabLabel = allTabs.find(t => t.id === settingsActiveTab)?.label || ''

  return (
    <div className="flex h-[100dvh] bg-bg">
      {/* Left sidebar navigation */}
      <div className="shrink-0 w-52 border-r border-border-subtle flex flex-col">
        <div className="flex items-center gap-3 h-12 px-4 shrink-0">
          <button
            onClick={() => dispatch({ type: 'HIDE_SETTINGS' })}
            className="text-muted hover:text-primary p-1.5 rounded-lg hover:bg-elevated transition-colors cursor-pointer"
          >
            <svg className="w-4 h-4 stroke-current fill-none stroke-2" viewBox="0 0 24 24" strokeLinecap="round">
              <path d="M19 12H5M12 19l-7-7 7-7" />
            </svg>
          </button>
          <h1 className="text-primary text-lg font-semibold">Settings</h1>
        </div>

        <nav className="flex-1 overflow-y-auto px-3 py-2">
          {tabGroups.map((group, gi) => (
            <div key={group.label} className={gi > 0 ? 'mt-4' : ''}>
              <div className="settings-nav-group-label">{group.label}</div>
              {group.tabs.map(t => (
                <button
                  key={t.id}
                  onClick={() => dispatch({ type: 'SET_SETTINGS_TAB', payload: t.id })}
                  className={`settings-nav-item ${settingsActiveTab === t.id ? 'active' : ''}`}
                >
                  {t.label}
                </button>
              ))}
            </div>
          ))}
        </nav>
      </div>

      {/* Scrollable content */}
      <div className="flex-1 overflow-y-auto">
        <div className="max-w-[740px] mx-auto px-5 py-6">
          <h2 className="text-primary text-lg font-semibold mb-4">{activeTabLabel}</h2>
          <TabContent tab={settingsActiveTab} />
        </div>
      </div>
    </div>
  )
}
