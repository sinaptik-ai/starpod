import React, { useRef, useEffect, useCallback } from 'react'
import './style.css'
import { AppProvider, useApp, isMobile } from './contexts/AppContext'
import { generateUUID } from './lib/utils'
import AuthGate from './components/AuthGate'
import Header from './components/Header'
import Sidebar from './components/Sidebar'
import Chat from './components/Chat'
import InputBar from './components/InputBar'
import PreviewPanel from './components/PreviewPanel'
import ToastContainer from './components/Toasts'
import SettingsView from './components/settings/SettingsView'

function AppInner() {
  const { state, dispatch } = useApp()
  const { wsStatus, settingsVisible, currentSessionId, currentSessionKey, previewUrl } = state
  const wsRef = useRef(null)
  const chatRef = useRef(null)
  const toastsRef = useRef(null)
  const reconnectAttemptRef = useRef(0)

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
        if (data.session_id && data.session_id !== currentSessionIdRef.current) {
          dispatch({ type: 'MARK_UNREAD', payload: data.session_id })
        }
        fetchSessionList()
        return
      }

      if (data.type === 'stream_start' && data.session_id) {
        currentSessionIdRef.current = data.session_id
        dispatch({ type: 'SET_SESSION', payload: { id: data.session_id, key: null } })
        dispatch({ type: 'MARK_READ', payload: data.session_id })
      }

      if (data.type === 'stream_end') {
        if (currentSessionIdRef.current) dispatch({ type: 'MARK_READ', payload: currentSessionIdRef.current })
        fetchSessionList()
      }

      if (chatRef.current) chatRef.current.handleStreamEvent(data)
    }
  }, [dispatch])

  // Track currentSessionId in a ref for WS callback
  const currentSessionIdRef = useRef(currentSessionId)
  useEffect(() => { currentSessionIdRef.current = currentSessionId }, [currentSessionId])

  useEffect(() => { connect() }, [connect])

  // ── Session fetching ──
  const fetchSessionList = useCallback(() => {
    const token = localStorage.getItem('starpod_api_key')
    const headers = {}
    if (token) headers['X-API-Key'] = token
    fetch('/api/sessions?limit=50', { headers })
      .then(r => r.ok ? r.json() : Promise.reject())
      .then(sessions => dispatch({ type: 'SET_SESSIONS', payload: sessions || [] }))
      .catch(() => {})
  }, [dispatch])

  // ── Send message ──
  const handleSend = useCallback((text, attachments) => {
    if (!wsRef.current || wsRef.current.readyState !== WebSocket.OPEN) return
    const payload = { type: 'message', text, channel_id: 'web', channel_session_key: currentSessionKey }
    if (attachments && attachments.length > 0) payload.attachments = attachments
    wsRef.current.send(JSON.stringify(payload))
    if (chatRef.current) chatRef.current.addUserMessage(text, attachments)
  }, [currentSessionKey])

  // ── Session selection ──
  const handleSelectSession = useCallback((session) => {
    if (settingsVisible) dispatch({ type: 'HIDE_SETTINGS' })
    dispatch({ type: 'SET_SESSION', payload: { id: session.id, key: session.channel_session_key || generateUUID() } })
    dispatch({ type: 'MARK_READ', payload: session.id })
    if (chatRef.current) chatRef.current.loadSession(session.id)
    if (isMobile()) dispatch({ type: 'CLOSE_SIDEBAR' })
  }, [dispatch, settingsVisible])

  // ── New chat ──
  const handleNewChat = useCallback(() => {
    if (settingsVisible) dispatch({ type: 'HIDE_SETTINGS' })
    dispatch({ type: 'NEW_CHAT' })
    if (chatRef.current) chatRef.current.showWelcome()
    if (isMobile()) dispatch({ type: 'CLOSE_SIDEBAR' })
  }, [dispatch, settingsVisible])

  // ── Toast navigation ──
  const handleToastNavigate = useCallback((sessionId) => {
    const session = state.sessions.find(s => s.id === sessionId)
    if (session) handleSelectSession(session)
    else {
      fetchSessionList()
      // Try again after fetch
      setTimeout(() => {
        const s = state.sessions.find(s => s.id === sessionId)
        if (s) handleSelectSession(s)
      }, 500)
    }
  }, [state.sessions, handleSelectSession, fetchSessionList])

  // ── Hash-based routing for /settings ──
  useEffect(() => {
    const onPopState = () => {
      const hash = window.location.hash
      if (hash.startsWith('#/settings')) {
        const tab = hash.split('/')[2]
        if (!settingsVisible) dispatch({ type: 'SHOW_SETTINGS' })
        if (tab) dispatch({ type: 'SET_SETTINGS_TAB', payload: tab })
      } else {
        if (settingsVisible) dispatch({ type: 'HIDE_SETTINGS' })
      }
    }
    window.addEventListener('popstate', onPopState)
    return () => window.removeEventListener('popstate', onPopState)
  }, [dispatch, settingsVisible])

  // ── Keyboard shortcuts ──
  useEffect(() => {
    const handler = (e) => {
      if ((e.metaKey || e.ctrlKey) && e.key === 'k') { e.preventDefault() }
      if (e.key === 'Escape') {
        if (previewUrl) { dispatch({ type: 'CLOSE_PREVIEW' }); return }
        if (settingsVisible) { dispatch({ type: 'HIDE_SETTINGS' }); return }
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

  // Settings is a full-page takeover
  if (settingsVisible) {
    return (
      <>
        <SettingsView />
        <ToastContainer ref={toastsRef} onNavigateToSession={handleToastNavigate} />
      </>
    )
  }

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
        <aside id="sidebar" className={`${state.sidebarOpen ? 'open' : ''} ${previewUrl ? 'transient' : ''}`}>
          <div id="sidebar-inner">
            <Sidebar
              onSelectSession={handleSelectSession}
              onNewChat={handleNewChat}
            />
          </div>
        </aside>

        {/* Main app */}
        <div id="app" className="flex flex-col min-w-0 flex-1">
          <Header onNewChat={handleNewChat} onToggleSidebar={() => { if (!state.sidebarOpen) fetchSessionList() }} />
          <Chat ref={chatRef} wsRef={wsRef} onSendPrompt={(text) => handleSend(text, [])} />
          <InputBar
            onSend={handleSend}
            disabled={wsStatus !== 'connected'}
          />
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
      <AppProvider>
        <AppInner />
      </AppProvider>
    </AuthGate>
  )
}
