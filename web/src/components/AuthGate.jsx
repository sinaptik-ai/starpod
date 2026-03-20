import React, { useState, useEffect, useCallback } from 'react'
import { apiHeaders } from '../lib/api'

const API_KEY_STORAGE = 'starpod_api_key'

export default function AuthGate({ children }) {
  const [status, setStatus] = useState('checking') // checking | authenticated | login
  const [key, setKey] = useState('')
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(false)

  const verify = useCallback(async (apiKey) => {
    const headers = { 'Content-Type': 'application/json' }
    if (apiKey) headers['X-API-Key'] = apiKey
    try {
      const resp = await fetch('/api/auth/verify', { headers })
      if (!resp.ok) return false
      const data = await resp.json()
      return data.authenticated
    } catch {
      return false
    }
  }, [])

  // Check on mount — try URL token first, then localStorage
  useEffect(() => {
    const params = new URLSearchParams(window.location.search)
    const urlToken = params.get('token')

    const tryAuth = async () => {
      // URL token takes priority (used by dev mode auto-login)
      if (urlToken) {
        const ok = await verify(urlToken)
        if (ok) {
          localStorage.setItem(API_KEY_STORAGE, urlToken)
          // Clean token from URL without reload
          params.delete('token')
          const clean = params.toString()
          window.history.replaceState({}, '', window.location.pathname + (clean ? '?' + clean : '') + window.location.hash)
          setStatus('authenticated')
          return
        }
      }
      // Fall back to stored key
      const stored = localStorage.getItem(API_KEY_STORAGE)
      const ok = await verify(stored)
      setStatus(ok ? 'authenticated' : 'login')
    }
    tryAuth()
  }, [verify])

  const handleSubmit = async (e) => {
    e.preventDefault()
    setError('')
    setLoading(true)
    const trimmed = key.trim()
    const ok = await verify(trimmed)
    setLoading(false)
    if (ok) {
      localStorage.setItem(API_KEY_STORAGE, trimmed)
      setStatus('authenticated')
    } else {
      setError('Invalid API key')
    }
  }

  if (status === 'checking') {
    return (
      <div className="flex items-center justify-center h-screen bg-bg">
        <div className="text-dim font-mono text-sm">connecting...</div>
      </div>
    )
  }

  if (status === 'authenticated') {
    return children
  }

  return (
    <div className="flex items-center justify-center h-screen bg-bg">
      <div className="w-full max-w-sm px-6">
        <div className="text-center mb-8">
          <div className="font-mono text-3xl font-extrabold tracking-tighter mb-2 bg-gradient-to-b from-primary to-muted bg-clip-text text-transparent select-none">
            starpod
          </div>
          <p className="text-sm text-dim font-mono">enter your API key</p>
        </div>

        <form onSubmit={handleSubmit}>
          <input
            type="password"
            value={key}
            onChange={(e) => setKey(e.target.value)}
            placeholder="sp_live_..."
            autoFocus
            className="w-full px-3 py-2.5 rounded-lg bg-surface border border-border-main text-primary font-mono text-sm placeholder:text-dim focus:outline-none focus:border-accent transition-colors"
          />
          {error && (
            <p className="mt-2 text-xs text-err font-mono">{error}</p>
          )}
          <button
            type="submit"
            disabled={loading || !key.trim()}
            className="mt-4 w-full py-2.5 rounded-lg bg-accent text-white font-mono text-sm font-medium hover:brightness-110 disabled:opacity-40 disabled:cursor-not-allowed transition-all"
          >
            {loading ? 'verifying...' : 'authenticate'}
          </button>
        </form>
      </div>
    </div>
  )
}
