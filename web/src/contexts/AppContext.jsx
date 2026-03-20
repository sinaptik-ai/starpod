import { createContext, useContext, useReducer } from 'react'
import { generateUUID } from '../lib/utils'

function parseHash() {
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

const initHash = parseHash()

const initialState = {
  wsStatus: 'connecting',
  sidebarOpen: window.innerWidth > 768,
  settingsVisible: initHash.settings.visible,
  settingsActiveTab: initHash.settings.tab,
  cronVisible: initHash.cronVisible,
  currentSessionId: initHash.sessionId,
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

    case 'HIDE_SETTINGS': {
      const chatHash = state.currentSessionId ? '#/chat/' + state.currentSessionId : '#/'
      window.history.pushState(null, '', chatHash)
      return { ...state, settingsVisible: false }
    }

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
      if (!action.payload._fromPopState) {
        window.history.pushState(null, '', '#/chat/' + action.payload.id)
      }
      return {
        ...state,
        currentSessionId: action.payload.id,
        currentSessionKey: action.payload.key ?? state.currentSessionKey,
      }

    case 'NEW_CHAT':
      if (!action._fromPopState) {
        window.history.pushState(null, '', '#/')
      }
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
