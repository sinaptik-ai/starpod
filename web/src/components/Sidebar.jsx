import React, { useState, useMemo } from 'react'
import { useApp, isMobile } from '../contexts/AppContext'
import { useUser } from './AuthGate'
import { formatSessionDate } from '../lib/utils'
import { markSessionRead } from '../lib/api'
import IconButton from './ui/IconButton'
import { ComposeIcon, CloseIcon, GearIcon, SearchIcon } from './ui/Icons'

function groupSessionsByDate(sessions) {
  const now = new Date()
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate())
  const yesterday = new Date(today); yesterday.setDate(yesterday.getDate() - 1)
  const weekAgo = new Date(today); weekAgo.setDate(weekAgo.getDate() - 7)

  const groups = { today: [], yesterday: [], week: [], older: [] }

  for (const s of sessions) {
    const d = new Date(s.last_message_at || s.created_at)
    if (d >= today) groups.today.push(s)
    else if (d >= yesterday) groups.yesterday.push(s)
    else if (d >= weekAgo) groups.week.push(s)
    else groups.older.push(s)
  }

  const result = []
  if (groups.today.length) result.push({ label: 'Today', sessions: groups.today })
  if (groups.yesterday.length) result.push({ label: 'Yesterday', sessions: groups.yesterday })
  if (groups.week.length) result.push({ label: 'Last 7 days', sessions: groups.week })
  if (groups.older.length) result.push({ label: 'Older', sessions: groups.older })
  return result
}

function Sidebar({ onSelectSession, onNewChat }) {
  const { state, dispatch } = useApp()
  const { sidebarOpen, currentSessionId, sessions, settingsVisible, cronVisible, filesVisible, previewUrl } = state
  const { user } = useUser()
  const showFiles = !user || user.filesystem_enabled // no user = auth_disabled = show
  const isTransient = sidebarOpen && !!previewUrl
  const [searchQuery, setSearchQuery] = useState('')

  const cfg = window.__STARPOD__ || {}
  const agentName = cfg.agent_name || 'Starpod'

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

  function handleCronClick() {
    dispatch({ type: 'SHOW_CRON' })
    if (isMobile()) closeSidebar()
  }

  function handleFilesClick() {
    dispatch({ type: 'SHOW_FILES' })
    if (isMobile()) closeSidebar()
  }

  // Sort: unread first, then by date
  const sorted = useMemo(() => {
    let list = [...(sessions || [])]
    if (searchQuery.trim()) {
      const q = searchQuery.toLowerCase()
      list = list.filter(s => {
        const text = (s.title || s.summary || '').toLowerCase()
        return text.includes(q)
      })
    }
    list.sort((a, b) => {
      const aUnread = a.is_read ? 0 : 1
      const bUnread = b.is_read ? 0 : 1
      if (aUnread !== bUnread) return bUnread - aUnread
      return new Date(b.last_message_at || b.created_at) - new Date(a.last_message_at || a.created_at)
    })
    return list
  }, [sessions, searchQuery])

  const dateGroups = useMemo(() => groupSessionsByDate(sorted), [sorted])

  function renderSession(s) {
    const active = s.id === currentSessionId
    const unread = !s.is_read && !active
    const summary = s.title || s.summary || 'New conversation'

    return (
      <button
        key={s.id}
        className={`session-item px-3 py-2.5 rounded-lg cursor-pointer mb-0.5 w-full text-left${active ? ' active' : ''}`}
        data-sid={s.id}
        onClick={() => handleSessionClick(s)}
      >
        <div className="flex items-center gap-2">
          {unread && <span className="unread-dot" />}
          <div className="flex-1 min-w-0">
            <div className={`text-[13px] leading-snug truncate${active ? ' text-primary font-medium' : ' text-secondary'}`}>
              {summary}
            </div>
          </div>
        </div>
      </button>
    )
  }

  return (
    <>
      {/* Header with branding and toggle */}
      <div className="flex items-center justify-between px-4 h-12 shrink-0">
        <span className="font-mono text-sm font-bold tracking-tight text-primary">starpod</span>
        {!isTransient && (
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
        )}
      </div>

      {/* Nav items */}
      <div className="px-3 pb-2 flex flex-col gap-0.5">
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
        {showFiles && (
          <button
            onClick={handleFilesClick}
            className={`flex items-center gap-2.5 px-3 py-2 rounded-lg transition-colors cursor-pointer text-[13px] w-full text-left ${filesVisible ? 'text-accent bg-accent-muted' : 'text-secondary hover:text-primary hover:bg-elevated'}`}
          >
            <svg className="w-4 h-4 stroke-current fill-none stroke-[1.5]" viewBox="0 0 24 24" strokeLinecap="round" strokeLinejoin="round">
              <path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z" />
            </svg>
            <span>Files</span>
          </button>
        )}
      </div>

      {/* Search */}
      <div className="px-3 pb-2">
        <div className="sidebar-search">
          <SearchIcon className="w-3.5 h-3.5 stroke-current fill-none stroke-[1.5] shrink-0 text-dim" />
          <input
            type="text"
            placeholder="Search chats..."
            value={searchQuery}
            onChange={e => setSearchQuery(e.target.value)}
            className="sidebar-search-input"
          />
        </div>
      </div>

      {/* Session list */}
      <div className="flex-1 overflow-y-auto px-2 py-1" id="session-list">
        {sorted.length === 0 && !searchQuery ? (
          <div className="text-center text-dim text-xs py-8">
            <p>No conversations yet</p>
            <button
              onClick={newChat}
              className="mt-2 text-accent hover:text-accent-soft transition-colors cursor-pointer"
            >
              Start a new chat
            </button>
          </div>
        ) : sorted.length === 0 && searchQuery ? (
          <div className="text-center text-dim text-xs py-8">
            No results
          </div>
        ) : (
          dateGroups.map(group => (
            <div key={group.label} className="mb-1">
              <div className="px-3 pt-3 pb-1">
                <span className="text-[11px] font-semibold text-dim tracking-wider uppercase">{group.label}</span>
              </div>
              {group.sessions.map(renderSession)}
            </div>
          ))
        )}
      </div>

      {/* Footer with settings */}
      <div className="sidebar-footer">
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
