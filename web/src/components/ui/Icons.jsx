import React from 'react'

// Shared SVG icon props
const s = (extra = '') => `w-4 h-4 stroke-current fill-none stroke-2${extra ? ' ' + extra : ''}`
const s35 = (extra = '') => `w-3.5 h-3.5 stroke-current fill-none stroke-[1.5]${extra ? ' ' + extra : ''}`

export function MenuIcon({ className }) {
  return (
    <svg className={className || s()} viewBox="0 0 24 24" strokeLinecap="round">
      <path d="M3 12h18M3 6h18M3 18h18" />
    </svg>
  )
}

export function SidebarOpenIcon({ className }) {
  return (
    <svg className={className || s()} viewBox="0 0 24 24" strokeLinecap="round">
      <rect x="3" y="3" width="18" height="18" rx="2" />
      <line x1="9" y1="3" x2="9" y2="21" />
    </svg>
  )
}

export function ComposeIcon({ className }) {
  return (
    <svg className={className || s35()} viewBox="0 0 24 24" strokeLinecap="round" strokeLinejoin="round">
      <path d="M12 20h9" /><path d="M16.5 3.5a2.121 2.121 0 013 3L7 19l-4 1 1-4L16.5 3.5z" />
    </svg>
  )
}

export function CloseIcon({ className }) {
  return (
    <svg className={className || s35()} viewBox="0 0 24 24" strokeLinecap="round">
      <line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" />
    </svg>
  )
}

export function BackIcon({ className }) {
  return (
    <svg className={className || s()} viewBox="0 0 24 24" strokeLinecap="round">
      <path d="M19 12H5M12 19l-7-7 7-7" />
    </svg>
  )
}

export function GearIcon({ className }) {
  return (
    <svg className={className || s35()} viewBox="0 0 24 24" strokeLinecap="round" strokeLinejoin="round">
      <circle cx="12" cy="12" r="3" />
      <path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 010 2.83 2 2 0 01-2.83 0l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-2 2 2 2 0 01-2-2v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83 0 2 2 0 010-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 01-2-2 2 2 0 012-2h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 010-2.83 2 2 0 012.83 0l.06.06A1.65 1.65 0 009 4.68a1.65 1.65 0 001-1.51V3a2 2 0 012-2 2 2 0 012 2v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 0 2 2 0 010 2.83l-.06.06A1.65 1.65 0 0019.4 9a1.65 1.65 0 001.51 1H21a2 2 0 012 2 2 2 0 01-2 2h-.09a1.65 1.65 0 00-1.51 1z" />
    </svg>
  )
}

export function PaperclipIcon({ className }) {
  return (
    <svg className={className || s()} viewBox="0 0 24 24" strokeLinecap="round">
      <path d="M21.44 11.05l-9.19 9.19a6 6 0 01-8.49-8.49l9.19-9.19a4 4 0 015.66 5.66l-9.2 9.19a2 2 0 01-2.83-2.83l8.49-8.48" />
    </svg>
  )
}

export function SendIcon({ className }) {
  return (
    <svg className={className || 'w-4 h-4 fill-current'} viewBox="0 0 24 24">
      <path d="M2.01 21L23 12 2.01 3 2 10l15 2-15 2z" />
    </svg>
  )
}

export function ChevronDownIcon({ className }) {
  return (
    <svg className={className || 'w-3 h-3'} fill="none" stroke="currentColor" viewBox="0 0 24 24">
      <path strokeLinecap="round" strokeWidth="2" d="M19 9l-7 7-7-7" />
    </svg>
  )
}
