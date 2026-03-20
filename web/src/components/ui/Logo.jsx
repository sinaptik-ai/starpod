import React from 'react'

export default function Logo({ large }) {
  return (
    <div className={`font-mono font-extrabold tracking-tighter bg-gradient-to-b from-primary to-muted bg-clip-text text-transparent select-none ${large ? 'text-5xl' : 'text-sm'}`}>
      starpod
    </div>
  )
}
