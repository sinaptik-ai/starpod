import { createContext, useContext, useReducer, useCallback } from 'react'
import { generateUUID } from '../lib/utils'
import { fetchConfig } from '../lib/api'

function parseHash() {
  const hash = window.location.hash
  if (hash.startsWith('#/settings')) {
    const tab = hash.split('/')[2]
    return { settings: { visible: true, tab: tab || 'general' }, cronVisible: false, sessionId: null }
  }
  if (hash === '#/cron') {
    return { settings: { visible: false, tab: 'general' }, cronVisible: true, filesVisible: false, sessionId: null }
  }
  if (hash === '#/files') {
    return { settings: { visible: false, tab: 'general' }, cronVisible: false, filesVisible: true, sessionId: null }
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
  filesVisible: initHash.filesVisible || false,
  currentSessionId: initHash.sessionId,
  currentSessionKey: generateUUID(),
  sessions: [],
  previewUrl: null,
  selectedModel: null, // null = use default (first in models list)
  chatTitle: null, // first user message, shown in header
  config: window.__STARPOD__ || {},
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
      return { ...state, settingsVisible: true, cronVisible: false, filesVisible: false, previewUrl: null }

    case 'HIDE_SETTINGS': {
      const chatHash = state.currentSessionId ? '#/chat/' + state.currentSessionId : '#/'
      window.history.pushState(null, '', chatHash)
      return { ...state, settingsVisible: false }
    }

    case 'SHOW_CRON':
      window.history.pushState(null, '', '#/cron')
      return { ...state, cronVisible: true, settingsVisible: false, filesVisible: false, previewUrl: null }

    case 'HIDE_CRON': {
      const chatHash2 = state.currentSessionId ? '#/chat/' + state.currentSessionId : '#/'
      window.history.pushState(null, '', chatHash2)
      return { ...state, cronVisible: false }
    }

    case 'SHOW_FILES':
      window.history.pushState(null, '', '#/files')
      return { ...state, filesVisible: true, settingsVisible: false, cronVisible: false, previewUrl: null }

    case 'HIDE_FILES': {
      const chatHash3 = state.currentSessionId ? '#/chat/' + state.currentSessionId : '#/'
      window.history.pushState(null, '', chatHash3)
      return { ...state, filesVisible: false }
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
        previewUrl: null,
      }

    case 'SET_SESSION_KEY':
      // Hydrate the channel_session_key for the current session without
      // touching any other state (no pushState, no previewUrl reset). Used
      // after a refresh where the URL gave us a session id but the key was
      // initialised to a fresh random UUID that won't match the server.
      if (action.payload.id !== state.currentSessionId) return state
      if (action.payload.key === state.currentSessionKey) return state
      return { ...state, currentSessionKey: action.payload.key }

    case 'NEW_CHAT':
      if (!action._fromPopState) {
        window.history.pushState(null, '', '#/')
      }
      return {
        ...state,
        currentSessionId: null,
        currentSessionKey: generateUUID(),
        chatTitle: null,
        previewUrl: null,
      }

    case 'SET_SESSIONS':
      return { ...state, sessions: action.payload || [] }

    case 'MARK_SESSION_READ':
      return {
        ...state,
        sessions: state.sessions.map(s =>
          s.id === action.payload ? { ...s, is_read: true } : s
        ),
      }

    case 'ARCHIVE_SESSION':
      return {
        ...state,
        sessions: state.sessions.filter(s => s.id !== action.payload),
        ...(state.currentSessionId === action.payload
          ? { currentSessionId: null, currentSessionKey: generateUUID(), chatTitle: null }
          : {}),
      }

    case 'SET_MODEL':
      return { ...state, selectedModel: action.payload }

    case 'SET_CHAT_TITLE':
      return { ...state, chatTitle: action.payload }

    case 'OPEN_PREVIEW':
      return { ...state, previewUrl: action.payload }

    case 'CLOSE_PREVIEW':
      return { ...state, previewUrl: null }

    case 'SET_CONFIG':
      return { ...state, config: action.payload }

    default:
      return state
  }
}

export const AppContext = createContext(null)

export function AppProvider({ children }) {
  const [state, dispatch] = useReducer(appReducer, initialState)
  const refreshConfig = useCallback(() => {
    fetchConfig().then(cfg => {
      if (cfg) dispatch({ type: 'SET_CONFIG', payload: cfg })
    })
  }, [])
  return (
    <AppContext.Provider value={{ state, dispatch, refreshConfig }}>
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
