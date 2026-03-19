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
  return '<pre class="bg-bg border border-border-main rounded-lg my-3 overflow-x-auto font-mono text-[13px] leading-relaxed text-secondary">' +
    '<div class="flex items-center justify-between px-3 py-1.5 border-b border-border-subtle text-[11px] text-dim font-mono tracking-wide select-none">' +
      '<span>' + escapedLang + '</span>' +
      '<button class="copy-btn bg-transparent border border-border-main text-dim font-mono text-[11px] px-2 py-0.5 rounded transition-all" onclick="copyCode(this,\'' + codeId + '\')">copy</button>' +
    '</div>' +
    '<div class="px-4 py-3" id="' + codeId + '">' + escaped + '</div>' +
  '</pre>'
}
renderer.link = function({ href, text }) {
  const safeUrl = href.replace(/'/g, "\\'")
  return '<a onclick="event.preventDefault();window._openPreview(\'' + safeUrl + '\')" href="#" class="text-accent-soft underline decoration-accent/30 hover:decoration-accent transition-colors cursor-pointer link-preview">' + text + '</a>'
}
marked.use({ renderer })

// Register global copyCode function
window.copyCode = function(btn, id) {
  const el = document.getElementById(id)
  if (!el) return
  navigator.clipboard.writeText(el.textContent).then(() => {
    btn.textContent = 'copied!'
    btn.classList.add('copied')
    setTimeout(() => { btn.textContent = 'copy'; btn.classList.remove('copied') }, 1500)
  })
}

export function formatText(text) {
  return marked.parse(text)
}

export function formatUserText(text) {
  let html = escapeHtml(text)
  html = html.replace(/(?<![="'])(https?:\/\/[^\s<>"')\]]+)/g, (_, url) => {
    const safeUrl = url.replace(/'/g, "\\'")
    return '<a onclick="event.preventDefault();window._openPreview(\'' + safeUrl + '\')" href="#" class="text-white/80 underline decoration-white/30 hover:decoration-white/60 transition-colors cursor-pointer link-preview">' + url + '</a>'
  })
  return html
}
