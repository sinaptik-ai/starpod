import './style.css'

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
const attachBtn = document.getElementById('attach-btn')
const fileInput = document.getElementById('file-input')
const attachmentPreview = document.getElementById('attachment-preview')

// ── State ──
let ws = null
let isStreaming = false
let currentMsg = null
let currentBubble = null
let reconnectAttempt = 0
let toolCounter = 0
let currentSessionId = null
let pendingAttachments = []

const MAX_FILE_SIZE = 20 * 1024 * 1024

// ── Helpers ──
function setStatus(state) {
  statusDot.className = 'w-1.5 h-1.5 rounded-full shrink-0 dot-' + state
  const labels = { connected: 'connected', connecting: 'connecting', disconnected: 'disconnected' }
  statusText.textContent = labels[state] || state
}

function scrollToBottom() {
  requestAnimationFrame(() => { messages.scrollTop = messages.scrollHeight })
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
  let html = escapeHtml(text)

  // Code blocks with language labels and copy buttons
  html = html.replace(/```(\w*)\n([\s\S]*?)```/g, (_, lang, code) => {
    const langLabel = lang || 'code'
    const codeId = 'code-' + Math.random().toString(36).slice(2, 8)
    return '<pre class="bg-bg border border-border-main rounded-lg my-3 overflow-x-auto font-mono text-[13px] leading-relaxed text-secondary">' +
      '<div class="flex items-center justify-between px-3 py-1.5 border-b border-border-subtle text-[11px] text-dim font-mono tracking-wide select-none">' +
        '<span>' + escapeHtml(langLabel) + '</span>' +
        '<button class="copy-btn bg-transparent border border-border-main text-dim cursor-pointer font-mono text-[11px] px-2 py-0.5 rounded transition-all" onclick="copyCode(this, \'' + codeId + '\')">copy</button>' +
      '</div>' +
      '<div class="px-4 py-3.5" id="' + codeId + '">' + code + '</div></pre>'
  })
  html = html.replace(/`([^`]+)`/g, '<code class="bg-elevated border border-border-subtle px-1.5 py-0.5 rounded font-mono text-[12.5px] text-accent">$1</code>')
  html = html.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
  html = html.replace(/(?<!\*)\*([^*]+)\*(?!\*)/g, '<em>$1</em>')
  html = html.replace(/(?<![="'])(https?:\/\/[^\s<>"')\]]+)/g, '<a href="$1" target="_blank" rel="noopener" class="text-accent no-underline border-b border-accent/30 hover:border-accent transition-colors">$1</a>')
  return html
}

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
        html += '<div class="bg-white/8 px-2.5 py-1.5 rounded-xl font-mono text-[11px] text-white/70">\u{1F4CE} ' + escapeHtml(att.file_name) + '</div>'
      }
    }
    html += '</div>'
  }
  if (text) html += '<div class="bg-accent-strong text-white rounded-[18px] rounded-br-[4px] px-4 py-2.5 leading-relaxed text-sm whitespace-pre-wrap break-words">' + escapeHtml(text) + '</div>'
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

  // Thinking indicator
  const thinking = document.createElement('div')
  thinking.className = 'flex items-center gap-1.5 py-2 text-dim font-mono text-xs'
  thinking.innerHTML = '<div class="flex gap-1"><span class="thinking-dot"></span><span class="thinking-dot"></span><span class="thinking-dot"></span></div>'
  msg._thinkingEl = thinking
  msg.appendChild(thinking)

  scrollToBottom()
}

function removeThinking() {
  if (currentMsg && currentMsg._thinkingEl) {
    currentMsg._thinkingEl.remove()
    currentMsg._thinkingEl = null
  }
}

function ensureBubble() {
  if (currentBubble) return currentBubble
  if (!currentMsg) return null
  removeThinking()
  const bubble = document.createElement('div')
  bubble.className = 'py-1 leading-[1.75] text-sm whitespace-pre-wrap break-words text-secondary streaming-cursor'
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

function addToolUse(name, input) {
  if (!currentMsg) return
  removeThinking()
  if (currentBubble) {
    currentBubble.classList.remove('streaming-cursor')
    currentBubble = null
  }

  const id = 'tool-' + (toolCounter++)
  const preview = getToolPreview(name, input)
  const inputJson = JSON.stringify(input, null, 2)

  const el = document.createElement('div')
  el.className = 'my-1 rounded-md overflow-hidden border border-border-subtle bg-surface transition-colors hover:border-border-main'
  el.id = id
  el.innerHTML =
    '<div class="flex items-center gap-2 px-2.5 py-1.5 cursor-pointer select-none text-xs text-secondary transition-colors hover:bg-elevated" onclick="toggleTool(\'' + id + '\')">' +
      '<span class="tool-chevron text-[8px] text-dim transition-transform duration-200 shrink-0 w-3 text-center">\u25B6</span>' +
      '<span class="' + toolIconClass(name) + ' text-[10px] w-5 h-5 flex items-center justify-center rounded-[5px] shrink-0 font-semibold">' + toolIconSymbol(name) + '</span>' +
      '<span class="font-mono font-medium text-[11.5px] text-secondary">' + escapeHtml(name) + '</span>' +
      '<span class="text-dim font-mono text-[11px] whitespace-nowrap overflow-hidden text-ellipsis flex-1 min-w-0">' + escapeHtml(preview) + '</span>' +
      '<span class="font-mono text-[10px] px-1.5 py-px rounded-full font-medium shrink-0 tracking-wide bg-accent-muted text-accent badge-running">running</span>' +
    '</div>' +
    '<div class="tool-body hidden px-2.5 pb-2.5 text-xs text-secondary">' +
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

function addToolResult(content, isError) {
  if (!currentMsg) return
  const tools = currentMsg.querySelectorAll('[id^="tool-"]')
  const last = tools[tools.length - 1]
  if (!last) return

  const badge = last.querySelector('.badge-running, [class*="badge"]')
  if (!badge) return

  if (isError) {
    badge.textContent = 'error'
    badge.className = 'font-mono text-[10px] px-1.5 py-px rounded-full font-medium shrink-0 tracking-wide bg-err-muted text-err'
  } else {
    badge.textContent = 'done'
    badge.className = 'font-mono text-[10px] px-1.5 py-px rounded-full font-medium shrink-0 tracking-wide bg-ok-muted text-ok'
  }

  const resultSection = last.querySelector('.tool-result-section')
  const resultPre = last.querySelector('.tool-result-pre')
  if (resultSection && resultPre && content) {
    resultPre.textContent = content
    resultSection.classList.remove('hidden')
  }

  scrollToBottom()
}

function endStream(data) {
  if (currentMsg) {
    removeThinking()
    currentMsg.querySelectorAll('.streaming-cursor').forEach(b => {
      b.classList.remove('streaming-cursor')
      if (b._rawText) b.innerHTML = formatText(b._rawText)
    })

    if (data.is_error && data.errors && data.errors.length > 0) {
      const hasText = Array.from(currentMsg.querySelectorAll('[class*="whitespace-pre-wrap"]')).some(b => b._rawText)
      if (!hasText) {
        const bubble = ensureBubble()
        if (bubble) {
          bubble.innerHTML = '<span class="text-err">' +
            data.errors.map(escapeHtml).join('<br>') + '</span>'
          bubble.classList.remove('streaming-cursor')
        }
      }
    }

    if (data.num_turns > 0) {
      const stats = document.createElement('div')
      stats.className = 'font-mono text-[11px] text-dim mt-2 pt-2 flex gap-3 flex-wrap'
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
  sendBtn.disabled = false
  inputText.focus()
  scrollToBottom()
}

// ── WebSocket ──
function connect() {
  setStatus('connecting')
  const proto = location.protocol === 'https:' ? 'wss:' : 'ws:'
  const token = localStorage.getItem('orion_api_key')
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
        if (data.session_id) currentSessionId = data.session_id
        startAssistantMessage()
        break
      case 'text_delta':
        appendText(data.text)
        break
      case 'tool_use':
        addToolUse(data.name, data.input)
        break
      case 'tool_result':
        addToolResult(data.content, data.is_error)
        break
      case 'stream_end':
        endStream(data)
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
      : '<span class="text-sm shrink-0">\u{1F4CE}</span>'
    return '<div class="flex items-center gap-1.5 bg-elevated border border-border-main rounded-md px-2 py-1 font-mono text-[11px] text-secondary max-w-[200px] transition-colors hover:border-dim">' +
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
  if ((!text && pendingAttachments.length === 0) || isStreaming || !ws || ws.readyState !== WebSocket.OPEN) return

  addUserMessage(text, pendingAttachments)
  isStreaming = true
  sendBtn.disabled = true

  const payload = { type: 'message', text, channel_id: 'web' }
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
function toggleSidebar() {
  const isOpen = document.body.classList.toggle('sidebar-open')
  sidebarOverlay.classList.toggle('hidden', !isOpen)
  sidebarOverlay.classList.toggle('opacity-100', isOpen)
  sidebarOverlay.classList.toggle('opacity-0', !isOpen)
  if (isOpen) fetchSessions()
}

function closeSidebar() {
  document.body.classList.remove('sidebar-open')
  sidebarOverlay.classList.add('hidden', 'opacity-0')
  sidebarOverlay.classList.remove('opacity-100')
}

const welcomeHTML =
  '<div class="text-center text-dim m-auto py-15 max-w-[420px]" id="welcome">' +
    '<div class="font-mono text-3xl font-extrabold tracking-tighter mb-2 bg-gradient-to-br from-zinc-100 via-accent to-purple bg-clip-text text-transparent">orion</div>' +
    '<p class="text-sm leading-relaxed text-dim">Your personal AI assistant</p>' +
    '<div class="flex gap-4 justify-center mt-6 flex-wrap">' +
      '<div class="flex items-center gap-1.5 font-mono text-xs text-dim px-3 py-1.5 bg-surface border border-border-subtle rounded-lg">' +
        '<kbd class="bg-elevated border border-border-main rounded px-1.5 py-0.5 font-mono text-[11px] text-secondary">Enter</kbd> send</div>' +
      '<div class="flex items-center gap-1.5 font-mono text-xs text-dim px-3 py-1.5 bg-surface border border-border-subtle rounded-lg">' +
        '<kbd class="bg-elevated border border-border-main rounded px-1.5 py-0.5 font-mono text-[11px] text-secondary">Shift+Enter</kbd> newline</div>' +
    '</div>' +
  '</div>'

function newChat() {
  currentSessionId = null
  messages.innerHTML = welcomeHTML
  closeSidebar()
  inputText.focus()
}

menuBtn.addEventListener('click', toggleSidebar)
sidebarClose.addEventListener('click', closeSidebar)
sidebarOverlay.addEventListener('click', closeSidebar)
newChatBtn.addEventListener('click', newChat)

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

function fetchSessions() {
  const token = localStorage.getItem('orion_api_key')
  const headers = {}
  if (token) headers['X-API-Key'] = token

  fetch('/api/sessions?limit=50', { headers })
    .then(r => r.ok ? r.json() : Promise.reject(r.statusText))
    .then(sessions => renderSessions(sessions))
    .catch(() => { sessionList.innerHTML = '<div class="text-center text-dim text-sm py-6 px-3">Failed to load sessions</div>' })
}

function renderSessions(sessions) {
  if (!sessions || sessions.length === 0) {
    sessionList.innerHTML = '<div class="text-center text-dim text-sm py-6 px-3">No conversations yet</div>'
    return
  }

  sessionList.innerHTML = ''
  sessions.forEach(s => {
    const el = document.createElement('div')
    const active = s.id === currentSessionId
    el.className = 'px-3 py-2.5 rounded-md cursor-pointer transition-colors mb-px border border-transparent' +
      (active ? ' bg-accent-muted border-accent/15' : ' hover:bg-elevated')

    const summary = s.title || s.summary || 'Untitled conversation'
    const date = formatSessionDate(s.last_message_at || s.created_at)
    const msgs = s.message_count || 0
    const closed = s.is_closed ? ' \u00b7 ended' : ''

    el.innerHTML =
      '<div class="text-[13px] leading-snug line-clamp-2 break-words' + (active ? ' text-zinc-100' : ' text-secondary') + '">' + escapeHtml(summary) + '</div>' +
      '<div class="font-mono text-[11px] text-dim mt-1 flex gap-2">' +
        '<span>' + date + '</span>' +
        '<span>' + msgs + ' msg' + (msgs !== 1 ? 's' : '') + closed + '</span>' +
      '</div>'

    el.addEventListener('click', () => selectSession(s))
    sessionList.appendChild(el)
  })
}

function selectSession(session) {
  currentSessionId = session.id
  closeSidebar()

  messages.innerHTML = '<div class="text-center text-dim text-sm py-6 px-3">Loading messages...</div>'

  const token = localStorage.getItem('orion_api_key')
  const headers = {}
  if (token) headers['X-API-Key'] = token

  fetch('/api/sessions/' + encodeURIComponent(session.id) + '/messages', { headers })
    .then(r => r.ok ? r.json() : Promise.reject(r.statusText))
    .then(msgs => {
      messages.innerHTML = ''
      if (!msgs || msgs.length === 0) {
        messages.innerHTML = '<div class="text-center text-dim m-auto py-15" id="welcome"><div class="font-mono text-3xl font-extrabold tracking-tighter mb-2 bg-gradient-to-br from-zinc-100 via-accent to-purple bg-clip-text text-transparent">orion</div><p class="text-sm text-dim">No messages in this conversation.</p></div>'
        return
      }

      let currentAssistantMsg = null

      msgs.forEach(m => {
        if (m.role === 'user') {
          currentAssistantMsg = null
          const el = document.createElement('div')
          el.className = 'max-w-[80%] self-end mt-4'
          el.innerHTML = '<div class="bg-accent-strong text-white rounded-[18px] rounded-br-[4px] px-4 py-2.5 leading-relaxed text-sm whitespace-pre-wrap break-words">' + escapeHtml(m.content) + '</div>'
          messages.appendChild(el)
        } else if (m.role === 'assistant') {
          const el = document.createElement('div')
          el.className = 'max-w-full mt-2'
          el.innerHTML = '<div class="py-1 leading-[1.75] text-sm whitespace-pre-wrap break-words text-secondary">' + formatText(m.content) + '</div>'
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
            el.className = 'my-1 rounded-md overflow-hidden border border-border-subtle bg-surface transition-colors hover:border-border-main'
            el.id = id
            el.innerHTML =
              '<div class="flex items-center gap-2 px-2.5 py-1.5 cursor-pointer select-none text-xs text-secondary transition-colors hover:bg-elevated" onclick="toggleTool(\'' + id + '\')">' +
                '<span class="tool-chevron text-[8px] text-dim transition-transform duration-200 shrink-0 w-3 text-center">\u25B6</span>' +
                '<span class="' + toolIconClass(data.name) + ' text-[10px] w-5 h-5 flex items-center justify-center rounded-[5px] shrink-0 font-semibold">' + toolIconSymbol(data.name) + '</span>' +
                '<span class="font-mono font-medium text-[11.5px] text-secondary">' + escapeHtml(data.name) + '</span>' +
                '<span class="text-dim font-mono text-[11px] whitespace-nowrap overflow-hidden text-ellipsis flex-1 min-w-0">' + escapeHtml(preview) + '</span>' +
                '<span class="font-mono text-[10px] px-1.5 py-px rounded-full font-medium shrink-0 tracking-wide bg-ok-muted text-ok">done</span>' +
              '</div>' +
              '<div class="tool-body hidden px-2.5 pb-2.5 text-xs text-secondary">' +
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
                    badge.className = 'font-mono text-[10px] px-1.5 py-px rounded-full font-medium shrink-0 tracking-wide bg-err-muted text-err'
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
      messages.innerHTML = '<div class="text-center text-dim m-auto py-15" id="welcome"><div class="font-mono text-3xl font-extrabold tracking-tighter mb-2 bg-gradient-to-br from-zinc-100 via-accent to-purple bg-clip-text text-transparent">orion</div><p class="text-sm text-dim">Failed to load messages.</p></div>'
    })

  inputText.focus()
}

// ── Keyboard shortcuts ──
document.addEventListener('keydown', (e) => {
  if ((e.metaKey || e.ctrlKey) && e.key === 'k') { e.preventDefault(); inputText.focus() }
  if (e.key === 'Escape' && document.body.classList.contains('sidebar-open')) closeSidebar()
})

// ── Init ──
connect()
