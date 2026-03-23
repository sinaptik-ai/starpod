import React, { useState, useEffect, useCallback, useMemo } from 'react'
import { useApp } from '../contexts/AppContext'
import { apiHeaders } from '../lib/api'
import { formatText } from '../lib/markdown'

function formatSize(bytes) {
  if (bytes === 0) return '—'
  if (bytes < 1024) return bytes + ' B'
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB'
  return (bytes / (1024 * 1024)).toFixed(1) + ' MB'
}

function getFileType(name) {
  const ext = name.split('.').pop().toLowerCase()
  if (ext === 'md') return 'markdown'
  if (ext === 'csv') return 'csv'
  if (ext === 'json') return 'json'
  if (ext === 'html' || ext === 'htm') return 'html'
  if (ext === 'pdf') return 'pdf'
  if (['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg', 'ico', 'bmp'].includes(ext)) return 'image'
  if (['mp4', 'webm', 'ogg', 'mov'].includes(ext)) return 'video'
  if (['mp3', 'wav', 'flac', 'aac', 'ogg'].includes(ext)) return 'audio'
  return 'text'
}

function isTextFile(name) {
  const ext = name.split('.').pop().toLowerCase()
  const textExts = ['txt', 'md', 'json', 'toml', 'yaml', 'yml', 'csv', 'xml', 'html', 'htm', 'css', 'js', 'ts', 'jsx', 'tsx', 'rs', 'py', 'rb', 'go', 'sh', 'bash', 'zsh', 'fish', 'conf', 'cfg', 'ini', 'env', 'log', 'sql', 'graphql', 'svg', 'lock', 'gitignore', 'dockerignore', 'editorconfig', 'prettierrc']
  return textExts.includes(ext) || !name.includes('.')
}

function isBinaryPreview(name) {
  const t = getFileType(name)
  return t === 'image' || t === 'pdf' || t === 'video' || t === 'audio'
}

function hasRichPreview(name) {
  const t = getFileType(name)
  return ['markdown', 'csv', 'json', 'html', 'image', 'pdf', 'video', 'audio'].includes(t)
}

// ── CSV parser (handles quoted fields) ──────────────────────────────────

function parseCsv(text) {
  const rows = []
  let i = 0
  while (i < text.length) {
    const row = []
    while (i < text.length) {
      if (text[i] === '"') {
        // Quoted field
        i++
        let field = ''
        while (i < text.length) {
          if (text[i] === '"') {
            if (text[i + 1] === '"') { field += '"'; i += 2 }
            else { i++; break }
          } else { field += text[i]; i++ }
        }
        row.push(field)
      } else {
        // Unquoted field
        let field = ''
        while (i < text.length && text[i] !== ',' && text[i] !== '\n' && text[i] !== '\r') {
          field += text[i]; i++
        }
        row.push(field)
      }
      if (i < text.length && text[i] === ',') { i++; continue }
      if (i < text.length && text[i] === '\r') i++
      if (i < text.length && text[i] === '\n') i++
      break
    }
    if (row.length > 0 && !(row.length === 1 && row[0] === '')) rows.push(row)
  }
  return rows
}

// ── JSON syntax coloring ────────────────────────────────────────────────

function colorizeJson(text) {
  try {
    const obj = JSON.parse(text)
    const pretty = JSON.stringify(obj, null, 2)
    // Tokenize: strings, numbers, booleans, null, keys
    return pretty.replace(
      /("(?:\\.|[^"\\])*")\s*:|("(?:\\.|[^"\\])*")|(\b(?:true|false)\b)|(\bnull\b)|(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)/g,
      (match, key, str, bool, nul, num) => {
        if (key) return `<span class="json-key">${key}</span>:`
        if (str) return `<span class="json-str">${str}</span>`
        if (bool) return `<span class="json-bool">${match}</span>`
        if (nul) return `<span class="json-null">${match}</span>`
        if (num) return `<span class="json-num">${match}</span>`
        return match
      }
    )
  } catch {
    return null
  }
}

// ── Preview renderers ───────────────────────────────────────────────────

function MarkdownPreview({ content }) {
  const html = useMemo(() => formatText(content), [content])
  return <div className="markdown-body" dangerouslySetInnerHTML={{ __html: html }} />
}

