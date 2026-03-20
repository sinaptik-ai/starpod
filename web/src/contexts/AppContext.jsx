import { createContext, useContext, useReducer } from 'react'
import { generateUUID } from '../lib/utils'

const UNREAD_KEY = 'starpod_read_sessions'

function loadReadSessions() {
  try {
    return new Set(JSON.parse(localStorage.getItem(UNREAD_KEY) || '[]'))
  } catch {
    return new Set()
  }
}

function persistReadSessions(readSessions) {
  localStorage.setItem(UNREAD_KEY, JSON.stringify([...readSessions]))
}

function initialSettingsFromHash() {
  const hash = window.location.hash
  if (hash.startsWith('#/settings')) {
    const tab = hash.split('/')[2]
    return { visible: true, tab: tab || 'general' }
  }
  return { visible: false, tab: 'general' }
}

const initSettings = initialSettingsFromHash()

const initialState = {
  wsStatus: 'connecting',
  sidebarOpen: window.innerWidth > 768,
  settingsVisible: initSettings.visible,
  settingsActiveTab: initSettings.tab,
  currentSessionId: null,
  currentSessionKey: generateUUID(),
  sessions: [],
  readSessions: loadReadSessions(),
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
      return { ...state, settingsVisible: true }

    case 'HIDE_SETTINGS':
      window.history.pushState(null, '', '#/')
      return { ...state, settingsVisible: false }

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

    case 'SET_SESSIONS': {
      const sessions = action.payload || []
      // Prune stale readSessions entries
      const ids = new Set(sessions.map(s => s.id))
      const pruned = new Set([...state.readSessions].filter(id => ids.has(id)))
      persistReadSessions(pruned)
      return { ...state, sessions, readSessions: pruned }
    }

    case 'MARK_READ': {
      const next = new Set(state.readSessions)
      next.add(action.payload)
      persistReadSessions(next)
      return { ...state, readSessions: next }
    }

    case 'MARK_UNREAD': {
      const next = new Set(state.readSessions)
      next.delete(action.payload)
      persistReadSessions(next)
      return { ...state, readSessions: next }
    }

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
