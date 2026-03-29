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

let cachedModels = null
export async function fetchModels() {
  if (cachedModels) return cachedModels
  try {
    const resp = await fetch('/api/settings/models', { headers: apiHeaders() })
    if (resp.ok) cachedModels = (await resp.json()).models
  } catch {}
  return cachedModels || {}
}
