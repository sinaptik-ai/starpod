import React, { useState } from 'react'
import { toolIconClass, toolIconSymbol, getToolPreview } from '../lib/utils'
import Badge from './ui/Badge'
import SectionLabel from './ui/SectionLabel'

function ToolCard({ id, name, input, status, result }) {
  const [expanded, setExpanded] = useState(false)

  const preview = getToolPreview(name, input || {})
  const inputJson = JSON.stringify(input || {}, null, 2)

  const badgeVariant = status === 'running' ? 'running' : status === 'error' ? 'err' : 'ok'
  const badgeText = status === 'running' ? 'running' : status === 'error' ? 'error' : 'done'

  function toggle() {
    setExpanded(prev => !prev)
  }

  return (
    <div
      className="my-1.5 rounded-lg overflow-hidden border border-border-subtle bg-surface transition-colors hover:border-border-main"
      id={id}
    >
      <button
        type="button"
        className="flex items-center gap-2 px-3 py-2 w-full cursor-pointer select-none text-xs text-secondary transition-colors hover:bg-elevated bg-transparent border-none text-left font-inherit"
        onClick={toggle}
        aria-expanded={expanded}
      >
        <span className={`text-[8px] text-dim transition-transform duration-200 shrink-0 w-3 text-center${expanded ? ' rotate-90' : ''}`}>
          {'\u25B6'}
        </span>
        <span className={`${toolIconClass(name)} text-[10px] w-5 h-5 flex items-center justify-center rounded-md shrink-0 font-semibold`}>
          {toolIconSymbol(name)}
        </span>
        <span className="font-mono font-medium text-xs text-secondary">{name}</span>
        <span className="text-dim font-mono text-[11px] whitespace-nowrap overflow-hidden text-ellipsis flex-1 min-w-0">
          {preview}
        </span>
        <Badge variant={badgeVariant} className="rounded-full px-2 py-0.5 tracking-wide">{badgeText}</Badge>
      </button>

      {expanded && (
        <div className="px-3 pb-3 text-xs text-secondary">
          <div>
            <SectionLabel className="mb-1 tracking-widest font-medium">Input</SectionLabel>
            <pre className="bg-bg border border-border-subtle rounded-md px-3 py-2.5 font-mono text-[11px] leading-normal whitespace-pre-wrap break-all text-dim max-h-60 overflow-y-auto">
              {inputJson}
            </pre>
          </div>
          {result != null && (
            <div className="mt-2">
              <SectionLabel className="mb-1 tracking-widest font-medium">Result</SectionLabel>
              <pre className="bg-bg border border-border-subtle rounded-md px-3 py-2.5 font-mono text-[11px] leading-normal whitespace-pre-wrap break-all text-dim max-h-60 overflow-y-auto">
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
