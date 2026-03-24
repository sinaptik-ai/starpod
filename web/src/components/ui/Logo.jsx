import React from 'react'

function StarpodIcon({ className = 'w-5 h-5' }) {
  return (
    <svg className={className} viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
      <rect x="2" y="4" width="20" height="16" rx="2" stroke="currentColor" strokeWidth="2"/>
      <path d="M12 7.5L12.8 9.8L15 10.5L12.8 11.2L12 13.5L11.2 11.2L9 10.5L11.2 9.8L12 7.5Z" fill="currentColor" opacity="0.3"/>
      <path d="M8 10L8.5 8.5L10 8L8.5 7.5L8 6L7.5 7.5L6 8L7.5 8.5L8 10Z" fill="currentColor"/>
      <path d="M16 10L16.5 8.5L18 8L16.5 7.5L16 6L15.5 7.5L14 8L15.5 8.5L16 10Z" fill="currentColor"/>
      <path d="M8 14C8 14 10 16 12 16C14 16 16 14 16 14" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round"/>
    </svg>
  )
}

export { StarpodIcon }

export default function Logo({ large }) {
  return (
    <div className={`flex items-center justify-center select-none ${large ? 'gap-3' : 'gap-2'}`}>
      <StarpodIcon className={large ? 'w-10 h-10 text-primary' : 'w-5 h-5 text-primary'} />
      <span className={`font-display font-bold uppercase tracking-[0.02em] text-primary ${large ? 'text-4xl' : 'text-sm'}`}>
        Starpod
      </span>
    </div>
  )
}
