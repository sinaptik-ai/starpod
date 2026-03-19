import React, { useState, useRef, useEffect, useImperativeHandle, forwardRef, useCallback } from 'react'
import { useApp } from '../contexts/AppContext'
import { escapeHtml, toolIconClass, toolIconSymbol, getToolPreview } from '../lib/utils'
import { formatText, formatUserText } from '../lib/markdown'
import { authHeaders } from '../lib/api'
import ToolCard from './ToolCard'
import Welcome from './Welcome'

const Chat = forwardRef(function Chat({ wsRef }, ref) {
  const { state, dispatch } = useApp()
  const { settingsVisible, currentSessionId } = state

  const [messages, setMessages] = useState([])
  const [streamingMessage, setStreamingMessage] = useState(null)

  const scrollRef = useRef(null)
  const streamRef = useRef(null)

  // Keep streamRef in sync
  useEffect(() => {
    streamRef.current = streamingMessage
  }, [streamingMessage])

  const scrollToBottom = useCallback(() => {
    const el = scrollRef.current
    if (el) {
      requestAnimationFrame(() => { el.scrollTop = el.scrollHeight })
    }
  }, [])

  useEffect(() => {
    scrollToBottom()
  }, [messages, streamingMessage, scrollToBottom])

  function handleStreamEvent(data) {
    switch (data.type) {
      case 'stream_start': {
        if (data.session_id) {
          dispatch({ type: 'SET_SESSION', payload: { id: data.session_id, key: null } })
          dispatch({ type: 'MARK_READ', payload: data.session_id })
        }
        setStreamingMessage({ bubbles: [{ text: '', done: false }], tools: [], stats: null })
        break
      }
      case 'text_delta': {
        setStreamingMessage(prev => {
          if (!prev) return prev
          const bubbles = [...prev.bubbles]
          const last = { ...bubbles[bubbles.length - 1] }
          last.text += data.text
          bubbles[bubbles.length - 1] = last
          return { ...prev, bubbles }
        })
        break
      }
      case 'tool_use': {
        setStreamingMessage(prev => {
          if (!prev) return prev
          const bubbles = [...prev.bubbles]
          // Mark current bubble done
          if (bubbles.length > 0) {
            const last = { ...bubbles[bubbles.length - 1] }
            last.done = true
            bubbles[bubbles.length - 1] = last
          }
          const tools = [...prev.tools, {
            id: data.id,
            name: data.name,
            input: data.input,
            status: 'running',
            result: null,
          }]
          // Start new bubble after tool
          bubbles.push({ text: '', done: false })
          return { ...prev, bubbles, tools }
        })
        break
      }
      case 'tool_result': {
        setStreamingMessage(prev => {
          if (!prev) return prev
          const tools = prev.tools.map(t => {
            if (t.id === data.tool_use_id) {
              return {
                ...t,
                status: data.is_error ? 'error' : 'done',
                result: data.content,
              }
            }
            return t
          })
          return { ...prev, tools }
        })
        break
      }
      case 'stream_end': {
        setStreamingMessage(prev => {
          if (!prev) return null

          // Finalize: set all bubbles done, add stats
          const bubbles = prev.bubbles.map(b => ({ ...b, done: true }))
          const tools = [...prev.tools]

          // Build stats
          let stats = null
          if (data.num_turns > 0) {
            const tokensIn = data.input_tokens >= 1000 ? Math.round(data.input_tokens / 1000) + 'k' : data.input_tokens
            const tokensOut = data.output_tokens >= 1000 ? Math.round(data.output_tokens / 1000) + 'k' : data.output_tokens
            stats = {
              numTurns: data.num_turns,
              costUsd: data.cost_usd,
              tokensIn,
              tokensOut,
            }
          }

          // Build errors if present
          let errors = null
          if (data.is_error && data.errors && data.errors.length > 0) {
            const hasText = bubbles.some(b => b.text.trim())
            if (!hasText) {
              errors = data.errors
            }
          }

          // Move to finalized messages
          const finalized = {
            role: 'assistant_stream',
            bubbles: bubbles.filter(b => b.text.trim()),
            tools,
            stats,
            errors,
          }

          setMessages(prev => [...prev, finalized])
          return null
        })

        if (data.session_id || currentSessionId) {
          dispatch({ type: 'MARK_READ', payload: data.session_id || currentSessionId })
        }
        break
      }
      case 'error': {
        // Check if we're currently streaming
        setStreamingMessage(prev => {
          if (prev) {
            // End the stream with an error
            const bubbles = prev.bubbles.map(b => ({ ...b, done: true }))
            const finalized = {
              role: 'assistant_stream',
              bubbles: [],
              tools: [...prev.tools],
              stats: null,
              errors: [data.message],
            }
            setMessages(p => [...p, finalized])
            return null
          } else {
            // Not streaming - show standalone error
            const errMsg = {
              role: 'assistant_stream',
              bubbles: [],
              tools: [],
              stats: null,
              errors: [data.message],
            }
            setMessages(p => [...p, errMsg])
            return null
          }
        })
        break
      }
    }
  }

  function loadSession(sessionId) {
    setMessages([])
    setStreamingMessage(null)

    fetch('/api/sessions/' + encodeURIComponent(sessionId) + '/messages', { headers: authHeaders() })
      .then(r => r.ok ? r.json() : Promise.reject(r.statusText))
      .then(msgs => {
        if (!msgs || msgs.length === 0) {
          setMessages([])
          return
        }

        const parsed = []
        let pendingToolUses = []

        msgs.forEach(m => {
          if (m.role === 'user') {
            parsed.push({ role: 'user', content: m.content, attachments: m.attachments })
          } else if (m.role === 'assistant') {
            // Flush any pending tools into a group with this text
            if (pendingToolUses.length > 0 || m.content) {
              parsed.push({
                role: 'assistant_stream',
                bubbles: m.content ? [{ text: m.content, done: true }] : [],
                tools: pendingToolUses,
                stats: null,
                errors: null,
              })
              pendingToolUses = []
            }
          } else if (m.role === 'tool_use') {
            try {
              const data = JSON.parse(m.content)
              pendingToolUses.push({
                id: data.id || ('hist-' + Math.random().toString(36).slice(2)),
                name: data.name,
                input: data.input || {},
                status: 'done',
                result: null,
              })
            } catch {}
          } else if (m.role === 'tool_result') {
            try {
              const data = JSON.parse(m.content)
              if (pendingToolUses.length > 0) {
                const last = pendingToolUses[pendingToolUses.length - 1]
                if (data.is_error) last.status = 'error'
                if (data.content) last.result = data.content
              }
            } catch {}
          }
        })

        // Flush remaining tools
        if (pendingToolUses.length > 0) {
          parsed.push({
            role: 'assistant_stream',
            bubbles: [],
            tools: pendingToolUses,
            stats: null,
            errors: null,
          })
        }

        setMessages(parsed)
      })
      .catch(() => {
        setMessages([{ role: 'error', content: 'Failed to load messages.' }])
      })
  }

  function showWelcome() {
    setMessages([])
    setStreamingMessage(null)
  }

  function addUserMessage(text, attachments) {
    setMessages(prev => [...prev, { role: 'user', content: text, attachments }])
  }

  useImperativeHandle(ref, () => ({
    handleStreamEvent,
    loadSession,
    showWelcome,
    addUserMessage,
  }))

  function handleSendPrompt(text) {
    if (!wsRef || !wsRef.current) return
    addUserMessage(text, [])
    wsRef.current(text, [])
  }

  // Don't render when settings visible
  if (settingsVisible) return null

  const hasContent = messages.length > 0 || streamingMessage

  return (
    <div
      ref={scrollRef}
      id="messages-scroll"
      className="flex-1 overflow-y-auto"
    >
      <div className="max-w-[740px] mx-auto px-5 py-4 flex flex-col" id="messages">
        {!hasContent && (
          <Welcome onSendPrompt={handleSendPrompt} />
        )}

        {messages.map((msg, idx) => {
          if (msg.role === 'user') {
            return <UserMessage key={idx} msg={msg} />
          }
          if (msg.role === 'assistant_stream') {
            return <AssistantMessage key={idx} msg={msg} />
          }
          if (msg.role === 'error') {
            return (
              <div key={idx} className="flex items-center justify-center text-center" style={{ minHeight: 'calc(100dvh - 120px)' }}>
                <div>
                  <div className="font-mono text-3xl font-extrabold tracking-tighter mb-2 bg-gradient-to-b from-primary to-muted bg-clip-text text-transparent">starpod</div>
                  <p className="text-sm text-muted">{msg.content}</p>
                </div>
              </div>
            )
          }
          return null
        })}

        {streamingMessage && (
          <StreamingMessage msg={streamingMessage} />
        )}
      </div>
    </div>
  )
})

