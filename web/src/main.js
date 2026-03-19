import './style.css'
import { marked } from 'marked'

// Fallback for crypto.randomUUID (unavailable over plain HTTP)
function generateUUID() {
  if (typeof crypto !== 'undefined' && crypto.randomUUID) {
    return crypto.randomUUID()
  }
  return '10000000-1000-4000-8000-100000000000'.replace(/[018]/g, c =>
    (+c ^ crypto.getRandomValues(new Uint8Array(1))[0] & 15 >> +c / 4).toString(16)
  )
}

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
  return '<pre class="bg-bg border border-border-main rounded-lg my-3 overflow-x-auto font-mono text-[13px] leading-relaxed text-secondary">' +
    '<div class="flex items-center justify-between px-3 py-1.5 border-b border-border-subtle text-[11px] text-dim font-mono tracking-wide select-none">' +
      '<span>' + escapeHtmlForRenderer(langLabel) + '</span>' +
      '<button class="copy-btn bg-transparent border border-border-main text-dim font-mono text-[11px] px-2 py-0.5 rounded transition-all" onclick="copyCode(this, \'' + codeId + '\')">copy</button>' +
    '</div>' +
    '<div class="px-4 py-3" id="' + codeId + '">' + escapeHtmlForRenderer(text) + '</div></pre>'
}
renderer.link = function({ href, text }) {
  const safeUrl = href.replace(/'/g, "\\'")
  return '<a onclick="event.preventDefault();window._openPreview(\'' + safeUrl + '\')" href="#" class="text-accent-soft underline decoration-accent/30 hover:decoration-accent transition-colors cursor-pointer link-preview">' + text + '</a>'
}
marked.use({ renderer })

function escapeHtmlForRenderer(text) {
  const div = document.createElement('div')
  div.textContent = text
  return div.innerHTML
}

// ── DOM refs ──
const messages = document.getElementById('messages')
const inputText = document.getElementById('input-text')
const sendBtn = document.getElementById('send-btn')
const inputForm = document.getElementById('input-form')
const statusDot = document.getElementById('status-dot')
const statusText = document.getElementById('status-text')
const sidebar = document.getElementById('sidebar')
const sessionList = document.getElementById('session-list')
const menuBtn = document.getElementById('menu-btn')
const sidebarClose = document.getElementById('sidebar-close')
const sidebarOverlay = document.getElementById('sidebar-overlay')
const newChatBtn = document.getElementById('new-chat-btn')
const newChatHeaderBtn = document.getElementById('new-chat-header-btn')
const attachBtn = document.getElementById('attach-btn')
const fileInput = document.getElementById('file-input')
const attachmentPreview = document.getElementById('attachment-preview')
const previewPanel = document.getElementById('preview-panel')
const previewIframe = document.getElementById('preview-iframe')
const previewUrl = document.getElementById('preview-url')
const previewOpen = document.getElementById('preview-open')
const previewClose = document.getElementById('preview-close')
const previewFallback = document.getElementById('preview-fallback')
const previewFallbackLink = document.getElementById('preview-fallback-link')
const previewOgImage = document.getElementById('preview-og-image')
const previewOgImg = document.getElementById('preview-og-img')
const previewOgTitle = document.getElementById('preview-og-title')

// ── State ──
let ws = null
let isStreaming = false
let currentMsg = null
let currentBubble = null
let reconnectAttempt = 0
let toolCounter = 0
let currentSessionId = null
let currentSessionKey = generateUUID()
let pendingAttachments = []
let cachedSessions = []

const MAX_FILE_SIZE = 20 * 1024 * 1024

// ── Unread tracking (localStorage) ──
// Sessions are "unread" until the user clicks on them or sends a message.
// Stored client-side as a Set of session IDs that have been seen.
// Stale entries are pruned when the session list is refreshed from the server.
const UNREAD_KEY = 'starpod_read_sessions'
let readSessions = new Set(JSON.parse(localStorage.getItem(UNREAD_KEY) || '[]'))

function saveReadSessions() {
  localStorage.setItem(UNREAD_KEY, JSON.stringify([...readSessions]))
}

function markRead(sessionId) {
  if (!readSessions.has(sessionId)) {
    readSessions.add(sessionId)
    saveReadSessions()
  }
  // Update DOM — remove blue dot
  const el = sessionList.querySelector(`.session-item[data-sid="${sessionId}"]`)
  if (el) {
    const dot = el.querySelector('.unread-dot')
    if (dot) dot.remove()
  }
}

function isUnread(sessionId) {
  return !readSessions.has(sessionId)
}

// ── Toast notifications ──
// Shown when a cron/heartbeat job completes while the web UI is open.
// Clicking a toast navigates to the cron job's session transcript.
// Toasts auto-dismiss after 6 seconds with a slide-out animation.
function showToast(jobName, preview, success, sessionId) {
  const container = document.getElementById('toast-container') || createToastContainer()
  const toast = document.createElement('div')
  toast.className = 'toast ' + (success ? 'toast-success' : 'toast-error')
  toast.innerHTML =
    '<div class="toast-icon">' + (success ? '\u2713' : '\u2717') + '</div>' +
    '<div class="toast-content">' +
      '<div class="toast-title">' + escapeHtml(jobName) + '</div>' +
      '<div class="toast-body">' + escapeHtml(preview.length > 120 ? preview.slice(0, 120) + '\u2026' : preview) + '</div>' +
    '</div>'
  if (sessionId) {
    toast.style.cursor = 'pointer'
    toast.addEventListener('click', () => {
      toast.remove()
      // Find the session in cached list and navigate to it
      const session = cachedSessions.find(s => s.id === sessionId)
      if (session) {
        selectSession(session)
      } else {
        // Fetch fresh sessions and navigate
        fetchSessions(() => {
          const s = cachedSessions.find(s => s.id === sessionId)
          if (s) selectSession(s)
        })
      }
    })
  }
  container.appendChild(toast)
  // Auto-dismiss after 6s
  setTimeout(() => {
    toast.style.animation = 'toast-out 0.3s ease forwards'
    setTimeout(() => toast.remove(), 300)
  }, 6000)
}

function createToastContainer() {
  const c = document.createElement('div')
  c.id = 'toast-container'
  document.body.appendChild(c)
  return c
}

// ── Helpers ──
function setStatus(state) {
  statusDot.className = 'w-1.5 h-1.5 rounded-full shrink-0 dot-' + state
  const labels = { connected: 'connected', connecting: 'connecting', disconnected: 'disconnected' }
  statusText.textContent = labels[state] || state
}

function scrollToBottom() {
  const scroller = document.getElementById('messages-scroll')
  requestAnimationFrame(() => { scroller.scrollTop = scroller.scrollHeight })
}

function autoResize() {
  inputText.style.height = 'auto'
  inputText.style.height = Math.min(inputText.scrollHeight, 160) + 'px'
}

function escapeHtml(text) {
  const div = document.createElement('div')
  div.textContent = text
  return div.innerHTML
}

function formatText(text) {
  return marked.parse(text)
}

function formatUserText(text) {
  let html = escapeHtml(text)
  html = html.replace(/(?<![="'])(https?:\/\/[^\s<>"')\]]+)/g, (_, url) => {
    const safeUrl = url.replace(/'/g, "\\'")
    return '<a onclick="event.preventDefault();window._openPreview(\'' + safeUrl + '\')" href="#" class="text-white/80 underline decoration-white/30 hover:decoration-white/60 transition-colors cursor-pointer link-preview">' + url + '</a>'
  })
  return html
}

// ── Link preview panel ──
let previewLoadTimer = null
let sidebarWasOpen = false

window._openPreview = openPreview
function openPreview(url) {
  previewUrl.textContent = url
  previewOpen.href = url
  previewFallbackLink.href = url
  previewFallback.classList.add('hidden')
  previewFallback.classList.remove('flex')
  previewIframe.classList.remove('hidden')
  previewIframe._loaded = false
  previewIframe.src = 'about:blank'
  previewPanel.classList.add('open')

  // Collapse sidebar into transient mode
  sidebarWasOpen = sidebar.classList.contains('open')
  sidebar.classList.remove('open')
  sidebar.classList.add('transient')

  // Reset og data
  previewOgImage.classList.add('hidden')
  previewOgImg.src = ''
  previewOgTitle.textContent = 'Connection refused'

  // Server-side pre-check: reliably detects X-Frame-Options / CSP blocking
  fetch('/api/frame-check?url=' + encodeURIComponent(url))
    .then(r => r.json())
    .then(data => {
      if (data.frameable) {
        previewIframe.src = url
      } else {
        if (data.ogImage) {
          previewOgImg.src = data.ogImage
          previewOgImage.classList.remove('hidden')
        }
        if (data.ogTitle) previewOgTitle.textContent = data.ogTitle
        else try { previewOgTitle.textContent = new URL(url).hostname } catch {}
        showPreviewFallback()
      }
    })
    .catch(() => {
      // Endpoint unavailable — load iframe and hope for the best
      previewIframe.src = url
    })

  // Safety timeout in case iframe hangs
  clearTimeout(previewLoadTimer)
  previewLoadTimer = setTimeout(() => {
    if (!previewIframe._loaded) showPreviewFallback()
  }, 6000)
}

function closePreview() {
  previewPanel.classList.remove('open')
  clearTimeout(previewLoadTimer)
  setTimeout(() => { previewIframe.src = 'about:blank' }, 300)

  // Restore sidebar state
  sidebar.classList.remove('transient')
  if (sidebarWasOpen) sidebar.classList.add('open')
}

previewClose.addEventListener('click', closePreview)

previewIframe.addEventListener('load', () => {
  if (previewIframe.src === 'about:blank') return
  previewIframe._loaded = true
  clearTimeout(previewLoadTimer)
})

previewIframe.addEventListener('error', showPreviewFallback)

function showPreviewFallback() {
  previewIframe.classList.add('hidden')
  previewFallback.classList.remove('hidden')
  previewFallback.classList.add('flex')
}


// Close preview with Escape
document.addEventListener('keydown', (e) => {
  if (e.key === 'Escape' && previewPanel.classList.contains('open')) {
    e.preventDefault()
    closePreview()
  }
})

// ── Copy code ──
window.copyCode = function(btn, id) {
  const el = document.getElementById(id)
  if (!el) return
  navigator.clipboard.writeText(el.textContent).then(() => {
    btn.textContent = 'copied!'
    btn.classList.add('copied')
    setTimeout(() => { btn.textContent = 'copy'; btn.classList.remove('copied') }, 1500)
  })
}

// ── Tool helpers ──
function toolIconClass(name) {
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

function toolIconSymbol(name) {
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

function getToolPreview(name, input) {
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

window.toggleTool = function(id) {
  const el = document.getElementById(id)
  if (el) {
    el.classList.toggle('expanded')
    const body = el.querySelector('.tool-body')
    if (body) body.classList.toggle('hidden')
    const chevron = el.querySelector('.tool-chevron')
    if (chevron) chevron.classList.toggle('rotate-90')
  }
}

// ── Messages ──
function addUserMessage(text, atts) {
  const welcome = messages.querySelector('#welcome')
  if (welcome) welcome.remove()

  const msg = document.createElement('div')
  msg.className = 'max-w-[80%] self-end mt-4'
  msg.style.animation = 'msg-in 0.25s cubic-bezier(0.16, 1, 0.3, 1)'

  let html = ''
  if (atts && atts.length > 0) {
    html += '<div class="flex flex-wrap gap-1.5 mb-1.5 justify-end">'
    for (const att of atts) {
      if (att.mime_type.startsWith('image/')) {
        html += '<img src="data:' + att.mime_type + ';base64,' + att.data + '" class="max-w-[200px] max-h-[200px] rounded-xl object-cover" alt="' + escapeHtml(att.file_name) + '">'
      } else {
        html += '<div class="bg-elevated px-2.5 py-1.5 rounded-lg font-mono text-[11px] text-muted border border-border-subtle">' + escapeHtml(att.file_name) + '</div>'
      }
    }
    html += '</div>'
  }
  if (text) html += '<div class="bg-accent text-white rounded-2xl rounded-br-md px-4 py-2.5 leading-relaxed text-sm whitespace-pre-wrap break-words">' + formatUserText(text) + '</div>'
  msg.innerHTML = html
  messages.appendChild(msg)
  scrollToBottom()
}

function startAssistantMessage() {
  const msg = document.createElement('div')
  msg.className = 'max-w-full mt-2'
  msg.style.animation = 'msg-in 0.25s cubic-bezier(0.16, 1, 0.3, 1)'
  currentMsg = msg
  currentBubble = null
  messages.appendChild(msg)

  // Show an empty bubble with the streaming cursor immediately
  ensureBubble()
  scrollToBottom()
}

function showThinking() {
  if (!currentMsg) return
  // End current bubble and start a fresh one with the streaming cursor
  if (currentBubble) {
    currentBubble.classList.remove('streaming-cursor')
    currentBubble = null
  }
  ensureBubble()
  scrollToBottom()
}

function ensureBubble() {
  if (currentBubble) return currentBubble
  if (!currentMsg) return null

  const bubble = document.createElement('div')
  bubble.className = 'py-1 leading-[1.75] text-sm break-words text-secondary streaming-cursor markdown-body'
  currentMsg.appendChild(bubble)
  currentBubble = bubble
  return bubble
}

function appendText(text) {
  const bubble = ensureBubble()
  if (!bubble) return
  if (!bubble._rawText) bubble._rawText = ''
  bubble._rawText += text
  bubble.innerHTML = formatText(bubble._rawText)
  bubble.classList.add('streaming-cursor')
  scrollToBottom()
}

function addToolUse(name, input, toolUseId) {
  if (!currentMsg) return

  if (currentBubble) {
    currentBubble.classList.remove('streaming-cursor')
    currentBubble = null
  }

  const id = toolUseId ? 'tool-' + toolUseId : 'tool-' + (toolCounter++)
  const preview = getToolPreview(name, input)
  const inputJson = JSON.stringify(input, null, 2)

  const el = document.createElement('div')
  el.className = 'my-1.5 rounded-lg overflow-hidden border border-border-subtle bg-surface transition-colors hover:border-border-main'
  el.id = id
  el.innerHTML =
    '<div class="flex items-center gap-2 px-3 py-2 cursor-pointer select-none text-xs text-secondary transition-colors hover:bg-elevated" onclick="toggleTool(\'' + id + '\')">' +
      '<span class="tool-chevron text-[8px] text-dim transition-transform duration-200 shrink-0 w-3 text-center">\u25B6</span>' +
      '<span class="' + toolIconClass(name) + ' text-[10px] w-5 h-5 flex items-center justify-center rounded-md shrink-0 font-semibold">' + toolIconSymbol(name) + '</span>' +
      '<span class="font-mono font-medium text-[12px] text-secondary">' + escapeHtml(name) + '</span>' +
      '<span class="text-dim font-mono text-[11px] whitespace-nowrap overflow-hidden text-ellipsis flex-1 min-w-0">' + escapeHtml(preview) + '</span>' +
      '<span class="font-mono text-[10px] px-2 py-0.5 rounded-full font-medium shrink-0 tracking-wide bg-accent-muted text-accent-soft badge-running">running</span>' +
    '</div>' +
    '<div class="tool-body hidden px-3 pb-3 text-xs text-secondary">' +
      '<div>' +
        '<div class="font-mono text-[10px] uppercase tracking-widest text-dim mb-1 font-medium">Input</div>' +
        '<pre class="bg-bg border border-border-subtle rounded-md px-3 py-2.5 font-mono text-[11.5px] leading-normal whitespace-pre-wrap break-all text-dim max-h-60 overflow-y-auto">' + escapeHtml(inputJson) + '</pre>' +
      '</div>' +
      '<div class="tool-result-section mt-2 hidden">' +
        '<div class="font-mono text-[10px] uppercase tracking-widest text-dim mb-1 font-medium">Result</div>' +
        '<pre class="tool-result-pre bg-bg border border-border-subtle rounded-md px-3 py-2.5 font-mono text-[11.5px] leading-normal whitespace-pre-wrap break-all text-dim max-h-60 overflow-y-auto"></pre>' +
      '</div>' +
    '</div>'

  currentMsg.appendChild(el)
  scrollToBottom()
}

function addToolResult(content, isError, toolUseId) {
  if (!currentMsg) return

  let target
  if (toolUseId) {
    target = document.getElementById('tool-' + toolUseId)
  }
  if (!target) {
    // Fallback: find the first tool still showing "running"
    const running = currentMsg.querySelector('.badge-running')
    target = running ? running.closest('[id^="tool-"]') : null
  }
  if (!target) return

  const badge = target.querySelector('.badge-running, [class*="badge"]')
  if (!badge) return

  if (isError) {
    badge.textContent = 'error'
    badge.className = 'font-mono text-[10px] px-2 py-0.5 rounded-full font-medium shrink-0 tracking-wide bg-err-muted text-err'
  } else {
    badge.textContent = 'done'
    badge.className = 'font-mono text-[10px] px-2 py-0.5 rounded-full font-medium shrink-0 tracking-wide bg-ok-muted text-ok'
  }

  const resultSection = target.querySelector('.tool-result-section')
  const resultPre = target.querySelector('.tool-result-pre')
  if (resultSection && resultPre && content) {
    resultPre.textContent = content
    resultSection.classList.remove('hidden')
  }
  scrollToBottom()
}

function endStream(data) {
  if (currentMsg) {
  
    currentMsg.querySelectorAll('.streaming-cursor').forEach(b => {
      b.classList.remove('streaming-cursor')
      if (b._rawText) {
        b.innerHTML = formatText(b._rawText)
      } else if (!b.textContent.trim()) {
        b.remove()
      }
    })

    if (data.is_error && data.errors && data.errors.length > 0) {
      const hasText = Array.from(currentMsg.querySelectorAll('.markdown-body')).some(b => b._rawText)
      if (!hasText) {
        const bubble = ensureBubble()
        if (bubble) {
          bubble.innerHTML = '<span class="text-err">' + data.errors.map(escapeHtml).join('<br>') + '</span>'
          bubble.classList.remove('streaming-cursor')
        }
      }
    }

    if (data.num_turns > 0) {
      const stats = document.createElement('div')
      stats.className = 'font-mono text-[11px] text-dim mt-2 pt-2 border-t border-border-subtle flex gap-3 flex-wrap'
      const tokens_in = data.input_tokens >= 1000 ? Math.round(data.input_tokens / 1000) + 'k' : data.input_tokens
      const tokens_out = data.output_tokens >= 1000 ? Math.round(data.output_tokens / 1000) + 'k' : data.output_tokens
      stats.innerHTML =
        '<span>' + data.num_turns + ' turn' + (data.num_turns > 1 ? 's' : '') + '</span>' +
        '<span>$' + data.cost_usd.toFixed(4) + '</span>' +
        '<span>' + tokens_in + ' in \u00b7 ' + tokens_out + ' out</span>'
      currentMsg.appendChild(stats)
    }
  }

  isStreaming = false
  currentMsg = null
  currentBubble = null
  inputText.focus()
  scrollToBottom()
}

// ── WebSocket ──
function connect() {
  setStatus('connecting')
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:'
  const token = localStorage.getItem('starpod_api_key')
  let url = proto + '//' + location.host + '/ws'
  if (token) url += '?token=' + encodeURIComponent(token)

  ws = new WebSocket(url)

  ws.onopen = () => { setStatus('connected'); reconnectAttempt = 0 }

  ws.onclose = () => {
    setStatus('disconnected')
    if (isStreaming) {
      endStream({ num_turns: 0, cost_usd: 0, input_tokens: 0, output_tokens: 0, is_error: true, errors: ['Connection lost'] })
    }
    const delay = Math.min(1000 * Math.pow(2, reconnectAttempt), 30000)
    reconnectAttempt++
    setTimeout(connect, delay)
  }

  ws.onerror = () => {}

  ws.onmessage = (event) => {
    let data
    try { data = JSON.parse(event.data) } catch { return }

    switch (data.type) {
      case 'stream_start':
        if (data.session_id) {
          currentSessionId = data.session_id
          markRead(data.session_id)
        }
        startAssistantMessage()
        break
      case 'text_delta':
        appendText(data.text)
        break
      case 'tool_use':
        addToolUse(data.name, data.input, data.id)
        break
      case 'tool_result':
        addToolResult(data.content, data.is_error, data.tool_use_id)
        showThinking()
        break
      case 'stream_end':
        endStream(data)
        // Mark current session as read + refresh sidebar
        if (currentSessionId) markRead(currentSessionId)
        fetchSessions()
        break
      case 'error':
        if (isStreaming) {
          endStream({ num_turns: 0, cost_usd: 0, input_tokens: 0, output_tokens: 0, is_error: true, errors: [data.message] })
        } else {
          startAssistantMessage()
          const bubble = ensureBubble()
          if (bubble) {
            bubble._rawText = 'Error: ' + data.message
            bubble.innerHTML = '<span class="text-err">' + escapeHtml(data.message) + '</span>'
            bubble.classList.remove('streaming-cursor')
          }
          currentMsg = null
          currentBubble = null
        }
        break
      case 'notification':
        // Cron/heartbeat job completed — show toast + refresh session list
        showToast(data.job_name, data.result_preview, data.success, data.session_id)
        // Mark the new session as unread (unless it's the active one)
        if (data.session_id && data.session_id !== currentSessionId) {
          readSessions.delete(data.session_id)
          saveReadSessions()
        }
        // Refresh session list to show the new cron session
        fetchSessions()
        break
    }
  }
}

// ── Attachments ──
function readFileAsBase64(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => { resolve(reader.result.split(',')[1]) }
    reader.onerror = reject
    reader.readAsDataURL(file)
  })
}

async function addFiles(files) {
  for (const file of files) {
    if (file.size > MAX_FILE_SIZE) {
      alert(`File "${file.name}" exceeds 20 MB limit (${(file.size / 1048576).toFixed(1)} MB)`)
      continue
    }
    const base64 = await readFileAsBase64(file)
    pendingAttachments.push({
      file_name: file.name,
      mime_type: file.type || 'application/octet-stream',
      data: base64,
    })
  }
  renderAttachmentPreview()
}

function removeAttachment(index) {
  pendingAttachments.splice(index, 1)
  renderAttachmentPreview()
}
window.removeAttachment = removeAttachment

function renderAttachmentPreview() {
  if (pendingAttachments.length === 0) {
    attachmentPreview.innerHTML = ''
    attachmentPreview.classList.add('hidden')
    attachmentPreview.classList.remove('flex')
    return
  }
  attachmentPreview.classList.remove('hidden')
  attachmentPreview.classList.add('flex')
  attachmentPreview.innerHTML = pendingAttachments.map((att, i) => {
    const isImage = att.mime_type.startsWith('image/')
    const thumb = isImage
      ? '<img src="data:' + att.mime_type + ';base64,' + att.data + '" class="w-7 h-7 object-cover rounded shrink-0">'
      : '<span class="text-sm shrink-0 text-dim">\u{1F4CE}</span>'
    return '<div class="flex items-center gap-1.5 bg-elevated border border-border-main rounded-lg px-2 py-1 font-mono text-[11px] text-secondary max-w-[200px] transition-colors hover:border-dim">' +
      thumb +
      '<span class="overflow-hidden text-ellipsis whitespace-nowrap flex-1 min-w-0">' + escapeHtml(att.file_name) + '</span>' +
      '<button class="bg-transparent border-none text-dim cursor-pointer text-sm leading-none px-0.5 shrink-0 hover:text-err transition-colors" onclick="removeAttachment(' + i + ')">&times;</button>' +
    '</div>'
  }).join('')
}

attachBtn.addEventListener('click', () => fileInput.click())
fileInput.addEventListener('change', () => {
  if (fileInput.files.length > 0) addFiles(fileInput.files)
  fileInput.value = ''
})

// ── Drag & drop ──
const app = document.getElementById('app')
let dragCounter = 0

app.addEventListener('dragenter', (e) => { e.preventDefault(); dragCounter++; app.classList.add('drag-over') })
app.addEventListener('dragover', (e) => { e.preventDefault() })
app.addEventListener('dragleave', (e) => { e.preventDefault(); dragCounter--; if (dragCounter <= 0) { dragCounter = 0; app.classList.remove('drag-over') } })
app.addEventListener('drop', (e) => { e.preventDefault(); dragCounter = 0; app.classList.remove('drag-over'); if (e.dataTransfer.files.length > 0) addFiles(e.dataTransfer.files) })

// ── Send ──
function sendMessage() {
  const text = inputText.value.trim()
  if ((!text && pendingAttachments.length === 0) || !ws || ws.readyState !== WebSocket.OPEN) return

  addUserMessage(text, pendingAttachments)

  // If there's an active assistant message (streaming), move it after the new user message
  if (currentMsg && currentMsg.parentNode === messages) {
    messages.appendChild(currentMsg)
  }

  isStreaming = true

  const payload = { type: 'message', text, channel_id: 'web', channel_session_key: currentSessionKey }
  if (pendingAttachments.length > 0) payload.attachments = pendingAttachments
  ws.send(JSON.stringify(payload))

  inputText.value = ''
  pendingAttachments = []
  renderAttachmentPreview()
  autoResize()
}

inputForm.addEventListener('submit', (e) => { e.preventDefault(); sendMessage() })
inputText.addEventListener('keydown', (e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); sendMessage() } })
inputText.addEventListener('input', autoResize)

// ── Sidebar ──
const isMobile = () => window.innerWidth <= 768

function openSidebar() {
  sidebar.classList.add('open')
  if (isMobile()) {
    sidebarOverlay.classList.remove('hidden')
    sidebarOverlay.classList.add('active')
  }
  fetchSessions()
}

function closeSidebar() {
  sidebar.classList.remove('open')
  sidebarOverlay.classList.add('hidden')
  sidebarOverlay.classList.remove('active')
}

function toggleSidebar() {
  sidebar.classList.contains('open') ? closeSidebar() : openSidebar()
}

// Open sidebar by default on desktop
if (!isMobile()) openSidebar()

function buildWelcomeHTML() {
  const cfg = window.__STARPOD__ || {}
  const greeting = cfg.greeting || 'ready_'
  let chips = ''
  if (cfg.prompts && cfg.prompts.length > 0) {
    chips = '<div class="mt-6 flex flex-col items-start gap-1.5">'
    for (const p of cfg.prompts) {
      chips += '<button class="prompt-chip" data-prompt="' + escapeHtml(p).replace(/"/g, '&quot;') + '">'
        + '<span class="text-dim font-mono mr-2">&gt;</span>' + escapeHtml(p) + '</button>'
    }
    chips += '</div>'
  }
  return '<div class="flex items-center justify-center text-center" id="welcome" style="min-height: calc(100dvh - 120px)">' +
    '<div>' +
      '<div class="font-mono text-3xl font-extrabold tracking-tighter mb-3 bg-gradient-to-b from-primary to-muted bg-clip-text text-transparent select-none">starpod</div>' +
      '<p class="text-sm text-dim font-mono">' + escapeHtml(greeting) + '</p>' +
      chips +
    '</div>' +
  '</div>'
}
const welcomeHTML = buildWelcomeHTML()

// Send a prompt from a suggestion chip
window._sendPrompt = function(text) {
  inputText.value = text
  sendMessage()
}

function newChat() {
  currentSessionId = null
  currentSessionKey = generateUUID()
  messages.innerHTML = welcomeHTML
  if (isMobile()) closeSidebar()
  // Deselect active session in sidebar
  sessionList.querySelectorAll('.session-item').forEach(el => el.classList.remove('active'))
  inputText.focus()
}

menuBtn.addEventListener('click', toggleSidebar)
sidebarClose.addEventListener('click', closeSidebar)
sidebarOverlay.addEventListener('click', closeSidebar)
newChatBtn.addEventListener('click', newChat)
newChatHeaderBtn.addEventListener('click', newChat)

// Prompt chip clicks (delegated)
messages.addEventListener('click', (e) => {
  const chip = e.target.closest('.prompt-chip')
  if (!chip) return
  window._sendPrompt(chip.dataset.prompt)
})

// Close transient sidebar when clicking outside it
document.addEventListener('mousedown', (e) => {
  if (sidebar.classList.contains('transient') && sidebar.classList.contains('open') && !sidebar.contains(e.target) && e.target !== menuBtn && !menuBtn.contains(e.target)) {
    closeSidebar()
  }
})

// ── Sessions ──
function formatSessionDate(isoStr) {
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

function fetchSessions(callback) {
  const token = localStorage.getItem('starpod_api_key')
  const headers = {}
  if (token) headers['X-API-Key'] = token

  fetch('/api/sessions?limit=50', { headers })
    .then(r => r.ok ? r.json() : Promise.reject(r.statusText))
    .then(sessions => {
      cachedSessions = sessions || []
      // Prune stale read markers
      const ids = new Set(cachedSessions.map(s => s.id))
      for (const id of readSessions) {
        if (!ids.has(id)) readSessions.delete(id)
      }
      saveReadSessions()
      renderSessions(cachedSessions)
      if (callback) callback()
    })
    .catch(() => { sessionList.innerHTML = '<div class="text-center text-dim text-xs py-8">Failed to load sessions</div>' })
}

function renderSessions(sessions) {
  if (!sessions || sessions.length === 0) {
    sessionList.innerHTML = '<div class="text-center text-dim text-xs py-8">No conversations yet</div>'
    return
  }

  // Sort: unread first (by recency), then read (by recency)
  const sorted = [...sessions].sort((a, b) => {
    const aUnread = isUnread(a.id) ? 1 : 0
    const bUnread = isUnread(b.id) ? 1 : 0
    if (aUnread !== bUnread) return bUnread - aUnread
    return new Date(b.last_message_at || b.created_at) - new Date(a.last_message_at || a.created_at)
  })

  sessionList.innerHTML = ''
  sorted.forEach(s => {
    const el = document.createElement('div')
    const active = s.id === currentSessionId
    const unread = isUnread(s.id) && !active
    el.className = 'session-item px-3.5 py-3 rounded-lg cursor-pointer mb-1' +
      (active ? ' active' : '')
    el.setAttribute('data-sid', s.id)

    const summary = s.title || s.summary || 'Untitled conversation'
    const date = formatSessionDate(s.last_message_at || s.created_at)
    const msgs = s.message_count || 0
    const closed = s.is_closed ? ' \u00b7 ended' : ''
    const dot = unread ? '<span class="unread-dot"></span>' : ''

    el.innerHTML =
      '<div class="flex items-start gap-2">' +
        dot +
        '<div class="flex-1 min-w-0">' +
          '<div class="text-[13px] leading-snug line-clamp-2 break-words' + (active ? ' text-primary font-medium' : ' text-secondary') + '">' + escapeHtml(summary) + '</div>' +
          '<div class="font-mono text-[11px] text-dim mt-1 flex gap-2">' +
            '<span>' + date + '</span>' +
            '<span>' + msgs + ' msg' + (msgs !== 1 ? 's' : '') + closed + '</span>' +
          '</div>' +
        '</div>' +
      '</div>'

    el._sessionId = s.id
    el.addEventListener('click', () => selectSession(s))
    sessionList.appendChild(el)
  })
}

function selectSession(session) {
  if (isMobile() || sidebar.classList.contains('transient')) closeSidebar()
  currentSessionId = session.id
  currentSessionKey = session.channel_session_key || generateUUID()
  // Mark as read and update sidebar
  markRead(session.id)
  sessionList.querySelectorAll('.session-item').forEach(el => {
    el.classList.toggle('active', el._sessionId === session.id)
  })

  messages.innerHTML = '<div class="text-center text-dim text-sm py-8">Loading messages...</div>'

  const token = localStorage.getItem('starpod_api_key')
  const headers = {}
  if (token) headers['X-API-Key'] = token

  fetch('/api/sessions/' + encodeURIComponent(session.id) + '/messages', { headers })
    .then(r => r.ok ? r.json() : Promise.reject(r.statusText))
    .then(msgs => {
      messages.innerHTML = ''
      if (!msgs || msgs.length === 0) {
        messages.innerHTML = '<div class="flex items-center justify-center text-center" style="min-height: calc(100dvh - 120px)" id="welcome"><div><div class="font-mono text-3xl font-extrabold tracking-tighter mb-2 bg-gradient-to-b from-primary to-muted bg-clip-text text-transparent">starpod</div><p class="text-sm text-muted">No messages in this conversation.</p></div></div>'
        return
      }

      let currentAssistantMsg = null

      msgs.forEach(m => {
        if (m.role === 'user') {
          currentAssistantMsg = null
          const el = document.createElement('div')
          el.className = 'max-w-[80%] self-end mt-4'
          el.innerHTML = '<div class="bg-accent text-white rounded-2xl rounded-br-md px-4 py-2.5 leading-relaxed text-sm whitespace-pre-wrap break-words">' + formatUserText(m.content) + '</div>'
          messages.appendChild(el)
        } else if (m.role === 'assistant') {
          const el = document.createElement('div')
          el.className = 'max-w-full mt-2'
          el.innerHTML = '<div class="py-1 leading-[1.75] text-sm break-words text-secondary markdown-body">' + formatText(m.content) + '</div>'
          messages.appendChild(el)
          currentAssistantMsg = el
        } else if (m.role === 'tool_use') {
          if (!currentAssistantMsg) {
            currentAssistantMsg = document.createElement('div')
            currentAssistantMsg.className = 'max-w-full mt-2'
            messages.appendChild(currentAssistantMsg)
          }
          try {
            const data = JSON.parse(m.content)
            const id = 'hist-tool-' + (toolCounter++)
            const preview = getToolPreview(data.name, data.input || {})
            const inputJson = JSON.stringify(data.input || {}, null, 2)

            const el = document.createElement('div')
            el.className = 'my-1.5 rounded-lg overflow-hidden border border-border-subtle bg-surface transition-colors hover:border-border-main'
            el.id = id
            el.innerHTML =
              '<div class="flex items-center gap-2 px-3 py-2 cursor-pointer select-none text-xs text-secondary transition-colors hover:bg-elevated" onclick="toggleTool(\'' + id + '\')">' +
                '<span class="tool-chevron text-[8px] text-dim transition-transform duration-200 shrink-0 w-3 text-center">\u25B6</span>' +
                '<span class="' + toolIconClass(data.name) + ' text-[10px] w-5 h-5 flex items-center justify-center rounded-md shrink-0 font-semibold">' + toolIconSymbol(data.name) + '</span>' +
                '<span class="font-mono font-medium text-[12px] text-secondary">' + escapeHtml(data.name) + '</span>' +
                '<span class="text-dim font-mono text-[11px] whitespace-nowrap overflow-hidden text-ellipsis flex-1 min-w-0">' + escapeHtml(preview) + '</span>' +
                '<span class="font-mono text-[10px] px-2 py-0.5 rounded-full font-medium shrink-0 tracking-wide bg-ok-muted text-ok">done</span>' +
              '</div>' +
              '<div class="tool-body hidden px-3 pb-3 text-xs text-secondary">' +
                '<div>' +
                  '<div class="font-mono text-[10px] uppercase tracking-widest text-dim mb-1 font-medium">Input</div>' +
                  '<pre class="bg-bg border border-border-subtle rounded-md px-3 py-2.5 font-mono text-[11.5px] leading-normal whitespace-pre-wrap break-all text-dim max-h-60 overflow-y-auto">' + escapeHtml(inputJson) + '</pre>' +
                '</div>' +
              '</div>'
            currentAssistantMsg.appendChild(el)
          } catch { /* ignore malformed tool_use */ }
        } else if (m.role === 'tool_result') {
          if (currentAssistantMsg) {
            const lastTool = currentAssistantMsg.querySelector('[id^="hist-tool-"]:last-child')
            if (lastTool) {
              try {
                const data = JSON.parse(m.content)
                if (data.is_error) {
                  const badge = lastTool.querySelector('[class*="bg-ok-muted"]')
                  if (badge) {
                    badge.textContent = 'error'
                    badge.className = 'font-mono text-[10px] px-2 py-0.5 rounded-full font-medium shrink-0 tracking-wide bg-err-muted text-err'
                  }
                }
                if (data.content) {
                  const body = lastTool.querySelector('.tool-body')
                  if (body) {
                    const section = document.createElement('div')
                    section.className = 'mt-2'
                    section.innerHTML =
                      '<div class="font-mono text-[10px] uppercase tracking-widest text-dim mb-1 font-medium">Result</div>' +
                      '<pre class="bg-bg border border-border-subtle rounded-md px-3 py-2.5 font-mono text-[11.5px] leading-normal whitespace-pre-wrap break-all text-dim max-h-60 overflow-y-auto">' + escapeHtml(data.content) + '</pre>'
                    body.appendChild(section)
                  }
                }
              } catch { /* ignore */ }
            }
          }
        }
      })
      scrollToBottom()
    })
    .catch(() => {
      messages.innerHTML = '<div class="flex items-center justify-center text-center" style="min-height: calc(100dvh - 120px)" id="welcome"><div><div class="font-mono text-3xl font-extrabold tracking-tighter mb-2 bg-gradient-to-b from-primary to-muted bg-clip-text text-transparent">starpod</div><p class="text-sm text-muted">Failed to load messages.</p></div></div>'
    })

  inputText.focus()
}

// ── Settings ──
const settingsBtn = document.getElementById('settings-btn')
let settingsVisible = false
let settingsActiveTab = 'general'

function apiHeaders() {
  const h = { 'Content-Type': 'application/json' }
  const token = localStorage.getItem('starpod_api_key')
  if (token) h['X-API-Key'] = token
  return h
}

function showSettings() {
  settingsVisible = true
  settingsBtn.classList.add('active')
  document.querySelector('#input-form').closest('.shrink-0.pt-2').style.display = 'none'
  renderSettingsView()
}

function hideSettings() {
  settingsVisible = false
  settingsBtn.classList.remove('active')
  document.querySelector('#input-form').closest('.shrink-0.pt-2').style.display = ''
  if (currentSessionId) {
    const s = cachedSessions.find(s => s.id === currentSessionId)
    if (s) selectSession(s)
    else messages.innerHTML = welcomeHTML
  } else {
    messages.innerHTML = welcomeHTML
  }
}

settingsBtn.addEventListener('click', () => {
  settingsVisible ? hideSettings() : showSettings()
})

function renderSettingsView() {
  const tabs = [
    { id: 'general', label: 'General' },
    { id: 'soul', label: 'Soul' },
    { id: 'frontend', label: 'Frontend' },
    { id: 'memory', label: 'Memory' },
    { id: 'cron', label: 'Cron' },
    { id: 'users', label: 'Users' },
  ]

  const scroll = document.getElementById('messages-scroll')
  scroll.innerHTML = ''

  const container = document.createElement('div')
  container.className = 'max-w-[740px] mx-auto px-5 py-6'

  container.innerHTML =
    '<div class="flex items-center gap-3 mb-6">' +
      '<button id="settings-back" class="text-muted hover:text-primary p-1.5 rounded-lg hover:bg-elevated transition-colors cursor-pointer">' +
        '<svg class="w-4 h-4 stroke-current fill-none stroke-2" viewBox="0 0 24 24" stroke-linecap="round"><path d="M19 12H5M12 19l-7-7 7-7"/></svg>' +
      '</button>' +
      '<h1 class="text-lg font-semibold text-primary">Settings</h1>' +
    '</div>' +
    '<div class="flex gap-1 mb-6 overflow-x-auto pb-1 border-b border-border-subtle" id="settings-tabs"></div>' +
    '<div id="settings-content"></div>'

  scroll.appendChild(container)

  const tabBar = document.getElementById('settings-tabs')
  tabs.forEach(tab => {
    const btn = document.createElement('button')
    btn.className = 'settings-tab px-3 py-2 text-sm font-medium whitespace-nowrap cursor-pointer transition-colors ' +
      (tab.id === settingsActiveTab ? 'active text-accent' : 'text-muted hover:text-primary')
    btn.textContent = tab.label
    btn.addEventListener('click', () => { settingsActiveTab = tab.id; renderSettingsView() })
    tabBar.appendChild(btn)
  })

  document.getElementById('settings-back').addEventListener('click', hideSettings)
  loadTabContent(settingsActiveTab)
}

async function loadTabContent(tab) {
  const el = document.getElementById('settings-content')
  el.innerHTML = '<div class="text-center text-dim text-sm py-8">Loading...</div>'
  try {
    switch (tab) {
      case 'general': await renderGeneralTab(el); break
      case 'soul': await renderSoulTab(el); break
      case 'frontend': await renderFrontendTab(el); break
      case 'memory': await renderMemoryTab(el); break
      case 'cron': await renderCronTab(el); break
      case 'users': await renderUsersTab(el); break
    }
  } catch (e) {
    el.innerHTML = '<div class="text-center text-err text-sm py-8">Failed to load: ' + escapeHtml(e.message) + '</div>'
  }
}

// ── Form helpers ──

function sField(label, desc, html) {
  return '<div class="settings-field">' +
    '<label>' + escapeHtml(label) + '</label>' +
    (desc ? '<div class="field-desc">' + escapeHtml(desc) + '</div>' : '') +
    html +
  '</div>'
}

function sInput(id, value, type, attrs) {
  return '<input id="' + id + '" type="' + (type || 'text') + '" value="' + escapeHtml(String(value ?? '')) + '" ' + (attrs || '') + ' class="settings-input">'
}

function sSelect(id, value, options) {
  let html = '<select id="' + id + '" class="settings-input">'
  options.forEach(o => {
    const val = typeof o === 'string' ? o : o.value
    const label = typeof o === 'string' ? o : o.label
    html += '<option value="' + escapeHtml(val) + '"' + (val === value ? ' selected' : '') + '>' + escapeHtml(label) + '</option>'
  })
  return html + '</select>'
}

function sToggle(id, checked) {
  return '<label class="toggle-switch"><input type="checkbox" id="' + id + '"' + (checked ? ' checked' : '') + '><div class="toggle-track"></div></label>'
}

function sTextarea(id, value, rows) {
  return '<textarea id="' + id + '" rows="' + (rows || 10) + '" class="settings-input">' + escapeHtml(value || '') + '</textarea>'
}

function sSaveBar() {
  return '<div class="settings-save-bar">' +
    '<button id="settings-save" class="bg-accent text-white px-6 py-2.5 rounded-xl text-sm font-medium hover:bg-blue-500 transition-colors cursor-pointer disabled:opacity-30 disabled:cursor-default">Save changes</button>' +
    '<span id="settings-status" class="ml-3 text-xs text-dim"></span>' +
  '</div>'
}

async function doSave(url, gatherFn) {
  const btn = document.getElementById('settings-save')
  const status = document.getElementById('settings-status')
  btn.disabled = true
  status.textContent = 'Saving...'
  status.className = 'ml-3 text-xs text-dim'
  try {
    const body = gatherFn()
    const resp = await fetch(url, { method: 'PUT', headers: apiHeaders(), body: JSON.stringify(body) })
    if (!resp.ok) {
      const data = await resp.json().catch(() => ({}))
      throw new Error(data.error || resp.statusText)
    }
    status.textContent = 'Saved!'
    status.className = 'ml-3 text-xs text-ok'
    setTimeout(() => { if (status) { status.textContent = ''; status.className = 'ml-3 text-xs text-dim' } }, 2500)
  } catch (e) {
    status.textContent = 'Error: ' + e.message
    status.className = 'ml-3 text-xs text-err'
  } finally {
    btn.disabled = false
  }
}

// ── General tab ──

async function renderGeneralTab(el) {
  const resp = await fetch('/api/settings/general', { headers: apiHeaders() })
  if (!resp.ok) throw new Error('Failed to load')
  const d = await resp.json()

  el.innerHTML =
    sField('Provider', 'LLM provider to use', sSelect('s-provider', d.provider, [
      'anthropic', 'openai', 'gemini', 'groq', 'deepseek', 'openrouter', 'ollama'
    ])) +
    sField('Model', 'Model identifier', sInput('s-model', d.model)) +
    '<div class="grid grid-cols-2 gap-4">' +
      sField('Max turns', 'Agentic turns per request', sInput('s-max-turns', d.max_turns, 'number', 'min="1" max="200"')) +
      sField('Max tokens', 'Response token limit', sInput('s-max-tokens', d.max_tokens, 'number', 'min="256" max="131072" step="256"')) +
    '</div>' +
    sField('Agent name', 'Display name for the AI assistant', sInput('s-agent-name', d.agent_name)) +
    sField('Timezone', 'IANA timezone for cron scheduling', sInput('s-timezone', d.timezone || '', 'text', 'placeholder="e.g. Europe/Rome"')) +
    sField('Reasoning effort', 'Extended thinking level', sSelect('s-reasoning', d.reasoning_effort || '', [
      { value: '', label: 'None' }, { value: 'low', label: 'Low' }, { value: 'medium', label: 'Medium' }, { value: 'high', label: 'High' }
    ])) +
    sField('Compaction model', 'Model for conversation compaction (blank = same as primary)', sInput('s-compaction-model', d.compaction_model || '', 'text', 'placeholder="same as primary"')) +
    sField('Followup mode', 'How followup messages are handled during active loops', sSelect('s-followup', d.followup_mode, [
      { value: 'inject', label: 'Inject' }, { value: 'queue', label: 'Queue' }
    ])) +
    sField('Server address', 'Bind address (restart required to take effect)', sInput('s-server-addr', d.server_addr)) +
    sSaveBar()

  document.getElementById('settings-save').addEventListener('click', () => {
    doSave('/api/settings/general', () => ({
      provider: document.getElementById('s-provider').value,
      model: document.getElementById('s-model').value,
      max_turns: parseInt(document.getElementById('s-max-turns').value) || 30,
      max_tokens: parseInt(document.getElementById('s-max-tokens').value) || 16384,
      agent_name: document.getElementById('s-agent-name').value,
      timezone: document.getElementById('s-timezone').value || null,
      reasoning_effort: document.getElementById('s-reasoning').value || null,
      compaction_model: document.getElementById('s-compaction-model').value || null,
      followup_mode: document.getElementById('s-followup').value,
      server_addr: document.getElementById('s-server-addr').value,
    }))
  })
}

// ── Soul tab ──

async function renderSoulTab(el) {
  const [soulResp, hbResp] = await Promise.all([
    fetch('/api/settings/files/SOUL.md', { headers: apiHeaders() }),
    fetch('/api/settings/files/HEARTBEAT.md', { headers: apiHeaders() }),
  ])
  const soul = soulResp.ok ? await soulResp.json() : { content: '' }
  const hb = hbResp.ok ? await hbResp.json() : { content: '' }

  el.innerHTML =
    '<div class="settings-section">' +
      '<div class="settings-section-title">Soul / Personality</div>' +
      '<div class="field-desc mb-3" style="color: var(--color-dim); font-size: 0.75rem;">Defines the agent\'s personality, tone, and behavior. Loaded at the start of every conversation.</div>' +
      sTextarea('s-soul', soul.content, 20) +
      '<div class="mt-2"><button id="save-soul" class="bg-accent text-white px-5 py-2 rounded-xl text-sm font-medium hover:bg-blue-500 transition-colors cursor-pointer">Save SOUL.md</button>' +
      '<span id="soul-status" class="ml-3 text-xs text-dim"></span></div>' +
    '</div>' +
    '<div class="settings-section">' +
      '<div class="settings-section-title">Heartbeat</div>' +
      '<div class="field-desc mb-3" style="color: var(--color-dim); font-size: 0.75rem;">Instructions for periodic heartbeat jobs. Runs on a schedule to perform maintenance, check-ins, etc.</div>' +
      sTextarea('s-heartbeat', hb.content, 12) +
      '<div class="mt-2"><button id="save-hb" class="bg-accent text-white px-5 py-2 rounded-xl text-sm font-medium hover:bg-blue-500 transition-colors cursor-pointer">Save HEARTBEAT.md</button>' +
      '<span id="hb-status" class="ml-3 text-xs text-dim"></span></div>' +
    '</div>'

  async function saveFile(btnId, statusId, name, textareaId) {
    const btn = document.getElementById(btnId)
    const status = document.getElementById(statusId)
    btn.disabled = true
    status.textContent = 'Saving...'
    status.className = 'ml-3 text-xs text-dim'
    try {
      const content = document.getElementById(textareaId).value
      const resp = await fetch('/api/settings/files/' + name, {
        method: 'PUT', headers: apiHeaders(), body: JSON.stringify({ content })
      })
      if (!resp.ok) { const d = await resp.json().catch(() => ({})); throw new Error(d.error || resp.statusText) }
      status.textContent = 'Saved!'
      status.className = 'ml-3 text-xs text-ok'
      setTimeout(() => { status.textContent = ''; status.className = 'ml-3 text-xs text-dim' }, 2500)
    } catch (e) {
      status.textContent = 'Error: ' + e.message
      status.className = 'ml-3 text-xs text-err'
    } finally {
      btn.disabled = false
    }
  }

  document.getElementById('save-soul').addEventListener('click', () => saveFile('save-soul', 'soul-status', 'SOUL.md', 's-soul'))
  document.getElementById('save-hb').addEventListener('click', () => saveFile('save-hb', 'hb-status', 'HEARTBEAT.md', 's-heartbeat'))
}

// ── Frontend tab ──

async function renderFrontendTab(el) {
  const resp = await fetch('/api/settings/frontend', { headers: apiHeaders() })
  if (!resp.ok) throw new Error('Failed to load')
  const d = await resp.json()

  function renderPrompts(prompts) {
    let html = '<div id="prompts-list">'
    prompts.forEach((p, i) => {
      html += '<div class="prompt-item">' +
        '<input type="text" class="settings-input prompt-input" value="' + escapeHtml(p) + '" placeholder="Prompt text...">' +
        '<button onclick="removePromptItem(this)" title="Remove">&times;</button>' +
      '</div>'
    })
    html += '</div>' +
      '<button id="add-prompt" class="text-accent text-sm font-medium mt-1 cursor-pointer bg-transparent border-none hover:underline">+ Add prompt</button>'
    return html
  }

  el.innerHTML =
    sField('Greeting', 'Welcome text shown on the home screen', sInput('s-greeting', d.greeting || '', 'text', 'placeholder="ready_"')) +
    '<div class="settings-field">' +
      '<label>Suggested prompts</label>' +
      '<div class="field-desc">Prompt chips shown on the welcome screen</div>' +
      renderPrompts(d.prompts || []) +
    '</div>' +
    sSaveBar()

  document.getElementById('add-prompt').addEventListener('click', () => {
    const list = document.getElementById('prompts-list')
    const item = document.createElement('div')
    item.className = 'prompt-item'
    item.innerHTML = '<input type="text" class="settings-input prompt-input" value="" placeholder="Prompt text...">' +
      '<button onclick="removePromptItem(this)" title="Remove">&times;</button>'
    list.appendChild(item)
    item.querySelector('input').focus()
  })

  document.getElementById('settings-save').addEventListener('click', () => {
    doSave('/api/settings/frontend', () => {
      const prompts = Array.from(document.querySelectorAll('.prompt-input')).map(i => i.value.trim()).filter(Boolean)
      return {
        greeting: document.getElementById('s-greeting').value || null,
        prompts,
      }
    })
  })
}

window.removePromptItem = function(btn) {
  btn.closest('.prompt-item').remove()
}

// ── Memory tab ──

async function renderMemoryTab(el) {
  const resp = await fetch('/api/settings/memory', { headers: apiHeaders() })
  if (!resp.ok) throw new Error('Failed to load')
  const d = await resp.json()

  el.innerHTML =
    sField('Half-life days', 'Temporal decay on daily logs — lower = faster forgetting', sInput('s-half-life', d.half_life_days, 'number', 'min="1" step="1"')) +
    sField('MMR lambda', '0 = max diversity, 1 = pure relevance',
      '<div class="flex items-center gap-3">' +
        '<input type="range" id="s-mmr" class="settings-range flex-1" min="0" max="1" step="0.05" value="' + d.mmr_lambda + '">' +
        '<span id="mmr-val" class="text-sm font-mono text-secondary w-10 text-right">' + d.mmr_lambda.toFixed(2) + '</span>' +
      '</div>'
    ) +
    '<div class="flex items-center justify-between settings-field">' +
      '<div><label>Vector search</label><div class="field-desc">Enable vector-based semantic search</div></div>' +
      sToggle('s-vector', d.vector_search) +
    '</div>' +
    '<div class="grid grid-cols-2 gap-4">' +
      sField('Chunk size', 'Characters per chunk for indexing', sInput('s-chunk-size', d.chunk_size, 'number', 'min="200" step="100"')) +
      sField('Chunk overlap', 'Overlap in characters between chunks', sInput('s-chunk-overlap', d.chunk_overlap, 'number', 'min="0" step="50"')) +
    '</div>' +
    '<div class="flex items-center justify-between settings-field">' +
      '<div><label>Export sessions</label><div class="field-desc">Export closed session transcripts for long-term recall</div></div>' +
      sToggle('s-export', d.export_sessions) +
    '</div>' +
    sSaveBar()

  document.getElementById('s-mmr').addEventListener('input', (e) => {
    document.getElementById('mmr-val').textContent = parseFloat(e.target.value).toFixed(2)
  })

  document.getElementById('settings-save').addEventListener('click', () => {
    doSave('/api/settings/memory', () => ({
      half_life_days: parseFloat(document.getElementById('s-half-life').value) || 30,
      mmr_lambda: parseFloat(document.getElementById('s-mmr').value) || 0.7,
      vector_search: document.getElementById('s-vector').checked,
      chunk_size: parseInt(document.getElementById('s-chunk-size').value) || 1600,
      chunk_overlap: parseInt(document.getElementById('s-chunk-overlap').value) || 320,
      export_sessions: document.getElementById('s-export').checked,
    }))
  })
}

// ── Cron tab ──

async function renderCronTab(el) {
  const resp = await fetch('/api/settings/cron', { headers: apiHeaders() })
  if (!resp.ok) throw new Error('Failed to load')
  const d = await resp.json()

  function timeoutHint(secs) {
    if (secs >= 3600) return (secs / 3600).toFixed(1).replace(/\.0$/, '') + ' hours'
    return (secs / 60).toFixed(0) + ' minutes'
  }

  el.innerHTML =
    sField('Max retries', 'Default maximum retries for failed jobs', sInput('s-retries', d.default_max_retries, 'number', 'min="0" max="20"')) +
    sField('Timeout', 'Default job timeout in seconds',
      sInput('s-timeout', d.default_timeout_secs, 'number', 'min="60" step="60"') +
      '<div class="field-desc mt-1" id="timeout-hint">= ' + timeoutHint(d.default_timeout_secs) + '</div>'
    ) +
    sField('Max concurrent runs', 'Maximum jobs running simultaneously', sInput('s-concurrent', d.max_concurrent_runs, 'number', 'min="1" max="10"')) +
    sSaveBar()

  document.getElementById('s-timeout').addEventListener('input', (e) => {
    const hint = document.getElementById('timeout-hint')
    const v = parseInt(e.target.value)
    if (v > 0) hint.textContent = '= ' + timeoutHint(v)
  })

  document.getElementById('settings-save').addEventListener('click', () => {
    doSave('/api/settings/cron', () => ({
      default_max_retries: parseInt(document.getElementById('s-retries').value) || 3,
      default_timeout_secs: parseInt(document.getElementById('s-timeout').value) || 7200,
      max_concurrent_runs: parseInt(document.getElementById('s-concurrent').value) || 1,
    }))
  })
}

// ── Users tab ──

async function renderUsersTab(el) {
  const resp = await fetch('/api/settings/users', { headers: apiHeaders() })
  if (!resp.ok) throw new Error('Failed to load')
  const users = await resp.json()

  let html =
    '<div class="flex items-center gap-3 mb-4">' +
      '<input type="text" id="new-user-id" class="settings-input flex-1" placeholder="New user ID...">' +
      '<button id="create-user-btn" class="bg-accent text-white px-4 py-2 rounded-xl text-sm font-medium hover:bg-blue-500 transition-colors cursor-pointer shrink-0">Create</button>' +
    '</div>' +
    '<div id="create-user-status" class="text-xs mb-3"></div>' +
    '<div id="users-list">'

  if (users.length === 0) {
    html += '<div class="text-center text-dim text-sm py-8">No users yet</div>'
  } else {
    users.forEach(u => {
      html +=
        '<div class="user-card" data-user="' + escapeHtml(u.id) + '">' +
          '<div class="flex items-center justify-between mb-2">' +
            '<div class="flex items-center gap-2">' +
              '<span class="font-mono text-sm font-semibold text-primary">' + escapeHtml(u.id) + '</span>' +
              '<span class="font-mono text-[11px] text-dim">' + u.daily_log_count + ' daily log' + (u.daily_log_count !== 1 ? 's' : '') + '</span>' +
            '</div>' +
            '<div class="flex items-center gap-1">' +
              '<button class="user-edit-btn text-muted hover:text-accent text-xs font-medium px-2 py-1 rounded hover:bg-accent-muted transition-colors cursor-pointer" data-uid="' + escapeHtml(u.id) + '">Edit</button>' +
              '<button class="user-delete-btn text-muted hover:text-err text-xs font-medium px-2 py-1 rounded hover:bg-err-muted transition-colors cursor-pointer" data-uid="' + escapeHtml(u.id) + '">Delete</button>' +
            '</div>' +
          '</div>' +
          '<div class="user-edit-area hidden" id="user-edit-' + escapeHtml(u.id) + '"></div>' +
        '</div>'
    })
  }
  html += '</div>'
  el.innerHTML = html

  // Create user
  document.getElementById('create-user-btn').addEventListener('click', async () => {
    const input = document.getElementById('new-user-id')
    const status = document.getElementById('create-user-status')
    const id = input.value.trim()
    if (!id) return
    status.textContent = 'Creating...'
    status.className = 'text-xs mb-3 text-dim'
    try {
      const resp = await fetch('/api/settings/users', {
        method: 'POST', headers: apiHeaders(), body: JSON.stringify({ id })
      })
      if (!resp.ok) { const d = await resp.json().catch(() => ({})); throw new Error(d.error || resp.statusText) }
      input.value = ''
      status.textContent = ''
      renderUsersTab(el)
    } catch (e) {
      status.textContent = 'Error: ' + e.message
      status.className = 'text-xs mb-3 text-err'
    }
  })

  // Edit buttons
  el.querySelectorAll('.user-edit-btn').forEach(btn => {
    btn.addEventListener('click', async () => {
      const uid = btn.dataset.uid
      const area = document.getElementById('user-edit-' + uid)
      if (!area.classList.contains('hidden')) {
        area.classList.add('hidden')
        return
      }
      area.innerHTML = '<div class="text-dim text-xs py-2">Loading...</div>'
      area.classList.remove('hidden')

      try {
        const resp = await fetch('/api/settings/users/' + encodeURIComponent(uid), { headers: apiHeaders() })
        if (!resp.ok) throw new Error('Failed to load')
        const data = await resp.json()

        area.innerHTML =
          '<div class="mt-2">' +
            '<div class="font-mono text-[11px] text-dim uppercase tracking-wider mb-1">USER.md</div>' +
            '<textarea id="user-md-' + escapeHtml(uid) + '" rows="12" class="settings-input">' + escapeHtml(data.user_md) + '</textarea>' +
            '<div class="flex items-center gap-2 mt-2">' +
              '<button class="user-save-btn bg-accent text-white px-4 py-1.5 rounded-lg text-xs font-medium hover:bg-blue-500 transition-colors cursor-pointer" data-uid="' + escapeHtml(uid) + '">Save</button>' +
              '<span class="user-save-status text-xs text-dim"></span>' +
            '</div>' +
          '</div>'

        area.querySelector('.user-save-btn').addEventListener('click', async function() {
          const saveBtn = this
          const status = this.nextElementSibling
          saveBtn.disabled = true
          status.textContent = 'Saving...'
          try {
            const content = document.getElementById('user-md-' + uid).value
            const resp = await fetch('/api/settings/users/' + encodeURIComponent(uid), {
              method: 'PUT', headers: apiHeaders(), body: JSON.stringify({ content })
            })
            if (!resp.ok) { const d = await resp.json().catch(() => ({})); throw new Error(d.error || resp.statusText) }
            status.textContent = 'Saved!'
            status.className = 'user-save-status text-xs text-ok'
            setTimeout(() => { status.textContent = ''; status.className = 'user-save-status text-xs text-dim' }, 2500)
          } catch (e) {
            status.textContent = 'Error: ' + e.message
            status.className = 'user-save-status text-xs text-err'
          } finally {
            saveBtn.disabled = false
          }
        })
      } catch (e) {
        area.innerHTML = '<div class="text-err text-xs py-2">Error: ' + escapeHtml(e.message) + '</div>'
      }
    })
  })

  // Delete buttons
  el.querySelectorAll('.user-delete-btn').forEach(btn => {
    btn.addEventListener('click', async () => {
      const uid = btn.dataset.uid
      if (!confirm('Delete user "' + uid + '"? This removes all their data including daily logs and memory.')) return
      try {
        const resp = await fetch('/api/settings/users/' + encodeURIComponent(uid), {
          method: 'DELETE', headers: apiHeaders()
        })
        if (!resp.ok) { const d = await resp.json().catch(() => ({})); throw new Error(d.error || resp.statusText) }
        renderUsersTab(el)
      } catch (e) {
        alert('Failed to delete: ' + e.message)
      }
    })
  })
}

// ── Keyboard shortcuts ──
document.addEventListener('keydown', (e) => {
  if ((e.metaKey || e.ctrlKey) && e.key === 'k') { e.preventDefault(); inputText.focus() }
  if (e.key === 'Escape') {
    if (settingsVisible) { hideSettings(); return }
    if (sidebar.classList.contains('open') && isMobile()) closeSidebar()
  }
  if ((e.metaKey || e.ctrlKey) && e.key === ',') { e.preventDefault(); settingsVisible ? hideSettings() : showSettings() }
})

// ── Init ──
messages.innerHTML = welcomeHTML
connect()
