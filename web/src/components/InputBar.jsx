import React, { useState, useRef, useEffect, useCallback } from 'react'
import { PaperclipIcon, SendIcon } from './ui/Icons'

const MAX_FILE_SIZE = 20 * 1024 * 1024

function readFileAsBase64(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => { resolve(reader.result.split(',')[1]) }
    reader.onerror = reject
    reader.readAsDataURL(file)
  })
}

function InputBar({ onSend, disabled }) {
  const [pendingAttachments, setPendingAttachments] = useState([])
  const textareaRef = useRef(null)
  const fileInputRef = useRef(null)
  const dragCounterRef = useRef(0)

  function autoResize() {
    const el = textareaRef.current
    if (!el) return
    el.style.height = 'auto'
    el.style.height = Math.min(el.scrollHeight, 160) + 'px'
  }

  const addFiles = useCallback(async (files) => {
    const newAttachments = []
    for (const file of files) {
      if (file.size > MAX_FILE_SIZE) {
        alert(`"${file.name}" is too large (${(file.size / 1048576).toFixed(1)} MB). Maximum file size is 20 MB.`)
        continue
      }
      const base64 = await readFileAsBase64(file)
      newAttachments.push({
        file_name: file.name,
        mime_type: file.type || 'application/octet-stream',
        data: base64,
      })
    }
    if (newAttachments.length > 0) {
      setPendingAttachments(prev => [...prev, ...newAttachments])
    }
  }, [])

  function removeAttachment(index) {
    setPendingAttachments(prev => prev.filter((_, i) => i !== index))
  }

  function handleSubmit(e) {
    e.preventDefault()
    const text = textareaRef.current ? textareaRef.current.value.trim() : ''
    if (!text && pendingAttachments.length === 0) return
    if (disabled) return

    onSend(text, pendingAttachments)

    if (textareaRef.current) {
      textareaRef.current.value = ''
      textareaRef.current.style.height = 'auto'
    }
    setPendingAttachments([])
  }

  function handleKeyDown(e) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault()
      handleSubmit(e)
    }
  }

  // Drag & drop on #app element
  useEffect(() => {
    const app = document.getElementById('app')
    if (!app) return

    function handleDragEnter(e) {
      e.preventDefault()
      dragCounterRef.current++
      app.classList.add('drag-over')
    }

    function handleDragOver(e) {
      e.preventDefault()
    }

    function handleDragLeave(e) {
      e.preventDefault()
      dragCounterRef.current--
      if (dragCounterRef.current <= 0) {
        dragCounterRef.current = 0
        app.classList.remove('drag-over')
      }
    }

    function handleDrop(e) {
      e.preventDefault()
      dragCounterRef.current = 0
      app.classList.remove('drag-over')
      if (e.dataTransfer.files.length > 0) {
        addFiles(e.dataTransfer.files)
      }
    }

    app.addEventListener('dragenter', handleDragEnter)
    app.addEventListener('dragover', handleDragOver)
    app.addEventListener('dragleave', handleDragLeave)
    app.addEventListener('drop', handleDrop)

    return () => {
      app.removeEventListener('dragenter', handleDragEnter)
      app.removeEventListener('dragover', handleDragOver)
      app.removeEventListener('dragleave', handleDragLeave)
      app.removeEventListener('drop', handleDrop)
    }
  }, [addFiles])

  return (
    <div className="shrink-0 pt-2 pb-4 w-full input-bar-safe">
      <div className="max-w-[740px] mx-auto px-5">
        {/* Attachment preview */}
        {pendingAttachments.length > 0 && (
          <div className="flex flex-wrap gap-1.5 mb-2">
            {pendingAttachments.map((att, i) => {
              const isImage = att.mime_type.startsWith('image/')
              return (
                <div
                  key={i}
                  className="flex items-center gap-1.5 bg-elevated border border-border-main rounded-lg px-2 py-1 font-mono text-[11px] text-secondary max-w-[200px] transition-colors hover:border-dim"
                >
                  {isImage ? (
                    <img
                      src={`data:${att.mime_type};base64,${att.data}`}
                      className="w-7 h-7 object-cover rounded shrink-0"
                      alt={att.file_name}
                    />
                  ) : (
                    <span className="text-sm shrink-0 text-dim">{'\u{1F4CE}'}</span>
                  )}
                  <span className="overflow-hidden text-ellipsis whitespace-nowrap flex-1 min-w-0">
                    {att.file_name}
                  </span>
                  <button
                    className="bg-transparent border-none text-dim cursor-pointer text-sm leading-none px-0.5 shrink-0 hover:text-err transition-colors"
                    onClick={() => removeAttachment(i)}
                    aria-label="Remove attachment"
                  >
                    &times;
                  </button>
                </div>
              )
            })}
          </div>
        )}

        {/* Input form */}
        <form
          id="input-form"
          className="flex gap-2 items-center bg-input-bg border border-border-subtle rounded-lg pl-3 pr-1.5 py-1.5"
          onSubmit={handleSubmit}
        >
          <input
            type="file"
            ref={fileInputRef}
            className="hidden"
            multiple
            onChange={(e) => {
              if (e.target.files.length > 0) addFiles(e.target.files)
              e.target.value = ''
            }}
          />
          <button
            type="button"
            className="text-dim hover:text-secondary transition-colors shrink-0 cursor-pointer p-1"
            onClick={() => fileInputRef.current && fileInputRef.current.click()}
            title="Attach file"
            aria-label="Attach file"
          >
            <PaperclipIcon />
          </button>
          <span className="text-dim text-xs font-mono shrink-0 select-none">&gt;</span>
          <textarea
            ref={textareaRef}
            className="flex-1 bg-transparent text-primary text-[13px] font-mono resize-none outline-none placeholder:text-dim leading-relaxed max-h-40"
            rows={1}
            placeholder="..."
            onInput={autoResize}
            onKeyDown={handleKeyDown}
            disabled={disabled}
            aria-label="Message"
          />
          <button
            type="submit"
            className="text-dim hover:text-secondary transition-colors shrink-0 cursor-pointer p-1.5 disabled:opacity-20 disabled:cursor-default"
            disabled={disabled}
            aria-label="Send message"
          >
            <SendIcon />
          </button>
        </form>
      </div>
    </div>
  )
}

export default InputBar