function CsvPreview({ content }) {
  const rows = useMemo(() => parseCsv(content), [content])
  if (rows.length === 0) return <div className="text-dim font-mono text-xs">Empty CSV</div>
  const header = rows[0]
  const body = rows.slice(1)
  return (
    <div className="overflow-auto">
      <table className="w-full text-[13px] font-mono border-collapse">
        <thead>
          <tr>
            {header.map((cell, i) => (
              <th key={i} className="text-left px-3 py-2 border-b border-border-main text-primary font-semibold bg-surface sticky top-0">{cell}</th>
            ))}
          </tr>
        </thead>
        <tbody>
          {body.map((row, ri) => (
            <tr key={ri} className="hover:bg-elevated/50 transition-colors">
              {row.map((cell, ci) => (
                <td key={ci} className="px-3 py-1.5 border-b border-border-subtle text-secondary whitespace-nowrap">{cell}</td>
              ))}
              {/* Pad short rows */}
              {row.length < header.length && Array.from({ length: header.length - row.length }, (_, k) => (
                <td key={`pad-${k}`} className="px-3 py-1.5 border-b border-border-subtle" />
              ))}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

function JsonPreview({ content }) {
  const html = useMemo(() => colorizeJson(content), [content])
  if (!html) return <pre className="text-[13px] font-mono text-secondary leading-relaxed whitespace-pre-wrap break-words">{content}</pre>
  return <pre className="text-[13px] font-mono leading-relaxed whitespace-pre-wrap break-words" dangerouslySetInnerHTML={{ __html: html }} />
}

function HtmlPreview({ content }) {
  return (
    <iframe
      srcDoc={content}
      sandbox="allow-scripts"
      className="w-full h-full border-0 rounded-lg bg-white"
      title="HTML preview"
    />
  )
}

// Hook to fetch a binary file via the raw endpoint (with auth headers) and
// return a blob URL that can be used in <img>, <iframe>, <video>, <audio>.
function useBlobUrl(path) {
  const [url, setUrl] = useState(null)
  useEffect(() => {
    let revoked = false
    const headers = {}
    const token = localStorage.getItem('starpod_api_key')
    if (token) headers['X-API-Key'] = token

    fetch(`/api/files/raw?path=${encodeURIComponent(path)}`, { headers })
      .then(r => r.ok ? r.blob() : null)
      .then(blob => {
        if (blob && !revoked) setUrl(URL.createObjectURL(blob))
      })
      .catch(() => {})
    return () => { revoked = true; if (url) URL.revokeObjectURL(url) }
  }, [path]) // eslint-disable-line react-hooks/exhaustive-deps
  return url
}

function ImagePreview({ path }) {
  const src = useBlobUrl(path)
  if (!src) return <div className="flex items-center justify-center h-full text-dim font-mono text-xs">Loading...</div>
  return (
    <div className="flex items-center justify-center h-full">
      <img src={src} alt={path.split('/').pop()} className="max-w-full max-h-full object-contain rounded-lg" />
    </div>
  )
}

function PdfPreview({ path }) {
  const src = useBlobUrl(path)
  if (!src) return <div className="flex items-center justify-center h-full text-dim font-mono text-xs">Loading...</div>
  return <iframe src={src} className="w-full h-full border-0 rounded-lg" title="PDF preview" />
}

function VideoPreview({ path }) {
  const src = useBlobUrl(path)
  if (!src) return <div className="flex items-center justify-center h-full text-dim font-mono text-xs">Loading...</div>
  return (
    <div className="flex items-center justify-center h-full">
      <video src={src} controls className="max-w-full max-h-full rounded-lg" />
    </div>
  )
}

function AudioPreview({ path }) {
  const src = useBlobUrl(path)
  if (!src) return <div className="flex items-center justify-center h-full text-dim font-mono text-xs">Loading...</div>
  return (
    <div className="flex items-center justify-center h-full">
      <audio src={src} controls className="w-full max-w-md" />
    </div>
  )
}

// ── Icons ───────────────────────────────────────────────────────────────

const FolderIcon = ({ className }) => (
  <svg className={className} viewBox="0 0 24 24" strokeLinecap="round" strokeLinejoin="round">
    <path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z" />
  </svg>
)

const FileIcon = ({ className }) => (
  <svg className={className} viewBox="0 0 24 24" strokeLinecap="round" strokeLinejoin="round">
    <path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" />
    <polyline points="14 2 14 8 20 8" />
  </svg>
)

const ChevronRight = ({ className }) => (
  <svg className={className} viewBox="0 0 24 24" strokeLinecap="round" strokeLinejoin="round">
    <polyline points="9 18 15 12 9 6" />
  </svg>
)

const INPUT_CLASS = 'w-full bg-elevated border border-border-main rounded-lg px-3 py-2 text-[13px] text-primary font-mono placeholder:text-dim focus:outline-none focus:border-accent/50 transition-colors'

export default function FilesView() {
  const { dispatch } = useApp()
  const [currentPath, setCurrentPath] = useState('.')
  const [entries, setEntries] = useState([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState(null)

  // File viewer state
  const [viewingFile, setViewingFile] = useState(null)
  const [fileContent, setFileContent] = useState('')
  const [fileLoading, setFileLoading] = useState(false)
  const [editing, setEditing] = useState(false)
  const [editContent, setEditContent] = useState('')
  const [saveStatus, setSaveStatus] = useState(null)
  const [rawMode, setRawMode] = useState(false)

  // Create dialog state
  const [showCreate, setShowCreate] = useState(null) // 'file' | 'folder' | null
  const [newName, setNewName] = useState('')
  const [creating, setCreating] = useState(false)

  // Delete confirmation
  const [deleteTarget, setDeleteTarget] = useState(null)

  const fetchEntries = useCallback(async (path) => {
    setLoading(true)
    setError(null)
    try {
      const resp = await fetch(`/api/files?path=${encodeURIComponent(path)}`, { headers: apiHeaders() })
      if (!resp.ok) {
        const data = await resp.json().catch(() => ({}))
        throw new Error(data.error || 'Failed to load directory')
      }
      const data = await resp.json()
      // Sort: directories first, then files, alphabetically
      data.sort((a, b) => {
        if (a.type !== b.type) return a.type === 'directory' ? -1 : 1
        return a.name.localeCompare(b.name)
      })
      setEntries(data)
    } catch (e) {
      setError(e.message)
      setEntries([])
    }
    setLoading(false)
  }, [])

  useEffect(() => { fetchEntries(currentPath) }, [currentPath, fetchEntries])

  function navigateTo(path) {
    setViewingFile(null)
    setEditing(false)
    setRawMode(false)
    setCurrentPath(path)
  }

  function navigateUp() {
    if (currentPath === '.') return
    const parts = currentPath.split('/')
    parts.pop()
    navigateTo(parts.length === 0 ? '.' : parts.join('/'))
  }

  function openEntry(entry) {
    const cleanName = entry.name.replace(/\/$/, '')
    const newPath = currentPath === '.' ? cleanName : `${currentPath}/${cleanName}`
    if (entry.type === 'directory') {
      navigateTo(newPath)
    } else {
      openFile(newPath, cleanName)
    }
  }

  async function openFile(path, name) {
    setFileLoading(true)
    setViewingFile({ path, name })
    setEditing(false)
    setSaveStatus(null)
    setRawMode(false)

    // Binary files don't need text content — just show preview
    if (isBinaryPreview(name)) {
      setFileContent('')
      setEditContent('')
      setFileLoading(false)
      return
    }

    try {
      const resp = await fetch(`/api/files/read?path=${encodeURIComponent(path)}`, { headers: apiHeaders() })
      if (!resp.ok) {
        const data = await resp.json().catch(() => ({}))
        setFileContent(`Error: ${data.error || 'Failed to read file'}`)
      } else {
        const data = await resp.json()
        setFileContent(data.content)
        setEditContent(data.content)
      }
    } catch (e) {
      setFileContent(`Error: ${e.message}`)
    }
    setFileLoading(false)
  }

  async function saveFile() {
    setSaveStatus('saving')
    try {
      const resp = await fetch('/api/files/write', {
        method: 'PUT',
        headers: apiHeaders(),
        body: JSON.stringify({ path: viewingFile.path, content: editContent }),
      })
      if (resp.ok) {
        setFileContent(editContent)
        setSaveStatus('saved')
        setEditing(false)
        setTimeout(() => setSaveStatus(null), 2000)
      } else {
        const data = await resp.json().catch(() => ({}))
        setSaveStatus(`Error: ${data.error || 'Failed to save'}`)
      }
    } catch (e) {
      setSaveStatus(`Error: ${e.message}`)
    }
  }

  async function handleCreate() {
    if (!newName.trim()) return
    setCreating(true)
    const path = currentPath === '.' ? newName.trim() : `${currentPath}/${newName.trim()}`
    try {
      if (showCreate === 'folder') {
        await fetch('/api/files/mkdir', {
          method: 'POST',
          headers: apiHeaders(),
          body: JSON.stringify({ path }),
        })
      } else {
        await fetch('/api/files/write', {
          method: 'PUT',
          headers: apiHeaders(),
          body: JSON.stringify({ path, content: '' }),
        })
      }
      setShowCreate(null)
      setNewName('')
      fetchEntries(currentPath)
    } catch {}
    setCreating(false)
  }

  async function handleDelete() {
    if (!deleteTarget) return
    const cleanName = deleteTarget.name.replace(/\/$/, '')
    const path = currentPath === '.' ? cleanName : `${currentPath}/${cleanName}`
    try {
      await fetch(`/api/files?path=${encodeURIComponent(path)}`, {
        method: 'DELETE',
        headers: apiHeaders(),
      })
      setDeleteTarget(null)
      fetchEntries(currentPath)
      if (viewingFile?.path === path) {
        setViewingFile(null)
      }
    } catch {}
  }

  // Breadcrumb segments
  const pathSegments = currentPath === '.' ? [] : currentPath.split('/')

  // Derived state for current file
  const fileType = viewingFile ? getFileType(viewingFile.name) : 'text'
  const canEdit = viewingFile && isTextFile(viewingFile.name)
  const canTogglePreview = viewingFile && hasRichPreview(viewingFile.name) && !isBinaryPreview(viewingFile.name)

  function renderFileContent() {
    if (fileLoading) return <div className="text-dim font-mono text-xs">Loading...</div>
    if (editing) {
      return (
        <textarea
          value={editContent}
          onChange={e => setEditContent(e.target.value)}
          className="w-full h-full bg-transparent text-[13px] font-mono text-primary resize-none focus:outline-none leading-relaxed"
          spellCheck={false}
        />
      )
    }

    // Raw mode → plain text
    if (rawMode) {
      return <pre className="text-[13px] font-mono text-secondary leading-relaxed whitespace-pre-wrap break-words">{fileContent}</pre>
    }

    // Rich previews
    switch (fileType) {
      case 'markdown':
        return <MarkdownPreview content={fileContent} />
      case 'csv':
        return <CsvPreview content={fileContent} />
      case 'json':
        return <JsonPreview content={fileContent} />
      case 'html':
        return <HtmlPreview content={fileContent} />
      case 'image':
        return <ImagePreview path={viewingFile.path} />
      case 'pdf':
        return <PdfPreview path={viewingFile.path} />
      case 'video':
        return <VideoPreview path={viewingFile.path} />
      case 'audio':
        return <AudioPreview path={viewingFile.path} />
      default:
        return <pre className="text-[13px] font-mono text-secondary leading-relaxed whitespace-pre-wrap break-words">{fileContent}</pre>
    }
  }

  return (
    <div className="flex-1 flex flex-col min-h-0 overflow-hidden">
      {/* Header */}
      <div className="flex items-center justify-between px-5 h-12 border-b border-border-subtle shrink-0">
        <div className="flex items-center gap-3">
          <button
            onClick={() => dispatch({ type: 'HIDE_FILES' })}
            className="text-muted hover:text-primary transition-colors cursor-pointer"
          >
            <svg className="w-4 h-4 stroke-current fill-none stroke-2" viewBox="0 0 24 24" strokeLinecap="round">
              <line x1="19" y1="12" x2="5" y2="12" /><polyline points="12 19 5 12 12 5" />
            </svg>
          </button>
          <h2 className="text-sm font-semibold text-primary tracking-tight">Files</h2>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={() => { setShowCreate('folder'); setNewName('') }}
            className="px-2.5 py-1.5 text-[12px] font-mono text-secondary hover:text-primary hover:bg-elevated rounded-lg transition-colors cursor-pointer"
          >
            + Folder
          </button>
          <button
            onClick={() => { setShowCreate('file'); setNewName('') }}
            className="px-2.5 py-1.5 text-[12px] font-mono text-secondary hover:text-primary hover:bg-elevated rounded-lg transition-colors cursor-pointer"
          >
            + File
          </button>
        </div>
      </div>

      {/* Breadcrumbs */}
      <div className="flex items-center gap-1 px-5 py-2 text-[12px] font-mono text-muted border-b border-border-subtle shrink-0 overflow-x-auto">
        <button
          onClick={() => navigateTo('.')}
          className={`hover:text-primary transition-colors cursor-pointer shrink-0 ${currentPath === '.' ? 'text-primary' : ''}`}
        >
          home
        </button>
        {pathSegments.map((seg, i) => (
          <React.Fragment key={i}>
            <ChevronRight className="w-3 h-3 stroke-current fill-none stroke-[1.5] shrink-0 text-dim" />
            <button
              onClick={() => navigateTo(pathSegments.slice(0, i + 1).join('/'))}
              className={`hover:text-primary transition-colors cursor-pointer shrink-0 ${i === pathSegments.length - 1 ? 'text-primary' : ''}`}
            >
              {seg}
            </button>
          </React.Fragment>
        ))}
      </div>

      {/* Create dialog */}
      {showCreate && (
        <div className="px-5 py-3 border-b border-border-subtle bg-surface shrink-0">
          <div className="flex items-center gap-2">
            <span className="text-[11px] text-muted font-mono uppercase tracking-wider shrink-0">
              New {showCreate}:
            </span>
            <input
              type="text"
              value={newName}
              onChange={e => setNewName(e.target.value)}
              onKeyDown={e => { if (e.key === 'Enter') handleCreate(); if (e.key === 'Escape') setShowCreate(null) }}
              placeholder={showCreate === 'folder' ? 'folder-name' : 'filename.txt'}
              autoFocus
              className={INPUT_CLASS + ' max-w-xs'}
            />
            <button
              onClick={handleCreate}
              disabled={creating || !newName.trim()}
              className="px-3 py-1.5 text-[12px] font-mono bg-accent text-white rounded-lg hover:brightness-110 disabled:opacity-40 transition-all cursor-pointer"
            >
              Create
            </button>
            <button
              onClick={() => setShowCreate(null)}
              className="px-2 py-1.5 text-[12px] font-mono text-muted hover:text-primary transition-colors cursor-pointer"
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {/* Delete confirmation */}
      {deleteTarget && (
        <div className="px-5 py-3 border-b border-border-subtle bg-surface shrink-0">
          <div className="flex items-center gap-3">
            <span className="text-[12px] text-err font-mono">
              Delete "{deleteTarget.name.replace(/\/$/, '')}"?
            </span>
            <button
              onClick={handleDelete}
              className="px-3 py-1.5 text-[12px] font-mono bg-err text-white rounded-lg hover:brightness-110 transition-all cursor-pointer"
            >
              Delete
            </button>
            <button
              onClick={() => setDeleteTarget(null)}
              className="px-2 py-1.5 text-[12px] font-mono text-muted hover:text-primary transition-colors cursor-pointer"
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {/* Main content */}
      <div className="flex-1 flex min-h-0 overflow-hidden">
        {/* File list */}
        <div className={`${viewingFile ? 'w-72 border-r border-border-subtle shrink-0' : 'flex-1'} overflow-y-auto`}>
          {loading ? (
            <div className="flex items-center justify-center py-12">
              <div className="text-dim font-mono text-xs">Loading...</div>
            </div>
          ) : error ? (
            <div className="flex items-center justify-center py-12">
              <div className="text-err font-mono text-xs">{error}</div>
            </div>
          ) : entries.length === 0 ? (
            <div className="flex items-center justify-center py-12">
              <div className="text-dim font-mono text-xs">Empty directory</div>
            </div>
          ) : (
            <div className="py-1">
              {currentPath !== '.' && (
                <button
                  onClick={navigateUp}
                  className="flex items-center gap-2.5 w-full px-5 py-2 text-left text-[13px] text-muted hover:text-primary hover:bg-elevated/50 transition-colors cursor-pointer"
                >
                  <svg className="w-4 h-4 stroke-current fill-none stroke-[1.5]" viewBox="0 0 24 24" strokeLinecap="round" strokeLinejoin="round">
                    <polyline points="15 18 9 12 15 6" />
                  </svg>
                  <span className="font-mono">..</span>
                </button>
              )}
              {entries.map((entry) => {
                const isDir = entry.type === 'directory'
                const cleanName = entry.name.replace(/\/$/, '')
                const entryPath = currentPath === '.' ? cleanName : `${currentPath}/${cleanName}`
                const isActive = viewingFile?.path === entryPath

                return (
                  <div
                    key={entry.name}
                    className={`group flex items-center gap-2.5 w-full px-5 py-2 text-left transition-colors ${isActive ? 'bg-elevated text-primary' : 'hover:bg-elevated/50'}`}
                  >
                    <button
                      onClick={() => openEntry(entry)}
                      className="flex items-center gap-2.5 flex-1 min-w-0 cursor-pointer"
                    >
                      {isDir ? (
                        <FolderIcon className="w-4 h-4 stroke-current fill-none stroke-[1.5] text-accent shrink-0" />
                      ) : (
                        <FileIcon className="w-4 h-4 stroke-current fill-none stroke-[1.5] text-muted shrink-0" />
                      )}
                      <span className="text-[13px] font-mono truncate text-secondary group-hover:text-primary transition-colors">
                        {cleanName}
                      </span>
                    </button>
                    {!isDir && (
                      <span className="text-[11px] text-dim font-mono shrink-0">{formatSize(entry.size)}</span>
                    )}
                    <button
                      onClick={(e) => { e.stopPropagation(); setDeleteTarget(entry) }}
                      className="opacity-0 group-hover:opacity-100 text-dim hover:text-err transition-all cursor-pointer shrink-0 p-0.5"
                      title="Delete"
                    >
                      <svg className="w-3.5 h-3.5 stroke-current fill-none stroke-[1.5]" viewBox="0 0 24 24" strokeLinecap="round">
                        <polyline points="3 6 5 6 21 6" /><path d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2" />
                      </svg>
                    </button>
                  </div>
                )
              })}
            </div>
          )}
        </div>

        {/* File viewer / editor */}
        {viewingFile && (
          <div className="flex-1 flex flex-col min-w-0 overflow-hidden">
            {/* File header */}
            <div className="flex items-center justify-between px-4 py-2 border-b border-border-subtle shrink-0">
              <div className="flex items-center gap-2 min-w-0">
                <FileIcon className="w-4 h-4 stroke-current fill-none stroke-[1.5] text-muted shrink-0" />
                <span className="text-[13px] font-mono text-primary truncate">{viewingFile.name}</span>
                {fileType !== 'text' && (
                  <span className="text-[10px] font-mono text-dim uppercase tracking-wider px-1.5 py-0.5 bg-elevated rounded">{fileType}</span>
                )}
              </div>
              <div className="flex items-center gap-2 shrink-0">
                {saveStatus && (
                  <span className={`text-[11px] font-mono ${saveStatus === 'saved' ? 'text-ok' : saveStatus === 'saving' ? 'text-muted' : 'text-err'}`}>
                    {saveStatus === 'saved' ? 'Saved' : saveStatus === 'saving' ? 'Saving...' : saveStatus}
                  </span>
                )}
                {canTogglePreview && !editing && (
                  <button
                    onClick={() => setRawMode(!rawMode)}
                    className={`px-2.5 py-1 text-[12px] font-mono rounded-lg transition-colors cursor-pointer ${rawMode ? 'text-accent bg-accent/10' : 'text-secondary hover:text-primary hover:bg-elevated'}`}
                  >
                    {rawMode ? 'Preview' : 'Raw'}
                  </button>
                )}
                {canEdit && !editing && (
                  <button
                    onClick={() => { setEditing(true); setEditContent(fileContent); setRawMode(false) }}
                    className="px-2.5 py-1 text-[12px] font-mono text-secondary hover:text-primary hover:bg-elevated rounded-lg transition-colors cursor-pointer"
                  >
                    Edit
                  </button>
                )}
                {editing && (
                  <>
                    <button
                      onClick={saveFile}
                      className="px-2.5 py-1 text-[12px] font-mono bg-accent text-white rounded-lg hover:brightness-110 transition-all cursor-pointer"
                    >
                      Save
                    </button>
                    <button
                      onClick={() => { setEditing(false); setEditContent(fileContent) }}
                      className="px-2 py-1 text-[12px] font-mono text-muted hover:text-primary transition-colors cursor-pointer"
                    >
                      Cancel
                    </button>
                  </>
                )}
                <button
                  onClick={() => { setViewingFile(null); setEditing(false); setRawMode(false) }}
                  className="text-muted hover:text-primary transition-colors cursor-pointer p-1"
                >
                  <svg className="w-4 h-4 stroke-current fill-none stroke-2" viewBox="0 0 24 24" strokeLinecap="round">
                    <line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" />
                  </svg>
                </button>
              </div>
            </div>

            {/* File content */}
            <div className={`flex-1 overflow-auto ${fileType === 'pdf' || fileType === 'html' ? '' : 'p-4'}`}>
              {renderFileContent()}
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
