import { useCallback, useEffect, useRef, useState } from 'react'
import { Check, Cpu, Download, LoaderCircle, RotateCw, Trash2, X } from '@/components/icons'
import { api } from '../../api'
import type { BuiltinAdvice, BuiltinDownload, BuiltinPick, BuiltinSettings, BuiltinStatus, EngineProgress } from '../../types'
import { cn } from '@/lib/utils'
import { t } from '../../i18n'
import './LocalModels.css'

// Settings → Model provider → Built-in: Neru's own llama.cpp engine. What this computer can run with the
// quantization that fits, downloads from Hugging Face with progress and cancel, the downloaded models
// (load, unload, speed check, remove) and the engine's settings (llama_server.rs, llama_models.rs).

const ROLE_LABELS: Record<string, string> = { agent: 'For Code', chat: 'For chat', helper: 'Helper' }
const BACKENDS: Record<string, string> = { cpu: 'the processor', vulkan: 'Vulkan', cuda: 'CUDA', metal: 'Metal', custom: 'your own llama-server' }
const gb = (bytes: number) => { const value = bytes / 1_073_741_824; return value >= 10 ? `${value.toFixed(0)} GB` : `${value.toFixed(1)} GB` }
const errorText = (cause: unknown) => cause instanceof Error ? cause.message : String(cause)
const keyOf = (pick: { repo: string; file: string }) => `${pick.repo}/${pick.file}`

function Bar({ completed, total, label, onCancel }: { completed: number; total: number; label: string; onCancel?: () => void }) {
  const share = total ? completed / total : 0
  return <span className="lm-progress">
    <span className="lm-bar" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(share * 100)}><i style={{ width: `${share * 100}%` }} /></span>
    <small>{label}</small>
    {onCancel && <button type="button" className="icon-button" onClick={onCancel} aria-label={t('Cancel download')} title={t('Cancel download')}><X size={13} /></button>}
  </span>
}

function downloadLabel(progress: BuiltinDownload) {
  if (progress.status === 'verifying') return t('Checking the file…')
  if (!progress.total) return t('Starting…')
  const speed = progress.speed ? ` · ${(progress.speed / 1_048_576).toFixed(1)} MB/s` : ''
  return `${t('{done} of {total}', { done: gb(progress.completed), total: gb(progress.total) })}${speed}`
}

export interface BuiltinViewProps {
  advice: BuiltinAdvice | null; status: BuiltinStatus | null; downloads: Record<string, BuiltinDownload>; engine: EngineProgress | null
  busy: Record<string, boolean>; error: string | null
  onDownload: (repo: string, file: string) => void; onCancel: (key: string) => void; onDelete: (id: string) => void
  onLoad: (id: string) => void; onUnload: (id: string) => void; onSpeed: (id: string) => void; onInstallEngine: () => void
  onSettings: (settings: BuiltinSettings) => void; onRefresh: () => void
}

