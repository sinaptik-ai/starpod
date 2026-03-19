import { useApp } from '../../contexts/AppContext'
import GeneralTab from './GeneralTab'
import FileTab from './FileTab'
import FrontendTab from './FrontendTab'
import MemoryTab from './MemoryTab'
import CronTab from './CronTab'
import UsersTab from './UsersTab'

const tabs = [
  { id: 'general', label: 'General' },
  { id: 'soul', label: 'Soul' },
  { id: 'heartbeat', label: 'Heartbeat' },
  { id: 'frontend', label: 'Frontend' },
  { id: 'memory', label: 'Memory' },
  { id: 'cron', label: 'Cron' },
  { id: 'users', label: 'Users' },
]

function TabContent({ tab }) {
  switch (tab) {
    case 'general': return <GeneralTab />
    case 'soul': return <FileTab fileName="SOUL.md" description="Defines the agent's personality, tone, and behavior." rows={24} />
    case 'heartbeat': return <FileTab fileName="HEARTBEAT.md" description="Instructions the agent follows on a recurring heartbeat schedule." rows={20} />
    case 'frontend': return <FrontendTab />
    case 'memory': return <MemoryTab />
    case 'cron': return <CronTab />
    case 'users': return <UsersTab />
    default: return null
  }
}

export default function SettingsView() {
  const { state, dispatch } = useApp()
  const { settingsActiveTab } = state

  return (
    <div className="flex flex-col h-[100dvh] bg-bg">
      {/* Fixed header */}
      <div className="shrink-0 border-b border-border-subtle">
        <div className="max-w-[740px] mx-auto px-5">
          <div className="flex items-center gap-3 h-12">
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

          {/* Tab bar */}
          <div className="flex gap-1 overflow-x-auto pb-0">
            {tabs.map(t => (
              <button
                key={t.id}
                onClick={() => dispatch({ type: 'SET_SETTINGS_TAB', payload: t.id })}
                className={`settings-tab px-3 py-2 text-xs font-medium cursor-pointer whitespace-nowrap transition-colors ${
                  settingsActiveTab === t.id ? 'active text-accent' : 'text-muted hover:text-secondary'
                }`}
              >
                {t.label}
              </button>
            ))}
          </div>
        </div>
      </div>

      {/* Scrollable content */}
      <div className="flex-1 overflow-y-auto">
        <div className="max-w-[740px] mx-auto px-5 py-6">
          <TabContent tab={settingsActiveTab} />
        </div>
      </div>
    </div>
  )
}
