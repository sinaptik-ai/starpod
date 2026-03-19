import React, { useState } from 'react'
import { escapeHtml, toolIconClass, toolIconSymbol, getToolPreview } from '../lib/utils'

function ToolCard({ id, name, input, status, result }) {
  const [expanded, setExpanded] = useState(false)

  const preview = getToolPreview(name, input || {})
  const inputJson = JSON.stringify(input || {}, null, 2)

  let badgeClass, badgeText
  if (status === 'running') {
    badgeClass = 'bg-accent-muted text-accent-soft badge-running'
    badgeText = 'running'
  } else if (status === 'error') {
    badgeClass = 'bg-err-muted text-err'
    badgeText = 'error'
  } else {
    badgeClass = 'bg-ok-muted text-ok'
    badgeText = 'done'
  }

  function toggle() {
    setExpanded(prev => !prev)
  }

  return (
    <div
      className="my-1.5 rounded-lg overflow-hidden border border-border-subtle bg-surface transition-colors hover:border-border-main"
      id={id}
    >
      <div
        className="flex items-center gap-2 px-3 py-2 cursor-pointer select-none text-xs text-secondary transition-colors hover:bg-elevated"
        onClick={toggle}
      >
        <span className={`text-[8px] text-dim transition-transform duration-200 shrink-0 w-3 text-center${expanded ? ' rotate-90' : ''}`}>
          {'\u25B6'}
        </span>
        <span className={`${toolIconClass(name)} text-[10px] w-5 h-5 flex items-center justify-center rounded-md shrink-0 font-semibold`}>
          {toolIconSymbol(name)}
        </span>
        <span className="font-mono font-medium text-[12px] text-secondary">{name}</span>
        <span className="text-dim font-mono text-[11px] whitespace-nowrap overflow-hidden text-ellipsis flex-1 min-w-0">
          {preview}
        </span>
        <span className={`font-mono text-[10px] px-2 py-0.5 rounded-full font-medium shrink-0 tracking-wide ${badgeClass}`}>
          {badgeText}
        </span>
      </div>

      {expanded && (
        <div className="px-3 pb-3 text-xs text-secondary">
          <div>
            <div className="font-mono text-[10px] uppercase tracking-widest text-dim mb-1 font-medium">Input</div>
            <pre className="bg-bg border border-border-subtle rounded-md px-3 py-2.5 font-mono text-[11.5px] leading-normal whitespace-pre-wrap break-all text-dim max-h-60 overflow-y-auto">
              {inputJson}
            </pre>
          </div>
          {result != null && (
            <div className="mt-2">
              <div className="font-mono text-[10px] uppercase tracking-widest text-dim mb-1 font-medium">Result</div>
              <pre className="bg-bg border border-border-subtle rounded-md px-3 py-2.5 font-mono text-[11.5px] leading-normal whitespace-pre-wrap break-all text-dim max-h-60 overflow-y-auto">
                {typeof result === 'string' ? result : JSON.stringify(result, null, 2)}
              </pre>
            </div>
          )}
        </div>
      )}
    </div>
  )
}

export default ToolCard