export function BuiltinModelsView(props: BuiltinViewProps) {
  const { advice, status, downloads, engine, busy, error } = props
  const [confirm, setConfirm] = useState<string | null>(null)
  const [quants, setQuants] = useState<Record<string, string>>({})
  const [repo, setRepo] = useState('')
  const [file, setFile] = useState('')
  const installed = new Map(status?.models.map(model => [`${model.repo}/${model.file}`, model]) ?? [])
  const loaded = new Map(status?.loaded.map(model => [model.id, model]) ?? [])
  const hardware = advice?.hardware
  const graphics = hardware?.gpus.find(gpu => gpu.dedicated)
  const fileFor = (pick: BuiltinPick) => {
    const quant = quants[pick.id] ?? pick.quant
    return quant === pick.quant ? pick.file : pick.file.replace(new RegExp(`${pick.quant}\\.gguf$`), `${quant}.gguf`)
  }
  const visible = (advice?.models ?? []).filter(pick => pick.fit !== 'no' || pick.recommended)
  const tooBig = (advice?.models ?? []).filter(pick => pick.fit === 'no' && !pick.recommended)
  const settings = status?.settings
  return <div className="local-models builtin-models">
    {advice && hardware && <div className="lm-machine">
      <Cpu size={16} aria-hidden />
      <div>
        <p className="lm-specs">{[t('{n} GB memory', { n: Math.round(hardware.ramGb) }), hardware.unified ? t('Apple silicon, shared memory') : graphics ? `${graphics.name} (${Math.round(graphics.vramGb)} GB)` : t('no graphics card with its own memory'), advice.diskFreeGb !== null ? t('{n} GB free for models', { n: Math.round(advice.diskFreeGb) }) : null].filter(Boolean).join(' · ')}</p>
        <p>{advice.verdict}</p>
      </div>
    </div>}

    <div className={cn('lm-status', status?.engine && 'is-running')} role="status">
      {!status ? <span><LoaderCircle size={14} className="animate-spin" /> {t('Checking the engine…')}</span>
        : engine && !engine.done ? <><span>{t('Getting llama.cpp for {backend}…', { backend: t(BACKENDS[engine.backend] ?? engine.backend) })}</span><Bar completed={engine.completed} total={engine.total} label={engine.status === 'downloading' ? t('{done} of {total}', { done: gb(engine.completed), total: gb(engine.total) }) : t(engine.status === 'verifying' ? 'Checking…' : 'Unpacking…')} /></>
          : status.engine ? <span><Check size={14} /> {t('llama.cpp {tag} on {backend} is ready.', { tag: status.engine.tag, backend: t(BACKENDS[status.engine.backend] ?? status.engine.backend) })}</span>
            : <><span>{t('The engine (llama.cpp for {backend}) downloads with the first model, or now.', { backend: t(BACKENDS[status.backend] ?? status.backend) })}</span><button type="button" className="button subtle small" onClick={props.onInstallEngine}>{t('Get the engine')}</button></>}
      <button type="button" className="icon-button" onClick={props.onRefresh} aria-label={t('Check again')} title={t('Check again')}><RotateCw size={13} /></button>
    </div>
    {error && <p className="lm-error" role="alert">{error}</p>}

    {visible.length > 0 && <>
      <p className="lm-subhead">{t('Models for this computer')}</p>
      <ul className="lm-list">
        {visible.map(pick => {
          const chosen = fileFor(pick)
          const key = `${pick.repo}/${chosen}`
          const progress = downloads[key]
          // Any quantization of it already downloaded counts, unless another one is chosen on purpose.
          const have = installed.get(key) ?? (quants[pick.id] ? undefined : status?.models.find(model => model.catalogId === pick.id))
          return <li key={pick.id} className={cn(pick.recommended && 'is-recommended')}>
            <span className="lm-name">
              <strong>{pick.name}{pick.recommended && <em className="lm-badge">{t('Recommended')}</em>}</strong>
              <small>{pick.roles.map(role => t(ROLE_LABELS[role] ?? role)).join(', ')} · {pick.note}</small>
              <small className={cn('lm-fit', `is-${pick.fit}`)}>{pick.fitLabel}</small>
              <small>
                {pick.alternatives.length > 0
                  ? <select className="lm-quant" value={quants[pick.id] ?? pick.quant} onChange={event => setQuants(current => ({ ...current, [pick.id]: event.target.value }))} aria-label={t('Quantization for {name}', { name: pick.name })}>
                    {[pick.quant, ...pick.alternatives].map(quant => <option key={quant} value={quant}>{quant}{quant === pick.quant ? ` (${t('fits best')})` : ''}</option>)}
                  </select>
                  : pick.quant}
                {' · '}{t(pick.quality)} · {gb(pick.bytes)} · {t('{n}K context', { n: Math.round(pick.context / 1024) })}
              </small>
            </span>
            {progress && !progress.done ? <Bar completed={progress.completed} total={progress.total} label={downloadLabel(progress)} onCancel={() => props.onCancel(key)} />
              : have ? <span className="lm-installed"><Check size={13} />{have.quant === pick.quant ? t('Downloaded') : t('Downloaded ({quant})', { quant: have.quant })}</span>
                : <button type="button" className="button subtle small" onClick={() => props.onDownload(pick.repo, chosen)} disabled={pick.fit === 'no' || pick.fit === 'disk'} aria-label={t('Download {tag}', { tag: pick.name })}><Download size={13} />{t('Download')}</button>}
          </li>
        })}
      </ul>
      {tooBig.length > 0 && <p className="lm-empty">{t('Too large here: {names}.', { names: tooBig.map(pick => pick.name).join(', ') })}</p>}
    </>}

    <p className="lm-subhead">{t('Downloaded')}</p>
    {!status?.models.length ? <p className="lm-empty">{t('No models yet. Download one above.')}</p>
      : <ul className="lm-list">
        {status.models.map(model => {
          const running = loaded.get(model.id)
          const speed = status.speeds[model.id]
          return <li key={model.id}>
            <span className="lm-name"><code>{model.id}</code>
              <small>{[model.quant, gb(model.bytes), running ? t('loaded · {n}K context · {backend}', { n: Math.round(running.context / 1024), backend: t(BACKENDS[running.backend] ?? running.backend) }) : null, running?.gpuLayers].filter(Boolean).join(' · ')}</small>
              {busy[`speed:${model.id}`] ? <small className="lm-speed"><LoaderCircle size={11} className="animate-spin" /> {t('Measuring its speed on this computer…')}</small>
                : speed ? <small className={cn('lm-speed', `is-${speed.verdict}`)} title={t('Reads {prompt} tokens/s, writes {write} tokens/s', { prompt: Math.round(speed.promptPerSecond), write: Math.round(speed.generatePerSecond) })}>{speed.sentence}</small>
                  : <button type="button" className="lm-link" onClick={() => props.onSpeed(model.id)}>{t('Check its speed')}</button>}
            </span>
            <span className="lm-actions">
              {busy[`load:${model.id}`] ? <span className="lm-installed"><LoaderCircle size={13} className="animate-spin" />{t('Loading…')}</span>
                : running ? <button type="button" className="button subtle small" onClick={() => props.onUnload(model.id)}>{t('Unload')}</button>
                  : <button type="button" className="button subtle small" onClick={() => props.onLoad(model.id)} title={t('Load it now so the first reply does not wait')}>{t('Load')}</button>}
              {confirm === model.id
                ? <span className="lm-confirm"><button type="button" className="button danger small" onClick={() => { setConfirm(null); props.onDelete(model.id) }}>{t('Remove')}</button><button type="button" className="button subtle small" onClick={() => setConfirm(null)}>{t('Keep')}</button></span>
                : <button type="button" className="icon-button" onClick={() => setConfirm(model.id)} aria-label={t('Remove {name}', { name: model.id })} title={t('Remove')}><Trash2 size={14} /></button>}
            </span>
          </li>
        })}
      </ul>}

    <form className="lm-custom" onSubmit={event => { event.preventDefault(); if (repo.trim() && file.trim()) { props.onDownload(repo.trim(), file.trim()); setFile('') } }}>
      <input value={repo} onChange={event => setRepo(event.target.value)} placeholder={t('Hugging Face repository, e.g. unsloth/Qwen3-8B-GGUF')} spellCheck={false} aria-label={t('Hugging Face repository')} />
      <input value={file} onChange={event => setFile(event.target.value)} placeholder={t('File, e.g. Qwen3-8B-Q4_K_M.gguf')} spellCheck={false} aria-label={t('GGUF file')} />
      <button type="submit" className="button subtle small" disabled={!repo.trim() || !file.trim().endsWith('.gguf')}><Download size={13} />{t('Download')}</button>
    </form>
    {Object.values(downloads).filter(progress => !progress.done && !visible.some(pick => progress.key.startsWith(`${pick.repo}/`))).map(progress => <div key={progress.key} className="lm-other"><code>{progress.key}</code><Bar completed={progress.completed} total={progress.total} label={downloadLabel(progress)} onCancel={() => props.onCancel(progress.key)} /></div>)}

    {settings && <div className="lm-settings">
      <label>{t('Keep a model loaded after its last reply')}
        <select value={settings.keepWarmMinutes} onChange={event => props.onSettings({ ...settings, keepWarmMinutes: Number(event.target.value) })}>
          {[5, 15, 60, 0].map(minutes => <option key={minutes} value={minutes}>{minutes ? t('{n} minutes', { n: minutes }) : t('Until Neru quits')}</option>)}
        </select>
      </label>
      <label>{t('Context')}
        <select value={settings.context} onChange={event => props.onSettings({ ...settings, context: Number(event.target.value) })}>
          {[0, 8192, 16384, 32768, 65536, 131072].map(size => <option key={size} value={size}>{size ? `${size / 1024}K` : t('Automatic (what fits)')}</option>)}
        </select>
      </label>
      <label>{t('Run on')}
        <select value={settings.backend} onChange={event => props.onSettings({ ...settings, backend: event.target.value })}>
          {['auto', 'cpu', 'vulkan', 'cuda', 'metal'].map(backend => <option key={backend} value={backend}>{backend === 'auto' ? t('Automatic ({backend})', { backend: t(BACKENDS[status?.backend ?? 'cpu'] ?? 'cpu') }) : t(BACKENDS[backend])}</option>)}
        </select>
      </label>
      {status?.modelsDir && <p className="lm-empty">{t('Models are kept in {dir}.', { dir: status.modelsDir })}</p>}
    </div>}
  </div>
}

