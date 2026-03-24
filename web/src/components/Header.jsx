import React, { useState, useRef, useEffect } from 'react'
import { useApp } from '../contexts/AppContext'
import IconButton from './ui/IconButton'
import ViewHeader from './ui/ViewHeader'
import { ChevronDownIcon, EllipsisIcon } from './ui/Icons'

/** Extract the model name (part after "/") from a "provider/model" spec. */
function modelLabel(spec) {
  if (!spec) return 'model'
  const idx = spec.indexOf('/')
  return idx >= 0 ? spec.slice(idx + 1) : spec
}

function Header() {
  const { state, dispatch } = useApp()
  const { wsStatus, selectedModel, chatTitle } = state

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

  function selectModel(spec) {
    dispatch({ type: 'SET_MODEL', payload: spec === models[0] ? null : spec })
    setDropdownOpen(false)
  }

  const modelSelector = (
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
  )

  return (
    <ViewHeader
      border={true}
      left={modelSelector}
      center={chatTitle ? (
        <span className="text-xs text-muted truncate block">{chatTitle}</span>
      ) : null}
      right={<>
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
      </>}
    />
  )
}

export default Header
