import { marked } from 'marked'
import { escapeHtml } from './utils'

// Configure marked for GFM (tables, strikethrough, etc.)
marked.setOptions({
  gfm: true,
  breaks: false,
})

// Custom renderer to add copy buttons to code blocks and preview links
const renderer = new marked.Renderer()
renderer.code = function({ text, lang }) {
  const langLabel = lang || 'code'
  const codeId = 'code-' + Math.random().toString(36).slice(2, 8)
  const escaped = escapeHtml(text)
  const escapedLang = escapeHtml(langLabel)
  return '<pre class="bg-bg border border-border-main rounded-none my-3 overflow-x-auto font-mono text-[13px] leading-relaxed text-secondary">' +
    '<div class="flex items-center justify-between px-3 py-1.5 border-b border-border-subtle text-[11px] text-dim font-mono tracking-wide select-none">' +
      '<span>' + escapedLang + '</span>' +
      '<button class="copy-btn bg-transparent border border-border-main text-dim font-mono text-[11px] px-2 py-0.5 rounded-none transition-all" data-code-id="' + codeId + '">copy</button>' +
    '</div>' +
    '<div class="px-4 py-3" id="' + codeId + '">' + escaped + '</div>' +
  '</pre>'
}
renderer.link = function({ href, text }) {
  const safeHref = href.replace(/"/g, '&quot;')
  return '<a href="#" data-preview-url="' + safeHref + '" class="text-accent-soft underline decoration-accent/30 hover:decoration-accent transition-colors cursor-pointer link-preview">' + text + '</a>'
}
renderer.table = function(token) {
  let html = '<thead><tr>'
  for (let i = 0; i < token.header.length; i++) {
    const cell = token.header[i]
    const align = token.align[i]
    const style = align ? ' style="text-align:' + align + '"' : ''
    html += '<th' + style + '>' + this.parser.parseInline(cell.tokens) + '</th>'
  }
  html += '</tr></thead>'
  if (token.rows.length > 0) {
    html += '<tbody>'
    for (const row of token.rows) {
      html += '<tr>'
      for (let i = 0; i < row.length; i++) {
        const cell = row[i]
        const align = token.align[i]
        const style = align ? ' style="text-align:' + align + '"' : ''
        html += '<td' + style + '>' + this.parser.parseInline(cell.tokens) + '</td>'
      }
      html += '</tr>'
    }
    html += '</tbody>'
  }
  return '<div class="table-wrap"><table>' + html + '</table></div>'
}
marked.use({ renderer })

// Delegated click listener for copy buttons and preview links
document.addEventListener('click', (e) => {
  // Copy button
  const copyBtn = e.target.closest('.copy-btn')
  if (copyBtn) {
    const codeId = copyBtn.dataset.codeId
    const el = codeId && document.getElementById(codeId)
    if (el) {
      navigator.clipboard.writeText(el.textContent).then(() => {
        copyBtn.textContent = 'copied!'
        copyBtn.classList.add('copied')
        setTimeout(() => { copyBtn.textContent = 'copy'; copyBtn.classList.remove('copied') }, 1500)
      })
    }
    return
  }
  // Preview links
  const link = e.target.closest('.link-preview')
  if (link) {
    e.preventDefault()
    const url = link.dataset.previewUrl
    if (url && window._openPreview) window._openPreview(url)
  }
})

export function formatText(text) {
  return marked.parse(text)
}

export function formatUserText(text) {
  let html = escapeHtml(text)
  html = html.replace(/(?<![="'])(https?:\/\/[^\s<>"')\]]+)/g, (_, url) => {
    const safeHref = url.replace(/"/g, '&quot;')
    return '<a href="#" data-preview-url="' + safeHref + '" class="text-white/80 underline decoration-white/30 hover:decoration-white/60 transition-colors cursor-pointer link-preview">' + url + '</a>'
  })
  return html
}
