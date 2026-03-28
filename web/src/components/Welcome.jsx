import React from 'react'
import { StarpodIcon } from './ui/Logo'

function Welcome({ onSendPrompt }) {
  const cfg = window.__STARPOD__ || {}
  const agentName = cfg.agent_name || 'Starpod'
  const greeting = cfg.greeting || 'What can I help with?'
  const prompts = cfg.prompts || []

  return (
    <div
      className="flex items-center justify-center text-center"
      id="welcome"
      style={{ minHeight: 'calc(100dvh - 120px)' }}
    >
      <div className="max-w-lg">
        <div className="mb-4 flex items-center justify-center gap-3">
          <StarpodIcon className="w-10 h-10 text-primary" />
          <span className="text-4xl font-display font-bold uppercase tracking-[0.02em] text-primary">{agentName}</span>
        </div>
        <p className="text-xl text-secondary font-light tracking-tight">{greeting}</p>
        {prompts.length > 0 && (
          <div className="mt-8 flex flex-wrap justify-center gap-2">
            {prompts.map((p, i) => (
              <button
                key={i}
                className="prompt-chip"
                onClick={() => onSendPrompt(p)}
              >
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