export function BuiltinModels({ onChanged }: { onChanged?: () => void }) {
  const [advice, setAdvice] = useState<BuiltinAdvice | null>(null)
  const [status, setStatus] = useState<BuiltinStatus | null>(null)
  const [downloads, setDownloads] = useState<Record<string, BuiltinDownload>>({})
  const [engine, setEngine] = useState<EngineProgress | null>(null)
  const [busy, setBusy] = useState<Record<string, boolean>>({})
  const [error, setError] = useState<string | null>(null)
  const changed = useRef(onChanged)
  useEffect(() => { changed.current = onChanged })
  const refresh = useCallback(() => { void api.builtinStatus().then(setStatus).catch(cause => setError(errorText(cause))) }, [])
  const flag = (key: string, on: boolean) => setBusy(current => ({ ...current, [key]: on }))
  const speed = useCallback((id: string) => {
    flag(`speed:${id}`, true)
    void api.builtinSpeed(id).then(refresh).catch(cause => setError(t('Could not measure {tag}: {error}', { tag: id, error: errorText(cause) }))).finally(() => flag(`speed:${id}`, false))
  }, [refresh])
  useEffect(() => {
    void api.builtinAdvice().then(setAdvice).catch(cause => setError(errorText(cause)))
    refresh()
  }, [refresh])
  useEffect(() => {
    const stopDownload = api.onBuiltinDownload(progress => {
      setDownloads(current => ({ ...current, [progress.key]: progress }))
      if (progress.done) {
        if (progress.error) setError(t('{tag} did not download: {error}', { tag: progress.key, error: progress.error }))
        refresh()
        changed.current?.()
      }
    })
    const stopEngine = api.onEngineProgress(progress => {
      setEngine(progress)
      if (progress.done) { if (progress.error) setError(progress.error); refresh() }
    })
    return () => { for (const stop of [stopDownload, stopEngine]) void stop.then(unlisten => unlisten()).catch(() => undefined) }
  }, [refresh])
  // While a model loads or a download runs, the loaded list changes; a slow poll keeps it current.
  useEffect(() => {
    const timer = window.setInterval(refresh, 15_000)
    return () => window.clearInterval(timer)
  }, [refresh])
  const download = (repo: string, file: string) => {
    setError(null)
    const key = keyOf({ repo, file })
    setDownloads(current => ({ ...current, [key]: { key, id: '', status: 'starting', completed: 0, total: 0, speed: 0, done: false, error: null } }))
    // The first download also fetches the engine, so the first message is ready when it finishes.
    if (!status?.engine) void api.builtinInstallEngine().catch(cause => setError(errorText(cause)))
    void api.builtinDownload(repo, file).then(model => speed(model.id)).catch(cause => {
      const message = errorText(cause)
      setDownloads(current => ({ ...current, [key]: { ...current[key], done: true, error: message } }))
      if (message !== 'cancelled') setError(message)
    })
  }
  const load = (id: string) => {
    flag(`load:${id}`, true); setError(null)
    void api.builtinLoad(id).then(refresh).catch(cause => setError(errorText(cause))).finally(() => flag(`load:${id}`, false))
  }
  return <BuiltinModelsView advice={advice} status={status} downloads={downloads} engine={engine} busy={busy} error={error}
    onDownload={download} onCancel={key => void api.builtinCancelDownload(key)}
    onDelete={id => { setError(null); void api.builtinDelete(id).then(() => { refresh(); changed.current?.() }).catch(cause => setError(errorText(cause))) }}
    onLoad={load} onUnload={id => void api.builtinUnload(id).then(refresh)} onSpeed={speed}
    onInstallEngine={() => { setError(null); void api.builtinInstallEngine().then(refresh).catch(cause => setError(errorText(cause))) }}
    onSettings={next => void api.builtinSetSettings(next).then(() => refresh()).catch(cause => setError(errorText(cause)))}
    onRefresh={() => { refresh(); void api.builtinAdvice().then(setAdvice).catch(() => undefined) }} />
}