function UserMessage({ msg }) {
  const { content, attachments } = msg
  const html = content ? formatUserText(content) : ''

  return (
    <div className="max-w-[80%] self-end mt-4" style={{ animation: 'msg-in 0.25s cubic-bezier(0.16, 1, 0.3, 1)' }}>
      {attachments && attachments.length > 0 && (
        <div className="flex flex-wrap gap-1.5 mb-1.5 justify-end">
          {attachments.map((att, i) => {
            if (att.mime_type && att.mime_type.startsWith('image/')) {
              return (
                <img
                  key={i}
                  src={`data:${att.mime_type};base64,${att.data}`}
                  className="max-w-[200px] max-h-[200px] rounded-xl object-cover"
                  alt={att.file_name}
                />
              )
            }
            return (
              <div key={i} className="bg-elevated px-2.5 py-1.5 rounded-lg font-mono text-[11px] text-muted border border-border-subtle">
                {att.file_name}
              </div>
            )
          })}
        </div>
      )}
      {content && (
        <div
          className="bg-accent text-white rounded-2xl rounded-br-md px-4 py-2.5 leading-relaxed text-sm whitespace-pre-wrap break-words"
          dangerouslySetInnerHTML={{ __html: html }}
        />
      )}
    </div>
  )
}

function AssistantMessage({ msg }) {
  const { bubbles, tools, stats, errors } = msg

  // Interleave bubbles and tools in order
  // Layout: bubble[0], tool[0], bubble[1], tool[1], ...
  const elements = []
  const maxLen = Math.max(bubbles.length, tools.length)

  for (let i = 0; i < maxLen; i++) {
    if (i < bubbles.length && bubbles[i].text.trim()) {
      elements.push(
        <div
          key={`bubble-${i}`}
          className="py-1 leading-[1.75] text-sm break-words text-secondary markdown-body"
          dangerouslySetInnerHTML={{ __html: formatText(bubbles[i].text) }}
        />
      )
    }
    if (i < tools.length) {
      elements.push(
        <ToolCard
          key={`tool-${tools[i].id}`}
          id={'tool-' + tools[i].id}
          name={tools[i].name}
          input={tools[i].input}
          status={tools[i].status}
          result={tools[i].result}
        />
      )
    }
  }

  // Remaining bubbles after tools
  for (let i = tools.length; i < bubbles.length; i++) {
    if (bubbles[i].text.trim()) {
      elements.push(
        <div
          key={`bubble-${i}`}
          className="py-1 leading-[1.75] text-sm break-words text-secondary markdown-body"
          dangerouslySetInnerHTML={{ __html: formatText(bubbles[i].text) }}
        />
      )
    }
  }

  if (errors && errors.length > 0) {
    elements.push(
      <div key="errors" className="py-1 leading-[1.75] text-sm break-words text-secondary markdown-body">
        <span className="text-err">{errors.join('\n')}</span>
      </div>
    )
  }

  return (
    <div className="max-w-full mt-2" style={{ animation: 'msg-in 0.25s cubic-bezier(0.16, 1, 0.3, 1)' }}>
      {elements}
      {stats && (
        <div className="font-mono text-[11px] text-dim mt-2 pt-2 border-t border-border-subtle flex gap-3 flex-wrap">
          <span>{stats.numTurns} turn{stats.numTurns > 1 ? 's' : ''}</span>
          <span>${stats.costUsd.toFixed(4)}</span>
          <span>{stats.tokensIn} in {'\u00b7'} {stats.tokensOut} out</span>
        </div>
      )}
    </div>
  )
}

