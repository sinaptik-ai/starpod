// Fallback for crypto.randomUUID (unavailable over plain HTTP)
export function generateUUID() {
  if (typeof crypto !== 'undefined' && crypto.randomUUID) {
    return crypto.randomUUID()
  }
  return '10000000-1000-4000-8000-100000000000'.replace(/[018]/g, c =>
    (+c ^ crypto.getRandomValues(new Uint8Array(1))[0] & 15 >> +c / 4).toString(16)
  )
}

export function escapeHtml(text) {
  const div = document.createElement('div')
  div.textContent = text
  return div.innerHTML
}

export function formatSessionDate(isoStr) {
  const d = new Date(isoStr)
  const now = new Date()
  const diff = now - d
  const mins = Math.floor(diff / 60000)
  if (mins < 1) return 'just now'
  if (mins < 60) return mins + 'm ago'
  const hrs = Math.floor(mins / 60)
  if (hrs < 24) return hrs + 'h ago'
  const days = Math.floor(hrs / 24)
  if (days < 7) return days + 'd ago'
  return d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' })
}

export function toolIconClass(name) {
  const n = name.toLowerCase()
  if (n === 'read') return 'tool-icon-read'
  if (n === 'write') return 'tool-icon-write'
  if (n === 'edit') return 'tool-icon-edit'
  if (n === 'bash') return 'tool-icon-bash'
  if (n === 'grep') return 'tool-icon-grep'
  if (n === 'glob') return 'tool-icon-glob'
  if (n.includes('search')) return 'tool-icon-search'
  return 'tool-icon-default'
}

export function toolIconSymbol(name) {
  const n = name.toLowerCase()
  if (n === 'read') return '\u25B7'
  if (n === 'write' || n === 'edit') return '\u270E'
  if (n === 'bash') return '$'
  if (n === 'grep' || n === 'glob') return '\u2315'
  if (n.includes('memory')) return '\u25C7'
  if (n.includes('vault')) return '\u2609'
  if (n.includes('skill')) return '\u2726'
  if (n.includes('cron')) return '\u23F0'
  return '\u2022'
}

export function getToolPreview(name, input) {
  if (input.file_path) return input.file_path
  if (input.pattern) return input.pattern
  if (input.description) return input.description.length > 80 ? input.description.slice(0, 80) + '\u2026' : input.description
  if (input.command) return input.command.length > 60 ? input.command.slice(0, 60) + '\u2026' : input.command
  if (input.query) return input.query.length > 60 ? input.query.slice(0, 60) + '\u2026' : input.query
  if (input.key) return input.key
  if (input.file) return input.file
  if (input.name) return input.name
  return ''
}
