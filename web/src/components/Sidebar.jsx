import React from 'react'
import { useApp, isMobile } from '../contexts/AppContext'
import { formatSessionDate } from '../lib/utils'
import { markSessionRead } from '../lib/api'
import IconButton from './ui/IconButton'
import { ComposeIcon, CloseIcon, GearIcon } from './ui/Icons'

function Sidebar({ onSelectSession, onNewChat }) {
  const { state, dispatch } = useApp()
  const { sidebarOpen, currentSessionId, sessions, settingsVisible, cronVisible } = state

  function closeSidebar() {
    dispatch({ type: 'CLOSE_SIDEBAR' })
  }

  function newChat() {
    if (onNewChat) onNewChat()
    else {
      dispatch({ type: 'NEW_CHAT' })
      if (isMobile()) closeSidebar()
    }
  }

  function handleSessionClick(session) {
    dispatch({ type: 'SET_SESSION', payload: { id: session.id, key: session.channel_session_key } })
    if (!session.is_read) {
      dispatch({ type: 'MARK_SESSION_READ', payload: session.id })
      markSessionRead(session.id)
    }
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
    const aUnread = a.is_read ? 0 : 1
    const bUnread = b.is_read ? 0 : 1
    if (aUnread !== bUnread) return bUnread - aUnread
    return new Date(b.last_message_at || b.created_at) - new Date(a.last_message_at || a.created_at)
  })

  function handleCronClick() {
    dispatch({ type: 'SHOW_CRON' })
    if (isMobile()) closeSidebar()
  }

  // Render the sidebar inner content — the <aside id="sidebar"> wrapper is in App.jsx
  return (
    <>
      {/* Header with branding and toggle */}
      <div className="flex items-center justify-between px-4 h-12 shrink-0">
        <span className="font-mono text-sm font-bold tracking-tight text-primary">starpod</span>
        <button
          onClick={closeSidebar}
          className="text-muted hover:text-primary p-1.5 rounded-lg hover:bg-elevated transition-colors cursor-pointer"
          id="sidebar-close"
          aria-label="Close sidebar"
        >
          <svg className="w-4 h-4 stroke-current fill-none stroke-2" viewBox="0 0 24 24" strokeLinecap="round">
            <rect x="3" y="3" width="18" height="18" rx="2" />
            <line x1="9" y1="3" x2="9" y2="21" />
          </svg>
        </button>
      </div>

      {/* Nav items */}
      <div className="px-3 pb-2 flex flex-col gap-1">
        <button
          onClick={newChat}
          className="flex items-center gap-2.5 px-3 py-2 rounded-lg text-secondary hover:text-primary hover:bg-elevated transition-colors cursor-pointer text-[13px] w-full text-left"
          id="new-chat-btn"
        >
          <svg className="w-4 h-4 stroke-current fill-none stroke-[1.5]" viewBox="0 0 24 24" strokeLinecap="round" strokeLinejoin="round">
            <path d="M12 20h9" /><path d="M16.5 3.5a2.121 2.121 0 013 3L7 19l-4 1 1-4L16.5 3.5z" />
          </svg>
          <span>New Chat</span>
        </button>
        <button
          onClick={handleCronClick}
          className={`flex items-center gap-2.5 px-3 py-2 rounded-lg transition-colors cursor-pointer text-[13px] w-full text-left ${cronVisible ? 'text-accent bg-accent-muted' : 'text-secondary hover:text-primary hover:bg-elevated'}`}
        >
          <svg className="w-4 h-4 stroke-current fill-none stroke-[1.5]" viewBox="0 0 24 24" strokeLinecap="round" strokeLinejoin="round">
            <circle cx="12" cy="12" r="10" /><polyline points="12 6 12 12 16 14" />
          </svg>
          <span>Cron Jobs</span>
        </button>
      </div>

      {/* Chats label */}
      <div className="px-4 pt-2 pb-1 border-t border-border-subtle">
        <span className="text-xs font-semibold text-muted tracking-wider uppercase">Chats</span>
      </div>

      {/* Session list */}
      <div className="flex-1 overflow-y-auto px-3 py-2" id="session-list">
        {sorted.length === 0 ? (
          <div className="text-center text-dim text-xs py-8">
            <p>No conversations yet</p>
            <button
              onClick={newChat}
              className="mt-2 text-accent hover:text-accent-soft transition-colors cursor-pointer"
            >
              Start a new chat
            </button>
          </div>
        ) : (
          sorted.map(s => {
            const active = s.id === currentSessionId
            const unread = !s.is_read && !active
            const summary = s.title || s.summary || 'New conversation'
            const date = formatSessionDate(s.last_message_at || s.created_at)
            const msgs = s.message_count || 0
            const closed = s.is_closed ? ' \u00b7 closed' : ''

            return (
              <button
                key={s.id}
                className={`session-item px-3.5 py-3 rounded-lg cursor-pointer mb-1 w-full text-left${active ? ' active' : ''}`}
                data-sid={s.id}
                onClick={() => handleSessionClick(s)}
              >
                <div className="flex items-start gap-2">
                  {unread && <span className="unread-dot" />}
                  <div className="flex-1 min-w-0">
                    <div className={`text-[13px] leading-snug line-clamp-2 break-words${active ? ' text-primary font-medium' : ' text-secondary'}`}>
                      {summary}
                    </div>
                    <div className="text-[11px] text-dim mt-1 flex gap-2">
                      <span>{date}</span>
                      <span>{msgs} message{msgs !== 1 ? 's' : ''}{closed}</span>
                    </div>
                  </div>
                </div>
              </button>
            )
          })
        )}
      </div>

      {/* Settings button — sticky footer */}
      <div className="sidebar-settings-footer">
        <button
          className={`sidebar-settings-btn${settingsVisible ? ' active' : ''}`}
          onClick={handleSettingsClick}
          aria-label="Settings"
        >
          <GearIcon className="w-4 h-4 stroke-current fill-none stroke-[1.5]" />
          <span>Settings</span>
        </button>
      </div>
    </>
  )
}

export default Sidebar
