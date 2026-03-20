import { createContext, useContext, useReducer } from 'react'
import { generateUUID } from '../lib/utils'

function initialSettingsFromHash() {
  const hash = window.location.hash
  if (hash.startsWith('#/settings')) {
    const tab = hash.split('/')[2]
    return { settings: { visible: true, tab: tab || 'general' }, cronVisible: false, sessionId: null }
  }
  if (hash === '#/cron') {
    return { settings: { visible: false, tab: 'general' }, cronVisible: true, sessionId: null }
  }
  if (hash.startsWith('#/chat/')) {
    const id = hash.slice('#/chat/'.length)
    if (id) return { settings: { visible: false, tab: 'general' }, cronVisible: false, sessionId: id }
  }
  return { settings: { visible: false, tab: 'general' }, cronVisible: false, sessionId: null }
}

const initSettings = initialSettingsFromHash()

const initialState = {
  wsStatus: 'connecting',
  sidebarOpen: window.innerWidth > 768,
  settingsVisible: initSettings.settings.visible,
  settingsActiveTab: initSettings.settings.tab,
  cronVisible: initSettings.cronVisible,
  currentSessionId: initSettings.sessionId,
  currentSessionKey: generateUUID(),
  sessions: [],
  previewUrl: null,
}

function appReducer(state, action) {
  switch (action.type) {
    case 'SET_WS_STATUS':
      return { ...state, wsStatus: action.payload }

    case 'TOGGLE_SIDEBAR':
      return { ...state, sidebarOpen: !state.sidebarOpen }

    case 'OPEN_SIDEBAR':
      return { ...state, sidebarOpen: true }

    case 'CLOSE_SIDEBAR':
      return { ...state, sidebarOpen: false }

    case 'SHOW_SETTINGS':
      window.history.pushState(null, '', '#/settings/' + state.settingsActiveTab)
      return { ...state, settingsVisible: true, cronVisible: false }

    case 'HIDE_SETTINGS':
      window.history.pushState(null, '', '#/')
      return { ...state, settingsVisible: false }

    case 'SHOW_CRON':
      window.history.pushState(null, '', '#/cron')
      return { ...state, cronVisible: true, settingsVisible: false }

    case 'HIDE_CRON': {
      const chatHash2 = state.currentSessionId ? '#/chat/' + state.currentSessionId : '#/'
      window.history.pushState(null, '', chatHash2)
      return { ...state, cronVisible: false }
    }

    case 'SET_SETTINGS_TAB':
      if (state.settingsVisible) window.history.replaceState(null, '', '#/settings/' + action.payload)
      return { ...state, settingsActiveTab: action.payload }

    case 'SET_SESSION':
      return {
        ...state,
        currentSessionId: action.payload.id,
        currentSessionKey: action.payload.key ?? state.currentSessionKey,
      }

    case 'NEW_CHAT':
      return {
        ...state,
        currentSessionId: null,
        currentSessionKey: generateUUID(),
      }

    case 'SET_SESSIONS':
      return { ...state, sessions: action.payload || [] }

    case 'OPEN_PREVIEW':
      return { ...state, previewUrl: action.payload }

    case 'CLOSE_PREVIEW':
      return { ...state, previewUrl: null }

    default:
      return state
  }
}

export const AppContext = createContext(null)

export function AppProvider({ children }) {
  const [state, dispatch] = useReducer(appReducer, initialState)
  return (
    <AppContext.Provider value={{ state, dispatch }}>
      {children}
    </AppContext.Provider>
  )
}

export function useApp() {
  const ctx = useContext(AppContext)
  if (!ctx) throw new Error('useApp must be used within AppProvider')
  return ctx
}

export const isMobile = () => window.innerWidth <= 768
