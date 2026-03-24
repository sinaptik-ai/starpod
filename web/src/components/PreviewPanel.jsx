import React, { useState, useEffect, useRef, useCallback } from 'react'
import { useApp } from '../contexts/AppContext'

function PreviewPanel() {
  const { state, dispatch } = useApp()
  const { previewUrl } = state

  const [frameable, setFrameable] = useState(null) // null = checking, true = show iframe, false = fallback
  const [ogImage, setOgImage] = useState(null)
  const [ogTitle, setOgTitle] = useState(null)
  const iframeRef = useRef(null)
  const loadTimerRef = useRef(null)
  const iframeLoadedRef = useRef(false)

  const isOpen = !!previewUrl

  const closePreview = useCallback(() => {
    dispatch({ type: 'CLOSE_PREVIEW' })
  }, [dispatch])

  // Frame check when URL changes
  useEffect(() => {
    if (!previewUrl) {
      setFrameable(null)
      setOgImage(null)
      setOgTitle(null)
      clearTimeout(loadTimerRef.current)
      return
    }

    setFrameable(null)
    setOgImage(null)
    setOgTitle(null)
    iframeLoadedRef.current = false

    // Localhost URLs are always frameable — skip the server-side check
    // (the gateway may not be able to reach the user's local machine)
    let isLocal = false
    try {
      const h = new URL(previewUrl).hostname
      isLocal = h === 'localhost' || h === '127.0.0.1' || h === '0.0.0.0' || h === '::1'
    } catch {}

    if (isLocal) {
      setFrameable(true)
    } else {
      fetch('/api/frame-check?url=' + encodeURIComponent(previewUrl))
        .then(r => r.json())
        .then(data => {
          if (data.frameable) {
            setFrameable(true)
          } else {
            setFrameable(false)
            if (data.ogImage) setOgImage(data.ogImage)
            if (data.ogTitle) setOgTitle(data.ogTitle)
            else {
              try { setOgTitle(new URL(previewUrl).hostname) } catch {}
            }
          }
        })
        .catch(() => {
          // Endpoint unavailable - try iframe
          setFrameable(true)
        })
    }

    // Safety timeout
    clearTimeout(loadTimerRef.current)
    loadTimerRef.current = setTimeout(() => {
      if (!iframeLoadedRef.current) {
        setFrameable(false)
      }
    }, 6000)

    return () => clearTimeout(loadTimerRef.current)
  }, [previewUrl])

  function handleIframeLoad() {
    if (!previewUrl) return
    iframeLoadedRef.current = true
    clearTimeout(loadTimerRef.current)
  }

  function handleIframeError() {
    setFrameable(false)
  }

  // Register global _openPreview for markdown links
  useEffect(() => {
    window._openPreview = (url) => {
      dispatch({ type: 'OPEN_PREVIEW', payload: url })
    }
    return () => { delete window._openPreview }
  }, [dispatch])

  return (
    <div id="preview-panel" className={isOpen ? 'open' : ''}>
      <div id="preview-inner">
        {/* Header */}
        <div className="flex items-center gap-2 px-4 py-2.5 border-b border-border-subtle shrink-0">
          <span className="flex-1 min-w-0 text-xs font-mono text-dim truncate">
            {previewUrl || ''}
          </span>
          {previewUrl && (
            <a
              href={previewUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="text-dim hover:text-secondary transition-colors shrink-0 p-1"
              title="Open in new tab"
              aria-label="Open in new tab"
            >
              <svg className="w-3.5 h-3.5 stroke-current fill-none stroke-2" viewBox="0 0 24 24" strokeLinecap="round">
                <path d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6M15 3h6v6M10 14L21 3" />
              </svg>
            </a>
          )}
          <button
            onClick={closePreview}
            className="text-dim hover:text-secondary transition-colors shrink-0 p-1 cursor-pointer"
            aria-label="Close preview"
          >
            <svg className="w-3.5 h-3.5 stroke-current fill-none stroke-2" viewBox="0 0 24 24" strokeLinecap="round">
              <path d="M18 6L6 18M6 6l12 12" />
            </svg>
          </button>
        </div>

        {/* Content */}
        <div className="flex-1 relative overflow-hidden">
          {frameable !== false && (
            <iframe
              ref={iframeRef}
              src={frameable === true ? previewUrl : 'about:blank'}
              className={`w-full h-full border-none bg-white ${frameable === false ? 'hidden' : ''}`}
              onLoad={handleIframeLoad}
              onError={handleIframeError}
              sandbox="allow-scripts allow-same-origin allow-forms allow-popups"
            />
          )}

          {frameable === false && (
            <div className="flex flex-col items-center justify-center h-full gap-4 px-8 text-center">
              {ogImage && (
                <div className="max-w-md rounded-none overflow-hidden border border-border-main shadow-lg">
                  <img src={ogImage} className="w-full" alt="" />
                </div>
              )}
              <div className="text-sm text-secondary font-medium">
                {ogTitle || 'Connection refused'}
              </div>
              <div className="text-xs text-dim">
                This site cannot be displayed in a frame.
              </div>
              {previewUrl && (
                <a
                  href={previewUrl}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="text-accent text-sm hover:underline"
                >
                  Open in new tab
                </a>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  )
}

export default PreviewPanel
