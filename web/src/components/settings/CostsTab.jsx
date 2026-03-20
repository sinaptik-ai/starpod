import { useState, useEffect, useCallback } from 'react'
import { apiHeaders } from '../../lib/api'
import { Card } from './fields'

const PERIODS = [
  { value: '7d', label: '7 days' },
  { value: '30d', label: '30 days' },
  { value: '90d', label: '90 days' },
  { value: 'all', label: 'All time' },
]

function formatCost(usd) {
  return `$${usd.toFixed(2)}`
}

function formatTokens(n) {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`
  return n.toLocaleString()
}

export default function CostsTab() {
  const [data, setData] = useState(null)
  const [period, setPeriod] = useState('30d')
  const [error, setError] = useState(null)

  const load = useCallback(() => {
    setError(null)
    fetch(`/api/settings/costs?period=${period}`, { headers: apiHeaders() })
      .then(r => {
        if (!r.ok) throw new Error('Failed to load')
        return r.json()
      })
      .then(d => setData(d))
      .catch(e => setError(e.message))
  }, [period])

  useEffect(() => { load() }, [load])

  return (
    <>
      {/* Period selector */}
      <div className="flex gap-1 mb-4">
        {PERIODS.map(p => (
          <button
            key={p.value}
            onClick={() => setPeriod(p.value)}
            className={`px-3 py-1.5 text-xs font-medium rounded-md cursor-pointer transition-colors ${
              period === p.value
                ? 'bg-accent/15 text-accent'
                : 'text-muted hover:text-secondary hover:bg-elevated'
            }`}
          >
            {p.label}
          </button>
        ))}
      </div>

      {error && (
        <div className="text-red-400 text-sm py-4 text-center">{error}</div>
      )}

      {!data && !error && (
        <div className="text-dim text-sm py-8 text-center">Loading...</div>
      )}

      {data && (
        <>
          {/* Overview */}
          <Card title="Overview">
            <div className="grid grid-cols-3 gap-4 py-2">
              <div className="text-center">
                <div className="text-2xl font-semibold text-primary">{formatCost(data.total_cost_usd)}</div>
                <div className="text-xs text-muted mt-1">Total cost</div>
              </div>
              <div className="text-center">
                <div className="text-2xl font-semibold text-primary">{formatTokens(data.total_input_tokens + data.total_output_tokens)}</div>
                <div className="text-xs text-muted mt-1">Total tokens</div>
              </div>
              <div className="text-center">
                <div className="text-2xl font-semibold text-primary">{data.total_turns.toLocaleString()}</div>
                <div className="text-xs text-muted mt-1">Turns</div>
              </div>
            </div>
          </Card>

          {/* By user */}
          {data.by_user.length > 0 && (
            <Card title="By User">
              <table className="w-full text-sm">
                <thead>
                  <tr className="text-muted text-xs">
                    <th className="text-left py-1.5 font-medium">User</th>
                    <th className="text-right py-1.5 font-medium">Cost</th>
                    <th className="text-right py-1.5 font-medium">Input</th>
                    <th className="text-right py-1.5 font-medium">Output</th>
                    <th className="text-right py-1.5 font-medium">Turns</th>
                  </tr>
                </thead>
                <tbody>
                  {data.by_user.map(u => (
                    <tr key={u.user_id} className="border-t border-border-subtle">
                      <td className="py-1.5 text-primary">{u.user_id}</td>
                      <td className="py-1.5 text-right text-primary font-mono text-xs">{formatCost(u.total_cost_usd)}</td>
                      <td className="py-1.5 text-right text-muted font-mono text-xs">{formatTokens(u.total_input_tokens)}</td>
                      <td className="py-1.5 text-right text-muted font-mono text-xs">{formatTokens(u.total_output_tokens)}</td>
                      <td className="py-1.5 text-right text-muted">{u.total_turns}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </Card>
          )}

          {/* By model */}
          {data.by_model.length > 0 && (
            <Card title="By Model">
              <table className="w-full text-sm">
                <thead>
                  <tr className="text-muted text-xs">
                    <th className="text-left py-1.5 font-medium">Model</th>
                    <th className="text-right py-1.5 font-medium">Cost</th>
                    <th className="text-right py-1.5 font-medium">Input</th>
                    <th className="text-right py-1.5 font-medium">Output</th>
                    <th className="text-right py-1.5 font-medium">Turns</th>
                  </tr>
                </thead>
                <tbody>
                  {data.by_model.map(m => (
                    <tr key={m.model} className="border-t border-border-subtle">
                      <td className="py-1.5 text-primary font-mono text-xs">{m.model || 'unknown'}</td>
                      <td className="py-1.5 text-right text-primary font-mono text-xs">{formatCost(m.total_cost_usd)}</td>
                      <td className="py-1.5 text-right text-muted font-mono text-xs">{formatTokens(m.total_input_tokens)}</td>
                      <td className="py-1.5 text-right text-muted font-mono text-xs">{formatTokens(m.total_output_tokens)}</td>
                      <td className="py-1.5 text-right text-muted">{m.total_turns}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </Card>
          )}
        </>
      )}
    </>
  )
}
