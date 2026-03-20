import { useState, useRef, useEffect, useCallback } from 'react'
import { createPortal } from 'react-dom'

export default function Tooltip({ text }) {
  const [open, setOpen] = useState(false)
  const [pos, setPos] = useState(null)
  const triggerRef = useRef(null)

  const calculate = useCallback(() => {
    if (!triggerRef.current) return
    const rect = triggerRef.current.getBoundingClientRect()
    const above = rect.top > 140
    setPos({
      above,
      top: above ? rect.top - 8 : rect.bottom + 8,
      left: rect.left + rect.width / 2,
    })
  }, [])

  useEffect(() => {
    if (open) calculate()
  }, [open, calculate])

  return (
    <span className="s-tip-wrap" onMouseEnter={() => setOpen(true)} onMouseLeave={() => setOpen(false)}>
      <span ref={triggerRef} className="s-tip-trigger">?</span>
      {open && pos && createPortal(
        <span
          className="s-tooltip"
          style={{
            position: 'fixed',
            left: pos.left,
            top: pos.above ? undefined : pos.top,
            bottom: pos.above ? `calc(100vh - ${pos.top}px)` : undefined,
            transform: 'translateX(-50%)',
          }}
        >
          {text}
          <span className={`s-tooltip-arrow ${pos.above ? 's-tooltip-arrow-down' : 's-tooltip-arrow-up'}`} />
        </span>,
        document.body
      )}
    </span>
  )
}
