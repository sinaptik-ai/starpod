import React from 'react'
import { useApp, isMobile } from '../contexts/AppContext'
import { formatSessionDate } from '../lib/utils'

function Sidebar({ onSelectSession }) {
  const { state, dispatch } = useApp()
  const { sidebarOpen, currentSessionId, sessions, readSessions, settingsVisible } = state

  function closeSidebar() {
    dispatch({ type: 'CLOSE_SIDEBAR' })
  }

  function newChat() {
    dispatch({ type: 'NEW_CHAT' })
    if (isMobile()) closeSidebar()
  }

  function handleSessionClick(session) {
    dispatch({ type: 'SET_SESSION', payload: { id: session.id, key: session.channel_session_key } })
    dispatch({ type: 'MARK_READ', payload: session.id })
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

  // Render the sidebar inner content — the <aside id="sidebar"> wrapper is in App.jsx
  return (
    <>
      {/* Header */}
      <div className="flex items-center justify-between px-4 h-12 border-b border-border-subtle shrink-0">
        <span className="text-xs font-semibold text-muted tracking-wider uppercase">Chats</span>
        <div className="flex items-center gap-1">
          <button
            onClick={newChat}
            className="text-muted hover:text-primary p-1.5 rounded-md hover:bg-elevated transition-colors cursor-pointer"
            id="new-chat-btn"
            title="New chat"
          >
            <svg className="w-3.5 h-3.5 stroke-current fill-none stroke-[1.5]" viewBox="0 0 24 24" strokeLinecap="round" strokeLinejoin="round">
              <path d="M12 20h9" /><path d="M16.5 3.5a2.121 2.121 0 013 3L7 19l-4 1 1-4L16.5 3.5z" />
            </svg>
          </button>
          <button
            onClick={closeSidebar}
            className="text-muted hover:text-primary p-1.5 rounded-md hover:bg-elevated transition-colors cursor-pointer"
            id="sidebar-close"
          >
            <svg className="w-3.5 h-3.5 stroke-current fill-none stroke-2" viewBox="0 0 24 24" strokeLinecap="round">
              <line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" />
            </svg>
          </button>
        </div>
      </div>

      {/* Session list */}
      <div className="flex-1 overflow-y-auto px-3 py-2" id="session-list">
        {sorted.length === 0 ? (
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

      {/* Settings button — sticky footer */}
      <div className="sidebar-settings-footer">
        <button
          className={`sidebar-settings-btn${settingsVisible ? ' active' : ''}`}
          onClick={handleSettingsClick}
        >
          <svg className="w-4 h-4 stroke-current fill-none stroke-[1.5]" viewBox="0 0 24 24" strokeLinecap="round" strokeLinejoin="round">
            <circle cx="12" cy="12" r="3" />
            <path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 010 2.83 2 2 0 01-2.83 0l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-2 2 2 2 0 01-2-2v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83 0 2 2 0 010-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 01-2-2 2 2 0 012-2h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 010-2.83 2 2 0 012.83 0l.06.06A1.65 1.65 0 009 4.68a1.65 1.65 0 001-1.51V3a2 2 0 012-2 2 2 0 012 2v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 0 2 2 0 010 2.83l-.06.06A1.65 1.65 0 0019.4 9a1.65 1.65 0 001.51 1H21a2 2 0 012 2 2 2 0 01-2 2h-.09a1.65 1.65 0 00-1.51 1z" />
          </svg>
          <span>Settings</span>
        </button>
      </div>
    </>
  )
}

export default Sidebar