function StreamingMessage({ msg }) {
  const { bubbles, tools } = msg

  const elements = []
  const maxLen = Math.max(bubbles.length, tools.length)

  for (let i = 0; i < maxLen; i++) {
    if (i < bubbles.length) {
      const bubble = bubbles[i]
      const isActive = !bubble.done
      const hasText = bubble.text.trim()

      if (hasText || isActive) {
        elements.push(
          <div
            key={`bubble-${i}`}
            className={`py-1 leading-[1.75] text-sm break-words text-secondary markdown-body${isActive ? ' streaming-cursor' : ''}`}
            dangerouslySetInnerHTML={{ __html: bubble.text ? formatText(bubble.text) : '' }}
          />
        )
      }
    }
    if (i < tools.length) {
      elements.push(
        <ToolCard
          key={`tool-${tools[i].id}`}
          id={'tool-' + tools[i].id}
          name={tools[i].name}
          input={tools[i].input}
          status={tools[i].status}
          result={tools[i].result}
        />
      )
    }
  }

  // Remaining bubbles after tools
  for (let i = tools.length; i < bubbles.length; i++) {
    const bubble = bubbles[i]
    const isActive = !bubble.done
    const hasText = bubble.text.trim()

    if (hasText || isActive) {
      elements.push(
        <div
          key={`bubble-${i}`}
          className={`py-1 leading-[1.75] text-sm break-words text-secondary markdown-body${isActive ? ' streaming-cursor' : ''}`}
          dangerouslySetInnerHTML={{ __html: bubble.text ? formatText(bubble.text) : '' }}
        />
      )
    }
  }

  return (
    <div className="max-w-full mt-2" style={{ animation: 'msg-in 0.25s cubic-bezier(0.16, 1, 0.3, 1)' }}>
      {elements}
    </div>
  )
}

export default Chat
