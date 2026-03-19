export function apiHeaders() {
  const h = { 'Content-Type': 'application/json' }
  const token = localStorage.getItem('starpod_api_key')
  if (token) h['X-API-Key'] = token
  return h
}

export function authHeaders() {
  const h = {}
  const token = localStorage.getItem('starpod_api_key')
  if (token) h['X-API-Key'] = token
  return h
}

let cachedModels = null
export async function fetchModels() {
  if (cachedModels) return cachedModels
  try {
    const resp = await fetch('/api/settings/models', { headers: apiHeaders() })
    if (resp.ok) cachedModels = (await resp.json()).models
  } catch {}
  return cachedModels || {}
}
