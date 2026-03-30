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

export function markSessionRead(sessionId, isRead = true) {
  fetch(`/api/sessions/${sessionId}/read`, {
    method: 'POST',
    headers: apiHeaders(),
    body: JSON.stringify({ is_read: isRead }),
  }).catch(() => {})
}

let cachedSkills = null
let skillsFetchedAt = 0
const SKILLS_TTL = 30_000

export async function fetchSkills() {
  if (cachedSkills && Date.now() - skillsFetchedAt < SKILLS_TTL) return cachedSkills
  try {
    const resp = await fetch('/api/settings/skills', { headers: apiHeaders() })
    if (resp.ok) {
      cachedSkills = await resp.json()
      skillsFetchedAt = Date.now()
    }
  } catch {}
  return cachedSkills || []
}

export function invalidateSkillsCache() {
  cachedSkills = null
  skillsFetchedAt = 0
}

let cachedVersion = null
export async function fetchVersion() {
  if (cachedVersion) return cachedVersion
  try {
    const resp = await fetch('/api/health')
    if (resp.ok) {
      const data = await resp.json()
      cachedVersion = data.version
    }
  } catch {}
  return cachedVersion || null
}

export async function fetchSystemVersion() {
  try {
    const resp = await fetch('/api/system/version', { headers: apiHeaders() })
    if (resp.ok) return await resp.json()
  } catch {}
  return null
}

export async function triggerUpdate() {
  const resp = await fetch('/api/system/update', {
    method: 'POST',
    headers: apiHeaders(),
  })
  if (!resp.ok) {
    const data = await resp.json().catch(() => ({}))
    throw new Error(data.error || `Update failed (HTTP ${resp.status})`)
  }
  return await resp.json()
}

export async function pollHealthForVersion(targetVersion, timeoutMs = 30000) {
  const start = Date.now()
  while (Date.now() - start < timeoutMs) {
    await new Promise(r => setTimeout(r, 2000))
    try {
      const resp = await fetch('/api/health')
      if (resp.ok) {
        const data = await resp.json()
        if (data.version === targetVersion) return true
      }
    } catch {}
  }
  return false
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
