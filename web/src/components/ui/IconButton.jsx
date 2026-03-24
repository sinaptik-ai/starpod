import React from 'react'

export default function IconButton({ onClick, children, id, title, className, ...props }) {
  return (
    <button
      onClick={onClick}
      className={className || 'text-muted hover:text-primary p-1.5 rounded-none hover:bg-elevated transition-colors cursor-pointer'}
      id={id}
      title={title}
      {...props}
    >
      {children}
    </button>
  )
}
