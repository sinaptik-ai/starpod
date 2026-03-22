import React, { useState, useRef, useEffect, useCallback } from 'react'
import { PaperclipIcon } from './ui/Icons'
import SlashMenu, { filterSkills } from './SlashMenu'
import { fetchSkills } from '../lib/api'

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
  const [slashOpen, setSlashOpen] = useState(false)
  const [slashFilter, setSlashFilter] = useState('')
  const [slashSkills, setSlashSkills] = useState([])
  const [slashIndex, setSlashIndex] = useState(0)
  const textareaRef = useRef(null)
  const fileInputRef = useRef(null)
  const dragCounterRef = useRef(0)

  useEffect(() => {
    if (!disabled && textareaRef.current) textareaRef.current.focus()
  }, [disabled])

  function autoResize() {
    const el = textareaRef.current
    if (!el) return
    el.style.height = 'auto'
    el.style.height = Math.min(el.scrollHeight, 160) + 'px'
  }

  async function handleSlashDetect() {
    const val = textareaRef.current?.value || ''
    if (val.startsWith('/') && !val.includes(' ')) {
      const query = val.slice(1)
      if (!slashOpen) {
        const skills = await fetchSkills()
        setSlashSkills(skills)
      }
      setSlashFilter(query)
      setSlashOpen(true)
      setSlashIndex(0)
    } else {
      setSlashOpen(false)
    }
  }

  function selectSkill(skill) {
    if (textareaRef.current) {
      textareaRef.current.value = `/${skill.name} `
      autoResize()
    }
    setSlashOpen(false)
    textareaRef.current?.focus()
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
    if (slashOpen) {
      const filtered = filterSkills(slashSkills, slashFilter)
      if (e.key === 'ArrowDown') {
        e.preventDefault()
        setSlashIndex(i => (i + 1) % Math.max(filtered.length, 1))
        return
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault()
        setSlashIndex(i => (i - 1 + filtered.length) % Math.max(filtered.length, 1))
        return
      }
      if ((e.key === 'Enter' || e.key === 'Tab') && filtered.length > 0) {
        e.preventDefault()
        selectSkill(filtered[slashIndex])
        return
      }
      if (e.key === 'Escape') {
        e.preventDefault()
        setSlashOpen(false)
        return
      }
    }
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

  const cfg = window.__STARPOD__ || {}
  const agentName = cfg.agent_name || 'Starpod'
  const hasContent = textareaRef.current?.value?.trim() || pendingAttachments.length > 0

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
        <div className="relative">
          {slashOpen && (
            <SlashMenu
              skills={slashSkills}
              filter={slashFilter}
              activeIndex={slashIndex}
              onSelect={selectSkill}
              onHover={setSlashIndex}
            />
          )}
        <form
          id="input-form"
          className="input-form-styled"
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
            className="text-dim hover:text-secondary transition-colors shrink-0 cursor-pointer p-1.5"
            onClick={() => fileInputRef.current && fileInputRef.current.click()}
            title="Attach file"
            aria-label="Attach file"
          >
            <PaperclipIcon />
          </button>
          <textarea
            ref={textareaRef}
            className="flex-1 bg-transparent text-primary text-[13px] resize-none outline-none placeholder:text-dim leading-relaxed max-h-40"
            rows={1}
            placeholder={`Message ${agentName}...`}
            onInput={() => { autoResize(); handleSlashDetect() }}
            onKeyDown={handleKeyDown}
            disabled={disabled}
            autoFocus
            aria-label="Message"
          />
          <button
            type="submit"
            className="send-btn"
            disabled={disabled}
            aria-label="Send message"
          >
            <svg className="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M12 19V5M5 12l7-7 7 7" />
            </svg>
          </button>
        </form>
        </div>
      </div>
    </div>
  )
}

export default InputBar
