import { isTauri } from '@tauri-apps/api/core'
import { Cpu, Download, FlaskConical, Play, SlidersHorizontal, Square, Trash2 } from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { api } from '@/api'
import { localModels, sizeLabel, type LocalConfig, type LocalModel } from '@/lib/pocket/catalog'
import { magicTouch } from '@/lib/pocket/magic-touch'
import './PocketLab.css'

type Status = Awaited<ReturnType<typeof api.pocketStatus>>
type Turn = { role: 'user' | 'assistant'; text: string; reasoning?: string }
const configs = (): Record<string, LocalConfig> => { try { return JSON.parse(localStorage.getItem('neru.pocket.config') ?? '{}') } catch { return {} } }

export function PocketLab({ onError, onNotice }: { onError: (text: string) => void; onNotice: (text: string) => void }) {
  const [status, setStatus] = useState<Status | null>(null)
  const [busy, setBusy] = useState('')
  const [progress, setProgress] = useState<{ id: string; loaded: number; total: number; phase: string } | null>(null)
  const [setupText, setSetupText] = useState('')
  const [token, setToken] = useState('')
  const [filter, setFilter] = useState('all')
  const [settings, setSettings] = useState(configs)
  const [editing, setEditing] = useState<LocalModel | null>(null)
  const [config, setConfig] = useState<LocalConfig>(localModels[0].defaults)
  const [chosen, setChosen] = useState<LocalModel | null>(null)
  const [draft, setDraft] = useState('')
  const [turns, setTurns] = useState<Turn[]>([])
  const [photo, setPhoto] = useState('')
  const [cutout, setCutout] = useState('')
  const requestId = useRef('')
  const dialog = useRef<HTMLDialogElement>(null)
  const [labError, setLabError] = useState('')
  useEffect(() => { if (editing || chosen) dialog.current?.showModal() }, [editing, chosen])
  const native = isTauri()
  const configFor = (m: LocalModel): LocalConfig => ({ ...m.defaults, ...settings[m.id], ...(m.kind === 'segmenter' ? { accelerator: 'cpu' as const } : {}) })
  const acceleratorsFor = (m: LocalModel) => m.kind === 'segmenter' ? ['cpu'] : m.accelerators
  const refresh = async () => setStatus(await api.pocketStatus())
  useEffect(() => {
    if (!native) return
    void refresh().catch(e => onError(String(e)))
    const unlisten = api.onPocketDownload(setProgress)
    const setup = api.onPocketSetup(setSetupText)
    const tokens = api.onPocketToken(chunk => { if (chunk.requestId !== requestId.current) return; setTurns(rows => rows.map((row, i) => i === rows.length - 1 ? { ...row, text: row.text + chunk.text, reasoning: (row.reasoning ?? '') + (chunk.reasoning ?? '') } : row)) })
    return () => { void unlisten.then(fn => fn()); void tokens.then(fn => fn()); void setup.then(fn => fn()) }
  }, [native, onError])
  const run = async (name: string, task: () => Promise<unknown>) => {
    setBusy(name); setLabError('')
    try { await task(); await refresh() } catch (e) { setLabError(e instanceof Error ? e.message : String(e)); onError(String(e)) } finally { setBusy(''); setProgress(null) }
  }
  const save = () => {
    const next = { ...settings, [editing!.id]: config }
    try { localStorage.setItem('neru.pocket.config', JSON.stringify(next)); setSettings(next); setEditing(null); onNotice('Pocket Lab configuration saved.') } catch { onError('Could not save the model configuration.') }
  }
  const send = async () => {
    if (!chosen || busy || !draft.trim()) return
    const messages: Turn[] = [...turns, { role: 'user', text: draft.trim() }]
    setTurns([...messages, { role: 'assistant', text: '' }]); setDraft('')
    requestId.current = crypto.randomUUID()
    await run('generating', () => api.pocketRun(chosen.id, requestId.current, { messages: messages.map(({ role, text }) => ({ role, text })), config: configFor(chosen) }))
  }
  const segment = (event: React.MouseEvent<HTMLImageElement>) => {
    if (!chosen || busy || !photo) return
    const box = event.currentTarget.getBoundingClientRect()
    void run('generating', async () => {
      setCutout(await magicTouch(await api.pocketVisionModel(), photo, (event.clientX - box.left) / box.width, (event.clientY - box.top) / box.height, configFor(chosen).accelerator))
    })
  }
  return <section className="settings-section pocket-lab"><div className="pocket-heading"><FlaskConical size={24} /><span>ON THIS COMPUTER · OFFLINE</span></div><h2>Pocket Lab</h2><p className="settings-lede">Big ideas. Pocket-sized. Download models from Google AI Edge Gallery’s catalog and run them on this computer with Google LiteRT-LM.</p>
    {!native ? <p className="settings-note">Open the Neru desktop app to download and run local models.</p> : <div className="pocket-runtime"><Cpu size={20} /><div><strong>{status?.runtime ? 'Local runtime ready' : 'Set up your local runtime'}</strong><p>{busy === 'setup' ? setupText || 'Downloading the local runtime…' : 'Models, runtime and caches stay in Neru’s data folder on D:. Downloads need internet; inference does not.'}</p></div>{!status?.runtime ? <button className="button subtle" disabled={!!busy} onClick={() => void run('setup', api.pocketSetup)}>Set up Pocket Lab</button> : null}</div>}
    <div className="pocket-tabs" role="tablist" aria-label="Model category">{[['all', 'All 10'], ['chat', 'Chat'], ['tools', 'Tools'], ['segmenter', 'Vision']].map(([id, label]) => <button key={id} role="tab" aria-selected={filter === id} className={filter === id ? 'active' : ''} onClick={() => setFilter(id)}>{label}</button>)}</div>
    <div className="pocket-catalog">{localModels.filter(m => filter === 'all' || m.kind === filter).map(m => {
      const state = status?.models.find(s => s.id === m.id)
      const active = progress?.id === m.id
      return <article key={m.id} className="pocket-card"><div className="pocket-card-title"><h3>{m.name}</h3>{state?.ready ? <span className="speech-tag accent">Downloaded</span> : <span className="speech-tag">{m.kind === 'segmenter' ? 'Vision' : m.kind === 'tools' ? 'Tools' : 'Chat'}</span>}</div><p>{m.description}</p><small>{sizeLabel(m.bytes)}{m.memory ? ` · ${m.memory} GB RAM recommended` : ''} · {acceleratorsFor(m).map(a => a.toUpperCase()).join(' / ')}</small>
        {active ? <div className="pocket-progress"><progress max={progress.total} value={progress.loaded} aria-label={`Downloading ${m.name}`} /><small>{progress.phase === 'verifying' ? 'Checking download…' : `${sizeLabel(progress.loaded)} of ${sizeLabel(progress.total)}`}</small></div> : null}
        <div className="pocket-actions">{state?.ready ? <><button className="button primary" disabled={!!busy || (m.kind !== 'segmenter' && !status?.runtime)} onClick={() => { setChosen(m); setTurns([]); setPhoto(''); setCutout('') }}><Play size={14} />{m.kind === 'segmenter' ? 'Try Magic Touch' : 'Open prompt lab'}</button><button className="icon-button" disabled={!!busy} aria-label={`Remove ${m.name}`} onClick={() => { if (window.confirm(`Remove ${m.name}?`)) void run('remove', () => api.pocketRemove(m.id)) }}><Trash2 size={16} /></button></> : busy === m.id ? <button className="button subtle" onClick={() => void api.pocketPause().catch(e => onError(String(e)))}>Pause</button> : <button className="button subtle" disabled={!native || !!busy} onClick={() => void run(m.id, () => api.pocketDownload(m.id, token))}><Download size={14} />{state?.partial ? 'Resume download' : 'Download'}</button>}
          <button className="icon-button" aria-label={`Configure ${m.name}`} disabled={!!busy} onClick={() => { setEditing(m); setConfig(configFor(m)) }}><SlidersHorizontal size={16} /></button></div><a href={m.url ? 'https://developers.google.com/edge/mediapipe/solutions/vision/interactive_segmenter' : `https://huggingface.co/${m.repo}`} target="_blank" rel="noreferrer">Model details and terms</a></article>
    })}</div>
    <details className="pocket-token"><summary>Hugging Face gated downloads</summary><p>Accept the model’s terms on Hugging Face. A read token is used only for downloads and is kept in memory for this window.</p><label>Hugging Face read token<input type="password" autoComplete="off" placeholder="hf_…" value={token} onChange={e => setToken(e.target.value)} /></label><button className="button subtle" onClick={() => setToken('')}>Clear token</button></details>
    {editing ? <dialog onKeyDown={e => { if (e.key === 'Escape') e.stopPropagation() }} ref={dialog} className="pocket-dialog" aria-labelledby="pocket-config-title" onCancel={() => setEditing(null)}><form onSubmit={e => { e.preventDefault(); save() }}><h2 id="pocket-config-title">{editing.name} · Configuration</h2><label>Accelerator<select value={config.accelerator} onChange={e => setConfig({ ...config, accelerator: e.target.value as 'cpu' | 'gpu' })}>{acceleratorsFor(editing).map(a => <option key={a} value={a}>{a.toUpperCase()}</option>)}</select></label>
      {editing.kind !== 'segmenter' ? <><label>System prompt<textarea rows={3} maxLength={50000} value={config.systemPrompt} onChange={e => setConfig({ ...config, systemPrompt: e.target.value })} /></label><div className="settings-grid">{([['contextTokens', 'Context tokens', 512, editing.maxContext, 1], ['maxTokens', 'Max output tokens', 1, config.contextTokens, 1], ['topK', 'Top K', 1, 128, 1], ['topP', 'Top P', 0.00001, 1, 'any'], ['temperature', 'Temperature', 0, 2, 0.01]] as const).map(([key, label, min, max, step]) => <label key={key}>{label}<input required type="number" min={min} max={max} step={step} value={config[key]} onChange={e => setConfig({ ...config, [key]: Number(e.target.value) })} /></label>)}</div>{(['thinking', 'speculative'] as const).map(key => <label key={key} className="pocket-check"><input type="checkbox" disabled={!editing[key]} checked={config[key]} onChange={e => setConfig({ ...config, [key]: e.target.checked })} />{key === 'thinking' ? 'Thinking' : 'Speculative decoding'}{!editing[key] ? ' · Not supported by this model' : ''}</label>)}</> : <p>Magic Touch selects objects in images using the verified desktop CPU runtime. Language generation settings do not apply.</p>}<div className="settings-actions"><button type="button" className="button subtle" onClick={() => setConfig(editing.defaults)}>Reset defaults</button><button type="button" className="button subtle" onClick={() => setEditing(null)}>Cancel</button><button className="button primary" type="submit">Save configuration</button></div></form></dialog> : null}
    {chosen ? <dialog onKeyDown={e => { if (e.key === 'Escape') e.stopPropagation() }} ref={dialog} className="pocket-dialog pocket-playground" aria-label={`${chosen.name} prompt lab`} onCancel={e => { if (busy) e.preventDefault(); else setChosen(null) }}><div className="pocket-card-title"><h2>{chosen.name}</h2><button className="button subtle" disabled={!!busy} onClick={() => setChosen(null)}>Close</button></div><p>Running on this computer · {configFor(chosen).accelerator.toUpperCase()} · Offline</p>
      {chosen.kind === 'segmenter' ? <><label>Choose a photo<input type="file" accept="image/*" disabled={!!busy} onChange={e => { const file = e.target.files?.[0]; if (!file) return; if (file.size > 20000000) return onError('Choose a photo smaller than 20 MB.'); const reader = new FileReader(); reader.onload = () => { setPhoto(String(reader.result)); setCutout('') }; reader.readAsDataURL(file) }} /></label><p>Tap the object to keep.</p>{photo ? <img className="pocket-photo" src={photo} onClick={segment} alt="Tap the object to select" /> : null}{cutout ? <><img className="pocket-photo pocket-cutout" src={cutout} alt="Selected object with transparent background" /><a className="button subtle" href={cutout} download="neru-cutout.png">Save cutout</a></> : null}</> : <><div className="pocket-turns" aria-live="polite">{turns.map((row, i) => <div key={i} className={`pocket-turn ${row.role}`}><strong>{row.role === 'user' ? 'You' : chosen.name}</strong>{row.reasoning ? <details><summary>Thinking</summary><p>{row.reasoning}</p></details> : null}<p>{row.text || (busy ? 'Loading the local model…' : 'No response text.')}</p></div>)}</div><form onSubmit={e => { e.preventDefault(); void send() }}><label>Prompt<textarea value={draft} onChange={e => setDraft(e.target.value)} rows={3} placeholder="Message this model…" disabled={!!busy} maxLength={100000} /></label><div className="settings-actions">{busy ? <button type="button" className="button subtle" onClick={() => void api.pocketCancel()}><Square size={14} />Stop</button> : <button className="button primary" disabled={!draft.trim()} type="submit">Send prompt</button>}</div></form></>}
      {labError ? <p role="alert">{labError}</p> : null}
      {chosen.kind === 'segmenter' && busy ? <p role="status">Selecting the object… <button className="button subtle" onClick={() => void api.pocketCancel()}>Stop</button></p> : null}</dialog> : null}
  </section>
}
