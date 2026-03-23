import React from 'react'
import Logo from './ui/Logo'

function Welcome({ onSendPrompt }) {
  const cfg = window.__STARPOD__ || {}
  const greeting = cfg.greeting || 'What can I help with?'
  const prompts = cfg.prompts || []

  return (
    <div
      className="flex items-center justify-center text-center"
      id="welcome"
      style={{ minHeight: 'calc(100dvh - 120px)' }}
    >
      <div className="max-w-lg">
        <div className="mb-4"><Logo large /></div>
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
