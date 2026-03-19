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
    <div className="max-w-[740px] mx-auto px-5 py-6">
      <div className="flex items-center gap-3 mb-6">
        <button
          onClick={() => dispatch({ type: 'HIDE_SETTINGS' })}
          className="text-muted hover:text-primary transition-colors cursor-pointer text-sm"
        >
          &larr; Back
        </button>
        <h1 className="text-primary text-lg font-semibold">Settings</h1>
      </div>

      <div className="flex gap-1 mb-6 overflow-x-auto border-b border-border-subtle pb-0">
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

      <TabContent tab={settingsActiveTab} />
    </div>
  )
}
