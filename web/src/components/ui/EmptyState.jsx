import React from 'react'

export function Loading({ text = 'Loading...' }) {
  return <div className="text-dim text-sm py-8 text-center">{text}</div>
}

export function Empty({ text }) {
  return <div className="text-dim text-sm text-center py-8">{text}</div>
}
