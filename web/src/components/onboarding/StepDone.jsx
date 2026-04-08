import { useEffect, useState } from 'react'
import { StarpodIcon } from '../ui/Logo'
import { apiHeaders } from '../../lib/api'
import { useApp } from '../../contexts/AppContext'

export default function StepDone({ agentName, onComplete }) {
  const { dispatch } = useApp()
  const [pendingCount, setPendingCount] = useState(0)

  const handleGoToConnectors = () => {
    dispatch({ type: 'SET_SETTINGS_TAB', payload: 'connectors' })
    dispatch({ type: 'SHOW_SETTINGS' })
    onComplete()
  }

  useEffect(() => {
    let cancelled = false
    fetch('/api/settings/connectors', { headers: apiHeaders() })
      .then(r => (r.ok ? r.json() : null))
      .then(data => {
        if (cancelled || !data) return
        const list = Array.isArray(data) ? data : data.connectors || []
        setPendingCount(list.filter(c => c.status !== 'connected').length)
      })
      .catch(() => {})
    return () => { cancelled = true }
  }, [])

  return (
    <div className="ob-done">
      <StarpodIcon className="w-14 h-14 text-primary" />
      <h2 className="ob-done-name">{agentName} is ready</h2>
      <p className="ob-done-sub">
        Your agent is configured and waiting. Start a conversation to get going.
      </p>

      {pendingCount > 0 && (
        <button type="button" className="ob-done-hint" onClick={handleGoToConnectors}>
          <span className="ob-done-hint-dot" />
          <span>
            {pendingCount} tool{pendingCount !== 1 ? 's' : ''} need credentials — wire
            them up in <strong>Settings → Connectors →</strong>
          </span>
        </button>
      )}

      <button className="ob-btn-primary ob-done-btn" onClick={onComplete} autoFocus>
        Start chatting
      </button>
    </div>
  )
}
