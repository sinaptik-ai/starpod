import React, { useState, useEffect, useCallback } from 'react'
import { useApp } from '../contexts/AppContext'
import { apiHeaders } from '../lib/api'

function formatSize(bytes) {
  if (bytes === 0) return '—'
  if (bytes < 1024) return bytes + ' B'
  if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB'
  return (bytes / (1024 * 1024)).toFixed(1) + ' MB'
}

function isTextFile(name) {
  const ext = name.split('.').pop().toLowerCase()
  const textExts = ['txt', 'md', 'json', 'toml', 'yaml', 'yml', 'csv', 'xml', 'html', 'css', 'js', 'ts', 'jsx', 'tsx', 'rs', 'py', 'rb', 'go', 'sh', 'bash', 'zsh', 'fish', 'conf', 'cfg', 'ini', 'env', 'log', 'sql', 'graphql', 'svg', 'lock', 'gitignore', 'dockerignore', 'editorconfig', 'prettierrc']
  return textExts.includes(ext) || !name.includes('.')
}

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
              </div>
              <div className="flex items-center gap-2 shrink-0">
                {saveStatus && (
                  <span className={`text-[11px] font-mono ${saveStatus === 'saved' ? 'text-ok' : saveStatus === 'saving' ? 'text-muted' : 'text-err'}`}>
                    {saveStatus === 'saved' ? 'Saved' : saveStatus === 'saving' ? 'Saving...' : saveStatus}
                  </span>
                )}
                {isTextFile(viewingFile.name) && !editing && (
                  <button
                    onClick={() => { setEditing(true); setEditContent(fileContent) }}
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
                  onClick={() => { setViewingFile(null); setEditing(false) }}
                  className="text-muted hover:text-primary transition-colors cursor-pointer p-1"
                >
                  <svg className="w-4 h-4 stroke-current fill-none stroke-2" viewBox="0 0 24 24" strokeLinecap="round">
                    <line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" />
                  </svg>
                </button>
              </div>
            </div>

            {/* File content */}
            <div className="flex-1 overflow-auto p-4">
              {fileLoading ? (
                <div className="text-dim font-mono text-xs">Loading...</div>
              ) : editing ? (
                <textarea
                  value={editContent}
                  onChange={e => setEditContent(e.target.value)}
                  className="w-full h-full bg-transparent text-[13px] font-mono text-primary resize-none focus:outline-none leading-relaxed"
                  spellCheck={false}
                />
              ) : (
                <pre className="text-[13px] font-mono text-secondary leading-relaxed whitespace-pre-wrap break-words">{fileContent}</pre>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  )
}
