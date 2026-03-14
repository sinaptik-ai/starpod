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

/** Pending attachments as {file_name, mime_type, data (base64)} */
let pendingAttachments = []

const MAX_FILE_SIZE = 20 * 1024 * 1024 // 20 MB

// ── Helpers ──
function setStatus(state) {
  statusDot.className = 'dot ' + state
  statusText.textContent = state === 'connected' ? 'Connected' :
                           state === 'connecting' ? 'Connecting' : 'Disconnected'
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
  html = html.replace(/```(\w*)\n([\s\S]*?)```/g, (_, _lang, code) => '<pre>' + code + '</pre>')
  html = html.replace(/`([^`]+)`/g, '<code>$1</code>')
  html = html.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
  html = html.replace(/(?<!\*)\*([^*]+)\*(?!\*)/g, '<em>$1</em>')
  html = html.replace(/(?<![="'])(https?:\/\/[^\s<>"')\]]+)/g, '<a href="$1" target="_blank" rel="noopener">$1</a>')
  return html
}

// ── Tool helpers ──
function toolIconClass(name) {
  const n = name.toLowerCase()
  if (n === 'read') return 'read'
  if (n === 'write') return 'write'
  if (n === 'edit') return 'edit'
  if (n === 'bash') return 'bash'
  if (n === 'grep') return 'grep'
  if (n === 'glob') return 'glob'
  if (n.includes('search')) return 'search'
  return 'default'
}

function toolIconSymbol(name) {
  const n = name.toLowerCase()
  if (n === 'read') return '\u25B7'
  if (n === 'write' || n === 'edit') return '\u270E'
  if (n === 'bash') return '\u25B8'
  if (n === 'grep' || n === 'glob') return '\u2315'
  if (n.includes('memory')) return '\u25C7'
  if (n.includes('vault')) return '\u2609'
  return '\u2022'
}

function getToolPreview(name, input) {
  if (input.file_path) return input.file_path
  if (input.pattern) return input.pattern
  if (input.command) return input.command.length > 50 ? input.command.slice(0, 50) + '...' : input.command
  if (input.query) return input.query.length > 50 ? input.query.slice(0, 50) + '...' : input.query
  if (input.key) return input.key
  if (input.file) return input.file
  if (input.name) return input.name
  return ''
}

// ── Tool toggle (needs to be global for onclick in innerHTML) ──
window.toggleTool = function(id) {
  const el = document.getElementById(id)
  if (el) el.classList.toggle('expanded')
}

// ── Messages ──
function addUserMessage(text, atts) {
  const welcome = messages.querySelector('.welcome')
  if (welcome) welcome.remove()

  const msg = document.createElement('div')
  msg.className = 'msg user'

  let html = ''
  if (atts && atts.length > 0) {
    html += '<div class="user-attachments">'
    for (const att of atts) {
      if (att.mime_type.startsWith('image/')) {
        html += '<img src="data:' + att.mime_type + ';base64,' + att.data + '" class="user-att-img" alt="' + escapeHtml(att.file_name) + '">'
      } else {
        html += '<div class="user-att-file">\u{1F4CE} ' + escapeHtml(att.file_name) + '</div>'
      }
    }
    html += '</div>'
  }
  if (text) html += '<div class="bubble">' + escapeHtml(text) + '</div>'
  msg.innerHTML = html
  messages.appendChild(msg)
  scrollToBottom()
}

function startAssistantMessage() {
  const msg = document.createElement('div')
  msg.className = 'msg assistant'
  currentMsg = msg
  currentBubble = null
  messages.appendChild(msg)
  scrollToBottom()
}

function ensureBubble() {
  if (currentBubble) return currentBubble
  if (!currentMsg) return null
  const bubble = document.createElement('div')
  bubble.className = 'bubble streaming-cursor'
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
  if (currentBubble) {
    currentBubble.classList.remove('streaming-cursor')
    currentBubble = null
  }

  const id = 'tool-' + (toolCounter++)
  const preview = getToolPreview(name, input)
  const inputJson = JSON.stringify(input, null, 2)

  const el = document.createElement('div')
  el.className = 'tool-item'
  el.id = id
  el.innerHTML =
    '<div class="tool-header" onclick="toggleTool(\'' + id + '\')">' +
      '<span class="tool-chevron">\u25B6</span>' +
      '<span class="tool-icon ' + toolIconClass(name) + '">' + toolIconSymbol(name) + '</span>' +
      '<span class="tool-name">' + escapeHtml(name) + '</span>' +
      '<span class="tool-preview">' + escapeHtml(preview) + '</span>' +
      '<span class="tool-badge running">running</span>' +
    '</div>' +
    '<div class="tool-body">' +
      '<div class="tool-section">' +
        '<div class="tool-label">Input</div>' +
        '<pre>' + escapeHtml(inputJson) + '</pre>' +
      '</div>' +
      '<div class="tool-section tool-result-section" style="display:none">' +
        '<div class="tool-label">Result</div>' +
        '<pre class="tool-result-pre"></pre>' +
      '</div>' +
    '</div>'

  currentMsg.appendChild(el)
  scrollToBottom()
}

function addToolResult(content, isError) {
  if (!currentMsg) return
  const tools = currentMsg.querySelectorAll('.tool-item')
  const last = tools[tools.length - 1]
  if (!last) return

  const badge = last.querySelector('.tool-badge')
  if (isError) {
    badge.textContent = 'error'
    badge.className = 'tool-badge err'
  } else {
    badge.textContent = 'done'
    badge.className = 'tool-badge ok'
  }

  const resultSection = last.querySelector('.tool-result-section')
  const resultPre = last.querySelector('.tool-result-pre')
  if (resultSection && resultPre && content) {
    resultPre.textContent = content
    resultSection.style.display = ''
  }

  scrollToBottom()
}

function endStream(data) {
  if (currentMsg) {
    currentMsg.querySelectorAll('.bubble').forEach(b => {
      b.classList.remove('streaming-cursor')
      if (b._rawText) b.innerHTML = formatText(b._rawText)
    })

    if (data.is_error && data.errors && data.errors.length > 0) {
      const hasText = Array.from(currentMsg.querySelectorAll('.bubble')).some(b => b._rawText)
      if (!hasText) {
        const bubble = ensureBubble()
        if (bubble) {
          bubble.innerHTML = '<span style="color:var(--red)">' +
            data.errors.map(escapeHtml).join('<br>') + '</span>'
          bubble.classList.remove('streaming-cursor')
        }
      }
    }

    if (data.num_turns > 0) {
      const stats = document.createElement('div')
      stats.className = 'stats'
      const tokens_in = data.input_tokens >= 1000 ? Math.round(data.input_tokens / 1000) + 'k' : data.input_tokens
      const tokens_out = data.output_tokens >= 1000 ? Math.round(data.output_tokens / 1000) + 'k' : data.output_tokens
      stats.innerHTML =
        '<span>' + data.num_turns + ' turn' + (data.num_turns > 1 ? 's' : '') + '</span>' +
        '<span>$' + data.cost_usd.toFixed(4) + '</span>' +
        '<span>' + tokens_in + ' in / ' + tokens_out + ' out</span>'
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
            bubble.innerHTML = '<span style="color:var(--red)">' + escapeHtml(data.message) + '</span>'
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
    reader.onload = () => {
      // result is "data:<mime>;base64,<data>" — extract the base64 part
      const base64 = reader.result.split(',')[1]
      resolve(base64)
    }
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

// Expose for onclick in innerHTML
window.removeAttachment = removeAttachment

function renderAttachmentPreview() {
  if (pendingAttachments.length === 0) {
    attachmentPreview.innerHTML = ''
    attachmentPreview.style.display = 'none'
    return
  }
  attachmentPreview.style.display = 'flex'
  attachmentPreview.innerHTML = pendingAttachments.map((att, i) => {
    const isImage = att.mime_type.startsWith('image/')
    const thumb = isImage
      ? '<img src="data:' + att.mime_type + ';base64,' + att.data + '" class="att-thumb">'
      : '<span class="att-icon">\u{1F4CE}</span>'
    return '<div class="att-chip">' +
      thumb +
      '<span class="att-name">' + escapeHtml(att.file_name) + '</span>' +
      '<button class="att-remove" onclick="removeAttachment(' + i + ')">&times;</button>' +
    '</div>'
  }).join('')
}

attachBtn.addEventListener('click', () => fileInput.click())
fileInput.addEventListener('change', () => {
  if (fileInput.files.length > 0) addFiles(fileInput.files)
  fileInput.value = '' // reset so same file can be re-selected
})

// ── Drag & drop ──
const app = document.getElementById('app')
app.addEventListener('dragover', (e) => { e.preventDefault(); app.classList.add('drag-over') })
app.addEventListener('dragleave', () => app.classList.remove('drag-over'))
app.addEventListener('drop', (e) => {
  e.preventDefault()
  app.classList.remove('drag-over')
  if (e.dataTransfer.files.length > 0) addFiles(e.dataTransfer.files)
})

// ── Send ──
function sendMessage() {
  const text = inputText.value.trim()
  if ((!text && pendingAttachments.length === 0) || isStreaming || !ws || ws.readyState !== WebSocket.OPEN) return

  addUserMessage(text, pendingAttachments)
  isStreaming = true
  sendBtn.disabled = true

  const payload = { type: 'message', text, channel_id: 'web' }
  if (pendingAttachments.length > 0) {
    payload.attachments = pendingAttachments
  }
  ws.send(JSON.stringify(payload))

  inputText.value = ''
  pendingAttachments = []
  renderAttachmentPreview()
  autoResize()
}

inputForm.addEventListener('submit', (e) => { e.preventDefault(); sendMessage() })
inputText.addEventListener('keydown', (e) => {
  if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); sendMessage() }
})
inputText.addEventListener('input', autoResize)

// ── Sidebar ──
function toggleSidebar() {
  const isOpen = document.body.classList.toggle('sidebar-open')
  if (isOpen) fetchSessions()
}

function closeSidebar() {
  document.body.classList.remove('sidebar-open')
}

function newChat() {
  currentSessionId = null
  messages.innerHTML = '<div class="welcome"><h2>Orion</h2><p>Your personal AI assistant. Ask anything.</p></div>'
  closeSidebar()
  inputText.focus()
}

menuBtn.addEventListener('click', toggleSidebar)
sidebarClose.addEventListener('click', closeSidebar)
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
    .catch(() => {
      sessionList.innerHTML = '<div class="session-empty">Failed to load sessions</div>'
    })
}

function renderSessions(sessions) {
  if (!sessions || sessions.length === 0) {
    sessionList.innerHTML = '<div class="session-empty">No conversations yet</div>'
    return
  }

  sessionList.innerHTML = ''
  sessions.forEach(s => {
    const el = document.createElement('div')
    el.className = 'session-item' + (s.id === currentSessionId ? ' active' : '')

    const summary = s.title || s.summary || 'Untitled conversation'
    const date = formatSessionDate(s.last_message_at || s.created_at)
    const msgs = s.message_count || 0
    const closed = s.is_closed ? ' \u00b7 ended' : ''

    el.innerHTML =
      '<div class="session-summary">' + escapeHtml(summary) + '</div>' +
      '<div class="session-meta">' +
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

  messages.innerHTML = '<div class="session-empty">Loading messages...</div>'

  const token = localStorage.getItem('orion_api_key')
  const headers = {}
  if (token) headers['X-API-Key'] = token

  fetch('/api/sessions/' + encodeURIComponent(session.id) + '/messages', { headers })
    .then(r => r.ok ? r.json() : Promise.reject(r.statusText))
    .then(msgs => {
      messages.innerHTML = ''
      if (!msgs || msgs.length === 0) {
        messages.innerHTML = '<div class="welcome"><h2>No messages</h2><p>This conversation has no stored messages.</p></div>'
        return
      }

      let currentAssistantMsg = null

      msgs.forEach(m => {
        if (m.role === 'user') {
          currentAssistantMsg = null
          const el = document.createElement('div')
          el.className = 'msg user'
          el.innerHTML = '<div class="bubble">' + escapeHtml(m.content) + '</div>'
          messages.appendChild(el)
        } else if (m.role === 'assistant') {
          const el = document.createElement('div')
          el.className = 'msg assistant'
          el.innerHTML = '<div class="bubble">' + formatText(m.content) + '</div>'
          messages.appendChild(el)
          currentAssistantMsg = el
        } else if (m.role === 'tool_use') {
          if (!currentAssistantMsg) {
            currentAssistantMsg = document.createElement('div')
            currentAssistantMsg.className = 'msg assistant'
            messages.appendChild(currentAssistantMsg)
          }
          try {
            const data = JSON.parse(m.content)
            const id = 'hist-tool-' + (toolCounter++)
            const preview = getToolPreview(data.name, data.input || {})
            const inputJson = JSON.stringify(data.input || {}, null, 2)

            const el = document.createElement('div')
            el.className = 'tool-item'
            el.id = id
            el.innerHTML =
              '<div class="tool-header" onclick="toggleTool(\'' + id + '\')">' +
                '<span class="tool-chevron">\u25B6</span>' +
                '<span class="tool-icon ' + toolIconClass(data.name) + '">' + toolIconSymbol(data.name) + '</span>' +
                '<span class="tool-name">' + escapeHtml(data.name) + '</span>' +
                '<span class="tool-preview">' + escapeHtml(preview) + '</span>' +
                '<span class="tool-badge ok">done</span>' +
              '</div>' +
              '<div class="tool-body">' +
                '<div class="tool-section">' +
                  '<div class="tool-label">Input</div>' +
                  '<pre>' + escapeHtml(inputJson) + '</pre>' +
                '</div>' +
              '</div>'
            currentAssistantMsg.appendChild(el)
          } catch { /* ignore malformed tool_use */ }
        } else if (m.role === 'tool_result') {
          if (currentAssistantMsg) {
            const lastTool = currentAssistantMsg.querySelector('.tool-item:last-child')
            if (lastTool) {
              try {
                const data = JSON.parse(m.content)
                if (data.is_error) {
                  const badge = lastTool.querySelector('.tool-badge')
                  if (badge) { badge.textContent = 'error'; badge.className = 'tool-badge err' }
                }
                if (data.content) {
                  const body = lastTool.querySelector('.tool-body')
                  if (body) {
                    const section = document.createElement('div')
                    section.className = 'tool-section'
                    section.innerHTML =
                      '<div class="tool-label">Result</div>' +
                      '<pre>' + escapeHtml(data.content) + '</pre>'
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
      messages.innerHTML = '<div class="welcome"><h2>Error</h2><p>Failed to load messages.</p></div>'
    })

  inputText.focus()
}

// ── Init ──
connect()
