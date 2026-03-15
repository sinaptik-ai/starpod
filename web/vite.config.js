import { defineConfig } from 'vite'
import tailwindcss from '@tailwindcss/vite'

function frameCheckPlugin() {
  return {
    name: 'frame-check',
    configureServer(server) {
      server.middlewares.use('/api/frame-check', async (req, res) => {
        const url = new URL(req.url, 'http://localhost').searchParams.get('url')
        if (!url) {
          res.writeHead(400, { 'Content-Type': 'application/json' })
          res.end(JSON.stringify({ frameable: false, reason: 'missing url param' }))
          return
        }
        try {
          const controller = new AbortController()
          const timeout = setTimeout(() => controller.abort(), 5000)
          const resp = await fetch(url, {
            signal: controller.signal,
            redirect: 'follow',
          })
          clearTimeout(timeout)

          const xfo = (resp.headers.get('x-frame-options') || '').toLowerCase()
          const csp = (resp.headers.get('content-security-policy') || '').toLowerCase()

          let frameable = true
          let reason = ''
          let ogImage = ''
          let ogTitle = ''

          if (xfo === 'deny' || xfo === 'sameorigin') {
            frameable = false
            reason = 'X-Frame-Options: ' + xfo
          }

          if (csp.includes('frame-ancestors')) {
            const match = csp.match(/frame-ancestors\s+([^;]+)/)
            if (match) {
              const val = match[1].trim()
              // Only frameable if frame-ancestors includes * (allow all)
              if (!val.includes('*')) {
                frameable = false
                reason = 'CSP frame-ancestors: ' + val
              }
            }
          }

          // Extract og:image and og:title when not frameable
          if (!frameable) {
            try {
              const html = await resp.text()
              const imgMatch = html.match(/<meta[^>]*property=["']og:image["'][^>]*content=["']([^"']+)["']/i)
                || html.match(/<meta[^>]*content=["']([^"']+)["'][^>]*property=["']og:image["']/i)
              if (imgMatch) ogImage = imgMatch[1]
              const titleMatch = html.match(/<meta[^>]*property=["']og:title["'][^>]*content=["']([^"']+)["']/i)
                || html.match(/<meta[^>]*content=["']([^"']+)["'][^>]*property=["']og:title["']/i)
              if (titleMatch) ogTitle = titleMatch[1]
            } catch {}
          }

          res.writeHead(200, { 'Content-Type': 'application/json' })
          res.end(JSON.stringify({ frameable, reason, ogImage, ogTitle }))
        } catch (e) {
          res.writeHead(200, { 'Content-Type': 'application/json' })
          res.end(JSON.stringify({ frameable: false, reason: e.message, ogImage: '', ogTitle: '' }))
        }
      })
    },
  }
}

export default defineConfig({
  root: '.',
  plugins: [tailwindcss(), frameCheckPlugin()],
  build: {
    outDir: '../crates/starpod-gateway/static/dist',
    emptyOutDir: true,
  },
  server: {
    proxy: {
      '/api': {
        target: 'http://127.0.0.1:3000',
        bypass(req) {
          if (req.url.startsWith('/api/frame-check')) return req.url
        },
      },
      '/ws': {
        target: 'ws://127.0.0.1:3000',
        ws: true,
      },
    },
  },
})
