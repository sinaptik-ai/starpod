import React from 'react'
import { escapeHtml } from '../lib/utils'

function Welcome({ onSendPrompt }) {
  const cfg = window.__STARPOD__ || {}
  const greeting = cfg.greeting || 'ready_'
  const prompts = cfg.prompts || []

  return (
    <div
      className="flex items-center justify-center text-center"
      id="welcome"
      style={{ minHeight: 'calc(100dvh - 120px)' }}
    >
      <div>
        <div className="font-mono text-3xl font-extrabold tracking-tighter mb-3 bg-gradient-to-b from-primary to-muted bg-clip-text text-transparent select-none">
          starpod
        </div>
        <p className="text-sm text-dim font-mono">{greeting}</p>
        {prompts.length > 0 && (
          <div className="mt-6 flex flex-col items-start gap-1.5">
            {prompts.map((p, i) => (
              <button
                key={i}
                className="prompt-chip"
                onClick={() => onSendPrompt(p)}
              >
                <span className="text-dim font-mono mr-2">&gt;</span>
                {p}
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}

export default Welcome
