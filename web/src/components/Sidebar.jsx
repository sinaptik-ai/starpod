import React, { useEffect, useCallback } from 'react'
import { useApp, isMobile } from '../contexts/AppContext'
import { escapeHtml, formatSessionDate } from '../lib/utils'
import { authHeaders } from '../lib/api'

function Sidebar({ onSelectSession }) {
  const { state, dispatch } = useApp()
  const { sidebarOpen, currentSessionId, sessions, readSessions, settingsVisible } = state

  const fetchSessions = useCallback(() => {
    fetch('/api/sessions?limit=50', { headers: authHeaders() })
      .then(r => r.ok ? r.json() : Promise.reject(r.statusText))
      .then(data => {
        dispatch({ type: 'SET_SESSIONS', sessions: data || [] })
      })
      .catch(() => {})
  }, [dispatch])

  useEffect(() => {
    if (sidebarOpen) {
      fetchSessions()
    }
  }, [sidebarOpen, fetchSessions])

  function closeSidebar() {
    dispatch({ type: 'CLOSE_SIDEBAR' })
  }

  function newChat() {
    dispatch({ type: 'NEW_CHAT' })
    if (isMobile()) closeSidebar()
  }

  function handleSessionClick(session) {
    dispatch({ type: 'SET_SESSION', sessionId: session.id, sessionKey: session.channel_session_key })
    dispatch({ type: 'MARK_READ', sessionId: session.id })
    if (isMobile()) closeSidebar()
    if (onSelectSession) onSelectSession(session)
  }

  function handleSettingsClick() {
    if (settingsVisible) {
      dispatch({ type: 'HIDE_SETTINGS' })
    } else {
      dispatch({ type: 'SHOW_SETTINGS' })
      if (isMobile()) closeSidebar()
    }
  }

  // Sort: unread first, then by date
  const sorted = [...(sessions || [])].sort((a, b) => {
    const aUnread = readSessions.has(a.id) ? 0 : 1
    const bUnread = readSessions.has(b.id) ? 0 : 1
    if (aUnread !== bUnread) return bUnread - aUnread
    return new Date(b.last_message_at || b.created_at) - new Date(a.last_message_at || a.created_at)
  })

  return (
    <>
      <div id="sidebar" className={sidebarOpen ? 'open' : ''}>
        <div id="sidebar-inner">
          {/* Header */}
          <div className="flex items-center justify-between px-4 h-12 shrink-0 border-b border-border-subtle">
            <span className="text-sm font-semibold text-primary">Chats</span>
            <div className="flex items-center gap-1">
              <button
                onClick={newChat}
                className="text-muted hover:text-primary p-1.5 rounded-lg hover:bg-elevated transition-colors cursor-pointer"
                id="new-chat-btn"
                title="New chat"
              >
                <svg className="w-4 h-4 stroke-current fill-none stroke-2" viewBox="0 0 24 24" strokeLinecap="round">
                  <path d="M12 20h9M16.5 3.5a2.121 2.121 0 013 3L7 19l-4 1 1-4L16.5 3.5z" />
                </svg>
              </button>
              <button
                onClick={closeSidebar}
                className="text-muted hover:text-primary p-1.5 rounded-lg hover:bg-elevated transition-colors cursor-pointer"
                id="sidebar-close"
                aria-label="Close sidebar"
              >
                <svg className="w-4 h-4 stroke-current fill-none stroke-2" viewBox="0 0 24 24" strokeLinecap="round">
                  <path d="M18 6L6 18M6 6l12 12" />
                </svg>
              </button>
            </div>
          </div>

          {/* Session list */}
          <div className="flex-1 overflow-y-auto px-2 py-2" id="session-list">
            {(!sorted || sorted.length === 0) ? (
              <div className="text-center text-dim text-xs py-8">No conversations yet</div>
            ) : (
              sorted.map(s => {
                const active = s.id === currentSessionId
                const unread = !readSessions.has(s.id) && !active
                const summary = s.title || s.summary || 'Untitled conversation'
                const date = formatSessionDate(s.last_message_at || s.created_at)
                const msgs = s.message_count || 0
                const closed = s.is_closed ? ' \u00b7 ended' : ''

                return (
                  <div
                    key={s.id}
                    className={`session-item px-3.5 py-3 rounded-lg cursor-pointer mb-1${active ? ' active' : ''}`}
                    data-sid={s.id}
                    onClick={() => handleSessionClick(s)}
                  >
                    <div className="flex items-start gap-2">
                      {unread && <span className="unread-dot" />}
                      <div className="flex-1 min-w-0">
                        <div className={`text-[13px] leading-snug line-clamp-2 break-words${active ? ' text-primary font-medium' : ' text-secondary'}`}>
                          {summary}
                        </div>
                        <div className="font-mono text-[11px] text-dim mt-1 flex gap-2">
                          <span>{date}</span>
                          <span>{msgs} msg{msgs !== 1 ? 's' : ''}{closed}</span>
                        </div>
                      </div>
                    </div>
                  </div>
                )
              })
            )}
          </div>

          {/* Settings footer */}
          <div className="sidebar-settings-footer">
            <button
              className={`sidebar-settings-btn${settingsVisible ? ' active' : ''}`}
              id="sidebar-settings-btn"
              onClick={handleSettingsClick}
            >
              <svg className="w-4 h-4 stroke-current fill-none stroke-2" viewBox="0 0 24 24" strokeLinecap="round" strokeLinejoin="round">
                <circle cx="12" cy="12" r="3" />
                <path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 010 2.83 2 2 0 01-2.83 0l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83 0 2 2 0 010-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 010-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 112.83-2.83l.06.06A1.65 1.65 0 009 4.68a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 2.83l-.06.06a1.65 1.65 0 00-.33 1.82V9a1.65 1.65 0 001.51 1H21a2 2 0 010 4h-.09a1.65 1.65 0 00-1.51 1z" />
              </svg>
              <span>Settings</span>
            </button>
          </div>
        </div>
      </div>

      {/* Mobile overlay */}
      <div
        id="sidebar-overlay"
        className={`fixed inset-0 bg-black/50 z-50 ${sidebarOpen && isMobile() ? 'active' : ''}`}
        style={{ opacity: sidebarOpen && isMobile() ? 1 : 0, pointerEvents: sidebarOpen && isMobile() ? 'auto' : 'none', transition: 'opacity 0.2s ease' }}
        onClick={closeSidebar}
      />
    </>
  )
}

export default Sidebar
