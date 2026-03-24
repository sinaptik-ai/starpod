import React, { createContext, useContext, useState, useEffect, useCallback } from 'react'
import { apiHeaders } from '../lib/api'
import Logo from './ui/Logo'

const API_KEY_STORAGE = 'starpod_api_key'

const UserContext = createContext({ user: null, isAdmin: false })

export function useUser() {
  return useContext(UserContext)
}

export default function AuthGate({ children }) {
  const [status, setStatus] = useState('checking') // checking | authenticated | login
  const [user, setUser] = useState(null)
  const [key, setKey] = useState('')
  const [error, setError] = useState('')
  const [loading, setLoading] = useState(false)

  const verify = useCallback(async (apiKey) => {
    const headers = { 'Content-Type': 'application/json' }
    if (apiKey) headers['X-API-Key'] = apiKey
    try {
      const resp = await fetch('/api/auth/verify', { headers })
      if (!resp.ok) return null
      const data = await resp.json()
      if (data.authenticated) {
        return data.user || null
      }
      return null
    } catch {
      return null
    }
  }, [])

  // Check on mount — try URL token first, then localStorage
  useEffect(() => {
    const params = new URLSearchParams(window.location.search)
    const urlToken = params.get('token')

    const tryAuth = async () => {
      // URL token takes priority (used by dev mode auto-login)
      if (urlToken) {
        const u = await verify(urlToken)
        if (u !== null || urlToken) {
          // verify returns null for auth_disabled (no users) — that's still authenticated
          const resp = await fetch('/api/auth/verify', {
            headers: { 'Content-Type': 'application/json', 'X-API-Key': urlToken },
          })
          const data = await resp.json()
          if (data.authenticated) {
            localStorage.setItem(API_KEY_STORAGE, urlToken)
            params.delete('token')
            const clean = params.toString()
            window.history.replaceState({}, '', window.location.pathname + (clean ? '?' + clean : '') + window.location.hash)
            setUser(data.user || null)
            setStatus('authenticated')
            return
          }
        }
      }
      // Fall back to stored key
      const stored = localStorage.getItem(API_KEY_STORAGE)
      const headers = { 'Content-Type': 'application/json' }
      if (stored) headers['X-API-Key'] = stored
      try {
        const resp = await fetch('/api/auth/verify', { headers })
        const data = await resp.json()
        if (data.authenticated) {
          setUser(data.user || null)
          setStatus('authenticated')
        } else {
          setStatus('login')
        }
      } catch {
        setStatus('login')
      }
    }
    tryAuth()
  }, [verify])

  const handleSubmit = async (e) => {
    e.preventDefault()
    setError('')
    setLoading(true)
    const trimmed = key.trim()
    try {
      const resp = await fetch('/api/auth/verify', {
        headers: { 'Content-Type': 'application/json', 'X-API-Key': trimmed },
      })
      const data = await resp.json()
      setLoading(false)
      if (data.authenticated) {
        localStorage.setItem(API_KEY_STORAGE, trimmed)
        setUser(data.user || null)
        setStatus('authenticated')
      } else {
        setError('Invalid key. Check STARPOD_API_KEY in your .env file.')
      }
    } catch {
      setLoading(false)
      setError('Invalid key. Check STARPOD_API_KEY in your .env file.')
    }
  }

  if (status === 'checking') {
    return (
      <div className="flex items-center justify-center h-screen bg-bg">
        <div className="text-dim font-mono text-sm">Checking authentication...</div>
      </div>
    )
  }

  if (status === 'authenticated') {
    const isAdmin = !user || user.role === 'admin' // no user = auth_disabled = full access
    return (
      <UserContext.Provider value={{ user, isAdmin }}>
        {children}
      </UserContext.Provider>
    )
  }

  return (
    <div className="flex items-center justify-center h-screen bg-bg">
      <div className="w-full max-w-sm px-6">
        <div className="text-center mb-8">
          <div className="mb-2"><Logo /></div>
          <p className="text-sm text-dim font-mono">Sign in with your API key</p>
        </div>

        <form onSubmit={handleSubmit}>
          <input
            type="password"
            value={key}
            onChange={(e) => setKey(e.target.value)}
            placeholder="sp_live_..."
            autoFocus
            className="w-full px-3 py-2.5 rounded-none bg-surface border border-border-main text-primary font-mono text-sm placeholder:text-dim focus:outline-none focus:border-accent transition-colors"
          />
          {error && (
            <p className="mt-2 text-xs text-err font-mono">{error}</p>
          )}
          <button
            type="submit"
            disabled={loading || !key.trim()}
            className="mt-4 w-full py-2.5 rounded-none bg-accent text-bg font-mono text-sm font-medium hover:brightness-110 disabled:opacity-40 disabled:cursor-not-allowed transition-all"
          >
            {loading ? 'Signing in...' : 'Sign in'}
          </button>
        </form>
      </div>
    </div>
  )
}
