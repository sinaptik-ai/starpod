import React from 'react'
import Logo from './ui/Logo'

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
        <div className="mb-3"><Logo large /></div>
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
