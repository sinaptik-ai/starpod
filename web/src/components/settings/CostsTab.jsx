import { useState, useEffect, useCallback, useMemo } from 'react'
import { apiHeaders } from '../../lib/api'
import { Card } from './fields'

const PERIODS = [
  { value: '7d', label: '7 days' },
  { value: '30d', label: '30 days' },
  { value: '90d', label: '90 days' },
  { value: 'all', label: 'All time' },
]

const MODEL_COLORS = [
  '#3b82f6', // blue
  '#8b5cf6', // violet
  '#06b6d4', // cyan
  '#f59e0b', // amber
  '#ef4444', // red
  '#22c55e', // green
  '#ec4899', // pink
  '#64748b', // slate
]

function formatCost(usd) {
  if (usd < 0.01 && usd > 0) return `$${usd.toFixed(3)}`
  return `$${usd.toFixed(2)}`
}

function formatTokens(n) {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`
  return n.toLocaleString()
}

function formatDate(dateStr) {
  const d = new Date(dateStr + 'T00:00:00')
  return d.toLocaleDateString('en-US', { month: 'short', day: 'numeric' })
}

function CostChart({ byDay }) {
  const [hoveredIdx, setHoveredIdx] = useState(null)

  const { models, maxCost, colorMap } = useMemo(() => {
    const modelSet = new Set()
    let max = 0
    for (const day of byDay) {
      for (const m of day.by_model) modelSet.add(m.model)
      if (day.total_cost_usd > max) max = day.total_cost_usd
    }
    const models = [...modelSet]
    const colorMap = {}
    models.forEach((m, i) => { colorMap[m] = MODEL_COLORS[i % MODEL_COLORS.length] })
    // Round up max for nicer grid lines
    if (max === 0) max = 1
    const magnitude = Math.pow(10, Math.floor(Math.log10(max)))
    max = Math.ceil(max / magnitude) * magnitude
    return { models, maxCost: max, colorMap }
  }, [byDay])

  if (byDay.length === 0) {
    return <div className="text-dim text-xs text-center py-6">No usage data for this period</div>
  }

  const W = 600, H = 200
  const padL = 52, padR = 12, padT = 8, padB = 28
  const chartW = W - padL - padR
  const chartH = H - padT - padB
  const barCount = byDay.length
  const gap = Math.max(1, Math.min(4, Math.floor(chartW / barCount * 0.15)))
  const barW = Math.max(2, (chartW - gap * (barCount - 1)) / barCount)

  // Y-axis: 4 ticks
  const ticks = [0, 0.25, 0.5, 0.75, 1].map(f => f * maxCost)

  return (
    <div className="relative">
      <svg
        viewBox={`0 0 ${W} ${H}`}
        className="w-full"
        style={{ maxHeight: 220 }}
        onMouseLeave={() => setHoveredIdx(null)}
      >
        {/* Grid lines */}
        {ticks.map((val, i) => {
          const y = padT + chartH - (val / maxCost) * chartH
          return (
            <g key={i}>
              <line x1={padL} x2={W - padR} y1={y} y2={y}
                stroke="var(--color-border-subtle)" strokeWidth={0.5} />
              <text x={padL - 6} y={y + 3} textAnchor="end"
                fill="var(--color-dim)" fontSize={9} fontFamily="ui-monospace, monospace">
                {formatCost(val)}
              </text>
            </g>
          )
        })}

        {/* Bars */}
        {byDay.map((day, i) => {
          const x = padL + i * (barW + gap)
          let yOffset = 0
          const isHovered = hoveredIdx === i
          return (
            <g key={day.date}
              onMouseEnter={() => setHoveredIdx(i)}
              style={{ cursor: 'default' }}
            >
              {/* Invisible hit area */}
              <rect x={x} y={padT} width={barW} height={chartH}
                fill="transparent" />
              {day.by_model.map((m, mi) => {
                const barH = (m.cost_usd / maxCost) * chartH
                const y = padT + chartH - yOffset - barH
                yOffset += barH
                return (
                  <rect key={mi} x={x} y={y} width={barW} height={Math.max(0.5, barH)}
                    rx={barW > 4 ? 1.5 : 0}
                    fill={colorMap[m.model]}
                    opacity={isHovered ? 1 : 0.75}
                    style={{ transition: 'opacity 0.15s' }}
                  />
                )
              })}
              {/* X label — show every Nth depending on count */}
              {(barCount <= 14 || i % Math.ceil(barCount / 14) === 0) && (
                <text x={x + barW / 2} y={H - 4} textAnchor="middle"
                  fill="var(--color-dim)" fontSize={8} fontFamily="ui-monospace, monospace">
                  {formatDate(day.date)}
                </text>
              )}
            </g>
          )
        })}
      </svg>

      {/* Tooltip */}
      {hoveredIdx !== null && byDay[hoveredIdx] && (
        <div
          className="absolute pointer-events-none bg-elevated border border-border-main rounded-lg px-3 py-2 text-xs shadow-lg"
          style={{
            top: 8,
            left: `${Math.min(75, Math.max(5, (hoveredIdx / byDay.length) * 100))}%`,
            transform: 'translateX(-50%)',
            zIndex: 10,
          }}
        >
          <div className="text-secondary font-medium mb-1">{formatDate(byDay[hoveredIdx].date)}</div>
          {byDay[hoveredIdx].by_model.map((m, i) => (
            <div key={i} className="flex items-center gap-2 text-muted">
              <span className="inline-block w-2 h-2 rounded-sm flex-shrink-0"
                style={{ backgroundColor: colorMap[m.model] }} />
              <span className="truncate" style={{ maxWidth: 120 }}>{m.model || 'unknown'}</span>
              <span className="ml-auto font-mono text-primary">{formatCost(m.cost_usd)}</span>
            </div>
          ))}
          <div className="border-t border-border-subtle mt-1 pt-1 text-primary font-mono font-medium">
            {formatCost(byDay[hoveredIdx].total_cost_usd)}
          </div>
        </div>
      )}

      {/* Legend */}
      {models.length > 1 && (
        <div className="flex flex-wrap gap-x-4 gap-y-1 mt-2 px-1">
          {models.map(m => (
            <div key={m} className="flex items-center gap-1.5 text-xs text-muted">
              <span className="inline-block w-2 h-2 rounded-sm flex-shrink-0"
                style={{ backgroundColor: colorMap[m] }} />
              <span>{m || 'unknown'}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  )
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
            <div className="grid grid-cols-4 gap-4 py-2">
              <div className="text-center">
                <div className="text-2xl font-semibold text-primary">{formatCost(data.total_cost_usd)}</div>
                <div className="text-xs text-muted mt-1">Total cost</div>
              </div>
              <div className="text-center">
                <div className="text-2xl font-semibold text-primary">{formatTokens(data.total_input_tokens + data.total_output_tokens)}</div>
                <div className="text-xs text-muted mt-1">Total tokens</div>
              </div>
              <div className="text-center">
                <div className="text-2xl font-semibold text-primary">{formatTokens((data.total_cache_read || 0) + (data.total_cache_write || 0))}</div>
                <div className="text-xs text-muted mt-1">Cached</div>
              </div>
              <div className="text-center">
                <div className="text-2xl font-semibold text-primary">{data.total_turns.toLocaleString()}</div>
                <div className="text-xs text-muted mt-1">Turns</div>
              </div>
            </div>
          </Card>

          {/* Daily chart */}
          {data.by_day && data.by_day.length > 0 && (
            <Card title="Daily Spend">
              <CostChart byDay={data.by_day} />
            </Card>
          )}

          {/* By user */}
          {data.by_user.length > 0 && (
            <Card title="By User">
              <table className="w-full text-sm">
                <thead>
                  <tr className="text-muted text-xs">
                    <th className="text-left py-1.5 pl-3 font-medium">User</th>
                    <th className="text-right py-1.5 font-medium">Cost</th>
                    <th className="text-right py-1.5 font-medium">Input</th>
                    <th className="text-right py-1.5 font-medium">Cached</th>
                    <th className="text-right py-1.5 font-medium">Output</th>
                    <th className="text-right py-1.5 pr-3 font-medium">Turns</th>
                  </tr>
                </thead>
                <tbody>
                  {data.by_user.map(u => (
                    <tr key={u.user_id} className="border-t border-border-subtle">
                      <td className="py-1.5 pl-3 text-primary">{u.user_id}</td>
                      <td className="py-1.5 text-right text-primary font-mono text-xs">{formatCost(u.total_cost_usd)}</td>
                      <td className="py-1.5 text-right text-muted font-mono text-xs">{formatTokens(u.total_input_tokens)}</td>
                      <td className="py-1.5 text-right text-dim font-mono text-xs">{formatTokens((u.total_cache_read || 0) + (u.total_cache_write || 0))}</td>
                      <td className="py-1.5 text-right text-muted font-mono text-xs">{formatTokens(u.total_output_tokens)}</td>
                      <td className="py-1.5 pr-3 text-right text-muted">{u.total_turns}</td>
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
                    <th className="text-left py-1.5 pl-3 font-medium">Model</th>
                    <th className="text-right py-1.5 font-medium">Cost</th>
                    <th className="text-right py-1.5 font-medium">Input</th>
                    <th className="text-right py-1.5 font-medium">Cached</th>
                    <th className="text-right py-1.5 font-medium">Output</th>
                    <th className="text-right py-1.5 pr-3 font-medium">Turns</th>
                  </tr>
                </thead>
                <tbody>
                  {data.by_model.map(m => (
                    <tr key={m.model} className="border-t border-border-subtle">
                      <td className="py-1.5 pl-3 text-primary font-mono text-xs">{m.model || 'unknown'}</td>
                      <td className="py-1.5 text-right text-primary font-mono text-xs">{formatCost(m.total_cost_usd)}</td>
                      <td className="py-1.5 text-right text-muted font-mono text-xs">{formatTokens(m.total_input_tokens)}</td>
                      <td className="py-1.5 text-right text-dim font-mono text-xs">{formatTokens((m.total_cache_read || 0) + (m.total_cache_write || 0))}</td>
                      <td className="py-1.5 text-right text-muted font-mono text-xs">{formatTokens(m.total_output_tokens)}</td>
                      <td className="py-1.5 pr-3 text-right text-muted">{m.total_turns}</td>
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
