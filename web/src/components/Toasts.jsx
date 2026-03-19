import React, { useState, useCallback, useEffect, useImperativeHandle, forwardRef } from 'react'
import { escapeHtml } from '../lib/utils'

let toastIdCounter = 0

const Toast = React.memo(function Toast({ toast, onRemove, onNavigate }) {
  useEffect(() => {
    const timer = setTimeout(() => {
      onRemove(toast.id)
    }, 6000)
    return () => clearTimeout(timer)
  }, [toast.id, onRemove])

  function handleClick() {
    if (toast.sessionId) {
      onRemove(toast.id)
      onNavigate(toast.sessionId)
    }
  }

  const preview = toast.preview.length > 120 ? toast.preview.slice(0, 120) + '\u2026' : toast.preview

  return (
    <div
      className={`toast ${toast.success ? 'toast-success' : 'toast-error'}`}
      style={toast.sessionId ? { cursor: 'pointer' } : undefined}
      onClick={handleClick}
    >
      <div className="toast-icon">{toast.success ? '\u2713' : '\u2717'}</div>
      <div className="toast-content">
        <div className="toast-title">{toast.jobName}</div>
        <div className="toast-body">{preview}</div>
      </div>
    </div>
  )
})

const ToastContainer = forwardRef(function ToastContainer({ onNavigateToSession }, ref) {
  const [toasts, setToasts] = useState([])

  const removeToast = useCallback((id) => {
    setToasts(prev => prev.filter(t => t.id !== id))
  }, [])

  const showToast = useCallback((jobName, preview, success, sessionId) => {
    const id = ++toastIdCounter
    setToasts(prev => [...prev, { id, jobName, preview, success, sessionId }])
  }, [])

  useImperativeHandle(ref, () => ({
    showToast,
  }), [showToast])

  // Also expose globally for WS handler
  useEffect(() => {
    window._showToast = showToast
    return () => { delete window._showToast }
  }, [showToast])

  function handleNavigate(sessionId) {
    if (onNavigateToSession) onNavigateToSession(sessionId)
  }

  if (toasts.length === 0) return null

  return (
    <div id="toast-container">
      {toasts.map(t => (
        <Toast
          key={t.id}
          toast={t}
          onRemove={removeToast}
          onNavigate={handleNavigate}
        />
      ))}
    </div>
  )
})

export default ToastContainer
