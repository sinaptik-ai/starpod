import { useState } from 'react'

/**
 * Collapsible step-by-step guide component.
 *
 * Usage:
 *   <StepGuide title="How to get a bot token" steps={[
 *     { text: 'Open Telegram and search for @BotFather' },
 *     { text: <>Send <code>/newbot</code> and follow the prompts</> },
 *   ]} />
 */
export default function StepGuide({ title, steps, note }) {
  const [open, setOpen] = useState(false)

  return (
    <div className="mx-1 mb-2">
      <button
        type="button"
        onClick={() => setOpen(v => !v)}
        className="flex items-center gap-1.5 text-xs text-accent hover:underline cursor-pointer bg-transparent border-0 p-0"
      >
        <svg
          className={`w-3 h-3 stroke-current fill-none stroke-2 transition-transform ${open ? 'rotate-90' : ''}`}
          viewBox="0 0 24 24"
          strokeLinecap="round"
          strokeLinejoin="round"
        >
          <path d="M9 18l6-6-6-6" />
        </svg>
        {title}
      </button>

      {open && (
        <ol className="mt-2 ml-1 space-y-1.5 text-xs text-dim list-none p-0">
          {steps.map((step, i) => (
            <li key={i} className="flex gap-2">
              <span className="shrink-0 w-4 h-4 rounded-full bg-elevated text-muted flex items-center justify-center text-[10px] font-medium mt-px">
                {i + 1}
              </span>
              <span>{step.text || step}</span>
            </li>
          ))}
          {note && (
            <li className="text-muted text-[11px] ml-6 mt-1">{note}</li>
          )}
        </ol>
      )}
    </div>
  )
}
