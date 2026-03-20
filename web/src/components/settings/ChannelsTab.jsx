import { useState, useEffect } from 'react'
import { apiHeaders } from '../../lib/api'
import { Card, Row, Input, Select, Toggle, SaveBar } from './fields'
import StepGuide from './StepGuide'
import { Loading } from '../ui/EmptyState'

export default function ChannelsTab() {
  const [config, setConfig] = useState(null)
  const [saving, setSaving] = useState(false)
  const [status, setStatus] = useState(null)

  useEffect(() => {
    fetch('/api/settings/channels', { headers: apiHeaders() })
      .then(r => r.json())
      .then(d => setConfig(d))
      .catch(() => setStatus({ type: 'error', text: 'Failed to load' }))
  }, [])

  if (!config) return <Loading />

  const tg = config.telegram || { enabled: false, gap_minutes: 360, stream_mode: 'final_only', bot_token: '' }
  const setTg = (key, val) => setConfig(prev => ({
    ...prev,
    telegram: { ...prev.telegram, [key]: val },
  }))
  const hasToken = !!(tg.bot_token && tg.bot_token.trim())

  const save = async () => {
    setSaving(true); setStatus(null)
    try {
      const resp = await fetch('/api/settings/channels', { method: 'PUT', headers: apiHeaders(), body: JSON.stringify(config) })
      setStatus(resp.ok ? { type: 'ok', text: 'Saved' } : { type: 'error', text: 'Failed' })
    } catch (e) { setStatus({ type: 'error', text: e.message }) }
    setSaving(false)
  }

  return (
    <>
      <Card title="Telegram">
        <Toggle label="Enabled" checked={tg.enabled} onChange={v => setTg('enabled', v)} />
        {tg.enabled && (
          <>
            <Row label="Bot token" sub="from @BotFather on Telegram">
              <Input type="password" value={tg.bot_token ?? ''} onChange={v => setTg('bot_token', v)} placeholder="123456:ABC-DEF..." />
            </Row>
            {!hasToken && (
              <StepGuide
                title="How to get a bot token"
                steps={[
                  { text: <>Open <a href="https://t.me/BotFather" target="_blank" rel="noopener noreferrer" className="text-accent hover:underline">@BotFather</a> on Telegram</> },
                  { text: <>Send <code className="text-secondary">/newbot</code> and choose a name for your bot</> },
                  { text: <>BotFather will reply with a token — copy it</> },
                  { text: 'Paste the token in the field above' },
                ]}
                note="A restart is required after changing the token."
              />
            )}
            <Row label="Session gap" sub="minutes of inactivity before new session">
              <Input type="number" value={tg.gap_minutes ?? 360} onChange={v => setTg('gap_minutes', v === '' ? null : Number(v))} placeholder="360" />
            </Row>
            <Row label="Stream mode" sub="how messages are sent to Telegram">
              <Select value={tg.stream_mode || 'final_only'} onChange={v => setTg('stream_mode', v)} options={[
                { value: 'final_only', label: 'Final only' },
                { value: 'all_messages', label: 'All messages' },
              ]} />
            </Row>
          </>
        )}
      </Card>

      <SaveBar onSave={save} saving={saving} status={status} />
    </>
  )
}
