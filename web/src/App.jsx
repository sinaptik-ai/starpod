import React, { useRef, useEffect, useCallback, useMemo } from 'react'
import './style.css'
import { AppProvider, useApp, isMobile } from './contexts/AppContext'
import { markSessionRead } from './lib/api'
import AuthGate, { useUser } from './components/AuthGate'
import OnboardingGate from './components/OnboardingGate'
import Header from './components/Header'
import Sidebar from './components/Sidebar'
import Chat from './components/Chat'
import InputBar from './components/InputBar'
import PreviewPanel from './components/PreviewPanel'
import ToastContainer from './components/Toasts'
import SettingsView from './components/settings/SettingsView'
import CronJobsView from './components/CronJobsView'
import FilesView from './components/FilesView'

function AppInner() {
  const { state, dispatch } = useApp()
  const { isAdmin } = useUser()
  const { wsStatus, settingsVisible, cronVisible, filesVisible, currentSessionId, currentSessionKey, previewUrl, chatTitle, settingsActiveTab } = state
  const wsRef = useRef(null)
  const chatRef = useRef(null)
  const toastsRef = useRef(null)
  const reconnectAttemptRef = useRef(0)

  // ── Dynamic page title ──
  const agentName = (state.config?.agent_name) || 'Starpod'
  const pageLabel = useMemo(() => {
    if (settingsVisible) return `Settings · ${(settingsActiveTab || 'general').replace(/^./, c => c.toUpperCase())}`
    if (cronVisible) return 'Schedules'
    if (filesVisible) return 'Files'
    if (chatTitle) return chatTitle
    return null
  }, [settingsVisible, settingsActiveTab, cronVisible, filesVisible, chatTitle])

  useEffect(() => {
    document.title = pageLabel ? `${pageLabel} — ${agentName}` : agentName
  }, [pageLabel, agentName])

  // ── WebSocket ──
  const connect = useCallback(() => {
    dispatch({ type: 'SET_WS_STATUS', payload: 'connecting' })
    const proto = location.protocol === 'https:' ? 'wss:' : 'ws:'
    const token = localStorage.getItem('starpod_api_key')
    let url = proto + '//' + location.host + '/ws'
    if (token) url += '?token=' + encodeURIComponent(token)

    const socket = new WebSocket(url)
    wsRef.current = socket

    socket.onopen = () => {
      dispatch({ type: 'SET_WS_STATUS', payload: 'connected' })
      reconnectAttemptRef.current = 0
    }

    socket.onclose = () => {
      dispatch({ type: 'SET_WS_STATUS', payload: 'disconnected' })
      wsRef.current = null
      if (chatRef.current) chatRef.current.handleStreamEvent({ type: 'ws_close' })
      const delay = Math.min(1000 * Math.pow(2, reconnectAttemptRef.current), 30000)
      reconnectAttemptRef.current++
      setTimeout(connect, delay)
    }

    socket.onerror = () => {}

    socket.onmessage = (event) => {
      let data
      try { data = JSON.parse(event.data) } catch { return }

      if (data.type === 'notification') {
        if (toastsRef.current) {
          toastsRef.current.showToast(data.job_name, data.result_preview, data.success, data.session_id)
        }
        // If the notification is for the currently active session, mark it read
        if (data.session_id && data.session_id === currentSessionIdRef.current) {
          markSessionRead(data.session_id)
        }
        fetchSessionList()
        return
      }

      if (data.type === 'stream_start' && data.session_id) {
        currentSessionIdRef.current = data.session_id
        dispatch({ type: 'SET_SESSION', payload: { id: data.session_id, key: null } })
        markSessionRead(data.session_id)
      }

      if (data.type === 'stream_end') {
        if (currentSessionIdRef.current) markSessionRead(currentSessionIdRef.current)
        fetchSessionList()
      }

      if (chatRef.current) chatRef.current.handleStreamEvent(data)
    }
  }, [dispatch])

  // Track currentSessionId in a ref for WS callback
  const currentSessionIdRef = useRef(currentSessionId)
  useEffect(() => { currentSessionIdRef.current = currentSessionId }, [currentSessionId])

  // ── Session fetching ──
  const fetchSessionList = useCallback(() => {
    const token = localStorage.getItem('starpod_api_key')
    const headers = {}
    if (token) headers['X-API-Key'] = token
    return fetch('/api/sessions?limit=50', { headers })
      .then(r => r.ok ? r.json() : Promise.reject())
      .then(sessions => {
        dispatch({ type: 'SET_SESSIONS', payload: sessions || [] })
        // Hydrate channel_session_key for the currently active session.
        // On page refresh the URL gives us the session id but the key is
        // initialised to a fresh random UUID. The server routes by
        // (channel, channel_session_key), so sending with the wrong key
        // forks off a brand new session and wipes the chat.
        const sid = currentSessionIdRef.current
        if (sid) {
          const match = (sessions || []).find(s => s.id === sid)
          if (match && match.channel_session_key) {
            dispatch({ type: 'SET_SESSION_KEY', payload: { id: sid, key: match.channel_session_key } })
          }
        }
        return sessions || []
      })
      .catch(() => [])
  }, [dispatch])

  useEffect(() => { connect(); fetchSessionList() }, [connect, fetchSessionList])

  // ── Send message ──
  const { selectedModel } = state
  const handleSend = useCallback((text, attachments) => {
    if (!wsRef.current || wsRef.current.readyState !== WebSocket.OPEN) return
    const payload = { type: 'message', text, channel_id: 'web', channel_session_key: currentSessionKey }
    if (attachments && attachments.length > 0) payload.attachments = attachments
    if (selectedModel) payload.model = selectedModel
    const json = JSON.stringify(payload)
    // Guard against payloads that exceed the WebSocket server limit
    // The WS limit is the configured max file size + overhead for base64 + JSON envelope
    const maxFileSize = state.config?.attachments?.max_file_size || (20 * 1024 * 1024)
    const MAX_WS_PAYLOAD = Math.max(maxFileSize * 2, 32 * 1024 * 1024)
    if (json.length > MAX_WS_PAYLOAD) {
      const sizeMB = (json.length / (1024 * 1024)).toFixed(1)
      if (chatRef.current) {
        chatRef.current.addUserMessage(text, attachments)
        chatRef.current.handleStreamEvent({
          type: 'error',
          message: `Message too large to send (${sizeMB} MB). Try removing some attachments or uploading smaller files.`,
        })
      }
      return
    }
    wsRef.current.send(json)
    if (chatRef.current) chatRef.current.addUserMessage(text, attachments)
  }, [currentSessionKey, selectedModel])

  // ── Session selection ──
  const handleSelectSession = useCallback((session) => {
    if (settingsVisible) dispatch({ type: 'HIDE_SETTINGS' })
    if (cronVisible) dispatch({ type: 'HIDE_CRON' })
    if (filesVisible) dispatch({ type: 'HIDE_FILES' })
    if (previewUrl) dispatch({ type: 'CLOSE_PREVIEW' })
    // Pass the session's real key through — do NOT fall back to a fresh
    // random UUID, because the server routes by channel_session_key and a
    // random key would fork the conversation into a brand new session on
    // the next send. SET_SESSION preserves the existing key when this one
    // is nullish (?? in the reducer).
    dispatch({ type: 'SET_SESSION', payload: { id: session.id, key: session.channel_session_key } })
    if (!session.is_read) {
      dispatch({ type: 'MARK_SESSION_READ', payload: session.id })
      markSessionRead(session.id)
    }
    if (isMobile()) dispatch({ type: 'CLOSE_SIDEBAR' })
  }, [dispatch, settingsVisible, cronVisible, filesVisible, previewUrl])

  // ── New chat ──
  const handleNewChat = useCallback(() => {
    if (settingsVisible) dispatch({ type: 'HIDE_SETTINGS' })
    if (cronVisible) dispatch({ type: 'HIDE_CRON' })
    if (filesVisible) dispatch({ type: 'HIDE_FILES' })
    if (previewUrl) dispatch({ type: 'CLOSE_PREVIEW' })
    dispatch({ type: 'NEW_CHAT' })
    if (isMobile()) dispatch({ type: 'CLOSE_SIDEBAR' })
  }, [dispatch, settingsVisible, cronVisible, filesVisible, previewUrl])

  // ── Toast navigation ──
  const handleToastNavigate = useCallback((sessionId) => {
    const session = state.sessions.find(s => s.id === sessionId)
    if (session) handleSelectSession(session)
    else {
      fetchSessionList().then(sessions => {
        const s = sessions.find(s => s.id === sessionId)
        if (s) handleSelectSession(s)
      })
    }
  }, [state.sessions, handleSelectSession, fetchSessionList])

  // ── Hash-based routing ──
  useEffect(() => {
    const onPopState = () => {
      const hash = window.location.hash
      if (hash.startsWith('#/settings')) {
        const tab = hash.split('/')[2]
        if (cronVisible) dispatch({ type: 'HIDE_CRON' })
        if (!settingsVisible) dispatch({ type: 'SHOW_SETTINGS' })
        if (tab) dispatch({ type: 'SET_SETTINGS_TAB', payload: tab })
      } else if (hash === '#/cron') {
        if (settingsVisible) dispatch({ type: 'HIDE_SETTINGS' })
        if (filesVisible) dispatch({ type: 'HIDE_FILES' })
        if (!cronVisible) dispatch({ type: 'SHOW_CRON' })
      } else if (hash === '#/files') {
        if (settingsVisible) dispatch({ type: 'HIDE_SETTINGS' })
        if (cronVisible) dispatch({ type: 'HIDE_CRON' })
        if (!filesVisible) dispatch({ type: 'SHOW_FILES' })
      } else if (hash.startsWith('#/chat/')) {
        const id = hash.slice('#/chat/'.length)
        if (settingsVisible) dispatch({ type: 'HIDE_SETTINGS' })
        if (cronVisible) dispatch({ type: 'HIDE_CRON' })
        if (filesVisible) dispatch({ type: 'HIDE_FILES' })
        if (id && id !== currentSessionIdRef.current) {
          dispatch({ type: 'SET_SESSION', payload: { id, _fromPopState: true } })
        }
      } else {
        if (settingsVisible) dispatch({ type: 'HIDE_SETTINGS' })
        if (cronVisible) dispatch({ type: 'HIDE_CRON' })
        if (filesVisible) dispatch({ type: 'HIDE_FILES' })
        if (currentSessionIdRef.current) {
          dispatch({ type: 'NEW_CHAT', _fromPopState: true })
        }
      }
    }
    window.addEventListener('popstate', onPopState)
    return () => window.removeEventListener('popstate', onPopState)
  }, [dispatch, settingsVisible, cronVisible, filesVisible])

  // ── Keyboard shortcuts ──
  useEffect(() => {
    const handler = (e) => {
      if (e.key === 'Escape') {
        if (previewUrl) { dispatch({ type: 'CLOSE_PREVIEW' }); return }
        if (settingsVisible) { dispatch({ type: 'HIDE_SETTINGS' }); return }
        if (cronVisible) { dispatch({ type: 'HIDE_CRON' }); return }
        if (filesVisible) { dispatch({ type: 'HIDE_FILES' }); return }
        if (state.sidebarOpen && isMobile()) dispatch({ type: 'CLOSE_SIDEBAR' })
      }
      if ((e.metaKey || e.ctrlKey) && e.key === ',') {
        e.preventDefault()
        dispatch({ type: settingsVisible ? 'HIDE_SETTINGS' : 'SHOW_SETTINGS' })
      }
    }
    document.addEventListener('keydown', handler)
    return () => document.removeEventListener('keydown', handler)
  }, [dispatch, settingsVisible, previewUrl, state.sidebarOpen])

  return (
    <>
      {/* Mobile sidebar overlay */}
      <div
        className="fixed inset-0 bg-black/50 z-[99]"
        id="sidebar-overlay"
        onClick={() => dispatch({ type: 'CLOSE_SIDEBAR' })}
        style={{
          opacity: state.sidebarOpen && isMobile() ? 1 : 0,
          pointerEvents: state.sidebarOpen && isMobile() ? 'auto' : 'none',
          transition: 'opacity 0.2s ease',
        }}
      />

      <div id="layout">
        {/* Sidebar */}
        <aside id="sidebar" className={`${state.sidebarOpen && !settingsVisible ? 'open' : ''} ${previewUrl ? 'transient' : ''}`}
          onMouseLeave={(e) => e.currentTarget.classList.remove('peeking')}>
          <Sidebar
            onSelectSession={handleSelectSession}
            onNewChat={handleNewChat}
          />
        </aside>

        {/* Main app */}
        <div id="app" role="main" className="flex flex-col min-w-0 flex-1">
          {settingsVisible && isAdmin ? (
            <SettingsView />
          ) : cronVisible ? (
            <CronJobsView />
          ) : filesVisible ? (
            <FilesView />
          ) : (
            <>
              <Header />
              <Chat ref={chatRef} wsRef={wsRef} onSendPrompt={(text) => handleSend(text, [])} />
              <InputBar
                onSend={handleSend}
                disabled={wsStatus !== 'connected'}
              />
            </>
          )}
        </div>

        {/* Preview panel */}
        <PreviewPanel />
      </div>

      {/* Toasts */}
      <ToastContainer ref={toastsRef} onNavigateToSession={handleToastNavigate} />
    </>
  )
}

export default function App() {
  return (
    <AuthGate>
      <OnboardingGate>
        <AppProvider>
          <AppInner />
        </AppProvider>
      </OnboardingGate>
    </AuthGate>
  )
}
