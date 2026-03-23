import React, { useState, useRef, useEffect, useCallback } from 'react'
import { useApp } from '../contexts/AppContext'
import IconButton from './ui/IconButton'
import { SidebarOpenIcon, ComposeIcon, ChevronDownIcon, EllipsisIcon } from './ui/Icons'

/** Extract the model name (part after "/") from a "provider/model" spec. */
function modelLabel(spec) {
  if (!spec) return 'model'
  const idx = spec.indexOf('/')
  return idx >= 0 ? spec.slice(idx + 1) : spec
}

function Header({ onToggleSidebar, onNewChat }) {
  const { state, dispatch } = useApp()
  const { wsStatus, sidebarOpen, previewUrl, currentSessionId, sessions, selectedModel } = state
  const sidebarVisible = sidebarOpen && !previewUrl
  const isTransient = sidebarOpen && !!previewUrl

  const cfg = window.__STARPOD__ || {}
  const models = cfg.models || []
  const agentName = cfg.agent_name || 'starpod'
  const activeModel = selectedModel || models[0] || null
  const hasMultiple = models.length > 1

  const [dropdownOpen, setDropdownOpen] = useState(false)
  const dropdownRef = useRef(null)

  // Close dropdown on outside click
  useEffect(() => {
    if (!dropdownOpen) return
    function handleClick(e) {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target)) {
        setDropdownOpen(false)
      }
    }
    document.addEventListener('mousedown', handleClick)
    return () => document.removeEventListener('mousedown', handleClick)
  }, [dropdownOpen])

  // Find current session title
  const currentSession = currentSessionId ? sessions.find(s => s.id === currentSessionId) : null
  const sessionTitle = currentSession?.title || currentSession?.summary || null

  const peekSidebar = useCallback(() => {
    const el = document.getElementById('sidebar')
    if (el) el.classList.add('peeking')
  }, [])

  function toggleSidebar() {
    if (isTransient) {
      peekSidebar()
      return
    }
    dispatch({ type: 'TOGGLE_SIDEBAR' })
    if (onToggleSidebar) onToggleSidebar()
  }

  function newChat() {
    if (onNewChat) onNewChat()
    else dispatch({ type: 'NEW_CHAT' })
  }

  function selectModel(spec) {
    dispatch({ type: 'SET_MODEL', payload: spec === models[0] ? null : spec })
    setDropdownOpen(false)
  }

  return (
    <header className="flex items-center gap-2 px-3 h-12 shrink-0" id="main-header">
      {!sidebarVisible && (
        <IconButton onClick={toggleSidebar} onMouseEnter={isTransient ? peekSidebar : undefined} aria-label="Open sidebar">
          <SidebarOpenIcon />
        </IconButton>
      )}
      <IconButton onClick={newChat} id="new-chat-header-btn" title="New chat" aria-label="New chat">
        <ComposeIcon className="w-4 h-4 stroke-current fill-none stroke-2" />
      </IconButton>

      {/* Model selector */}
      <div className="relative" ref={dropdownRef}>
        <button
          className="model-pill"
          onClick={hasMultiple ? () => setDropdownOpen(!dropdownOpen) : () => {
            dispatch({ type: 'SHOW_SETTINGS' })
            dispatch({ type: 'SET_SETTINGS_TAB', payload: 'general' })
          }}
          title={hasMultiple ? 'Switch model' : 'Change model'}
        >
          <span>{activeModel ? modelLabel(activeModel) : agentName}</span>
          {hasMultiple && <ChevronDownIcon className="w-2.5 h-2.5 opacity-50" />}
        </button>

        {dropdownOpen && (
          <div className="model-dropdown">
            {models.map(spec => (
              <button
                key={spec}
                className={`model-dropdown-item${spec === activeModel ? ' active' : ''}`}
                onClick={() => selectModel(spec)}
              >
                <span className="model-dropdown-name">{modelLabel(spec)}</span>
                <span className="model-dropdown-provider">{spec.split('/')[0]}</span>
              </button>
            ))}
          </div>
        )}
      </div>

      {/* Session title (centered) */}
      {sessionTitle && (
        <div className="flex-1 min-w-0 flex justify-center">
          <span className="text-xs text-muted truncate max-w-[300px] select-none">
            {sessionTitle}
          </span>
        </div>
      )}
      {!sessionTitle && <div className="flex-1" />}

      {/* Right side */}
      <div className="flex items-center gap-1.5">
        <span
          className={`w-2 h-2 rounded-full shrink-0 dot-${wsStatus}`}
          title={`WebSocket: ${wsStatus}`}
        />
        {wsStatus !== 'connected' && (
          <span className="text-[11px] text-muted select-none">
            {wsStatus === 'connecting' ? 'connecting...' : 'offline'}
          </span>
        )}
        <IconButton
          onClick={() => dispatch({ type: state.settingsVisible ? 'HIDE_SETTINGS' : 'SHOW_SETTINGS' })}
          aria-label="More options"
          title="Settings"
        >
          <EllipsisIcon className="w-4 h-4" />
        </IconButton>
      </div>
    </header>
  )
}

export default Header
