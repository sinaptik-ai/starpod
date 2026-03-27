import { useState, useEffect, useCallback } from 'react'
import { apiHeaders } from '../lib/api'
import OnboardingWizard from './onboarding/OnboardingWizard'

export default function OnboardingGate({ children }) {
  const [status, setStatus] = useState('checking') // checking | needs_setup | ready
  const [setupData, setSetupData] = useState(null)

  const checkSetup = useCallback(async () => {
    try {
      const resp = await fetch('/api/settings/setup-status', { headers: apiHeaders() })
      if (!resp.ok) {
        // If endpoint doesn't exist (404), skip onboarding
        setStatus('ready')
        return
      }
      const data = await resp.json()
      setSetupData(data)
      setStatus(data.complete ? 'ready' : 'needs_setup')
    } catch {
      // Network error or other failure — skip onboarding gracefully
      setStatus('ready')
    }
  }, [])

  useEffect(() => { checkSetup() }, [checkSetup])

  if (status === 'checking') {
    return (
      <div className="flex items-center justify-center h-screen bg-bg">
        <div className="text-dim font-mono text-sm">Loading...</div>
      </div>
    )
  }

  if (status === 'needs_setup') {
    return (
      <OnboardingWizard
        initialData={setupData}
        onComplete={() => setStatus('ready')}
      />
    )
  }

  return children
}
