import { useCallback, useEffect, useRef, useState } from 'react'
import { Check, Cpu, Download, ExternalLink, LoaderCircle, RotateCw, Trash2, X } from '@/components/icons'
import { api } from '../../api'
import type { LocalAdvice, LocalPick, OllamaStatus, PullProgress } from '../../types'
import { cn } from '@/lib/utils'
import { t } from '../../i18n'
import './LocalModels.css'

// The local model library under Settings → Model provider → Ollama: what this computer can run, whether
// Ollama is there, and downloading or removing models without a terminal (ollama.rs, hardware.rs).

const ROLES: Record<string, { label: string; hint: string }> = {
  agent: { label: 'For Code', hint: 'Can drive the agent itself. Pick it in step 3 to use it.' },
  helper: { label: 'Helper', hint: 'Takes titles, summaries, commit messages and repairs automatically, so they cost no free requests.' },
}

const size = (gb: number) => gb >= 10 ? `${gb.toFixed(0)} GB` : `${gb.toFixed(1)} GB`
const errorText = (cause: unknown) => cause instanceof Error ? cause.message : String(cause)

function Progress({ progress, onCancel }: { progress: PullProgress; onCancel: () => void }) {
  const share = progress.total ? progress.completed / progress.total : 0
  return <span className="lm-progress">
    <span className="lm-bar" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(share * 100)}><i style={{ width: `${share * 100}%` }} /></span>
    <small>{progress.total ? t('{done} of {total}', { done: size(progress.completed / 1_073_741_824), total: size(progress.total / 1_073_741_824) }) : t(progress.status)}</small>
    <button type="button" className="icon-button" onClick={onCancel} aria-label={t('Cancel download')} title={t('Cancel download')}><X size={13} /></button>
  </span>
}

export function LocalModelsView({ advice, status, pulls, error, busy, onPull, onCancel, onDelete, onStart, onRefresh }: {
  advice: LocalAdvice | null; status: OllamaStatus | null; pulls: Record<string, PullProgress>; error: string | null; busy: boolean
  onPull: (tag: string) => void; onCancel: (tag: string) => void; onDelete: (tag: string) => void; onStart: () => void; onRefresh: () => void
}) {
  const [confirm, setConfirm] = useState<string | null>(null)
  const [custom, setCustom] = useState('')
  const installed = new Set(status?.models.map(model => model.name) ?? [])
  // Neru uses local models for Code and for chores today; completion and embeddings come later.
  const picks = (advice?.picks ?? []).filter((pick): pick is LocalPick => pick.role in ROLES)
  const hardware = advice?.hardware
  const graphics = hardware?.gpus.find(gpu => gpu.dedicated)
  const row = (tag: string, action: React.ReactNode) => pulls[tag] && !pulls[tag].done ? <Progress progress={pulls[tag]} onCancel={() => onCancel(tag)} /> : action
  return <div className="local-models">
    {advice && hardware && <div className="lm-machine">
      <Cpu size={16} aria-hidden />
      <div>
        <p className="lm-specs">{[t('{n} GB memory', { n: Math.round(hardware.ramGb) }), hardware.unified ? t('Apple silicon, shared memory') : graphics ? `${graphics.name} (${Math.round(graphics.vramGb)} GB)` : t('no graphics card with its own memory'), hardware.diskFreeGb !== null ? t('{n} GB free for models', { n: Math.round(hardware.diskFreeGb) }) : null].filter(Boolean).join(' · ')}</p>
        <p>{advice.verdict}</p>
      </div>
    </div>}

    <div className={cn('lm-status', status?.running && 'is-running')} role="status">
      {!status ? <span><LoaderCircle size={14} className="animate-spin" /> {t('Looking for Ollama…')}</span>
        : status.running ? <span><Check size={14} /> {status.version ? t('Ollama {version} is running.', { version: status.version }) : t('Ollama is running.')}</span>
          : status.installed ? <><span>{t('Ollama is installed but not running.')}</span><button type="button" className="button subtle small" onClick={onStart} disabled={busy}>{busy ? <LoaderCircle size={13} className="animate-spin" /> : null}{t('Start Ollama')}</button></>
            : <><span>{t('Ollama runs the models on this computer. Install it, then come back.')}</span><a className="button subtle small" href="https://ollama.com/download" target="_blank" rel="noreferrer">{t('Get Ollama')}<ExternalLink size={12} /></a></>}
      <button type="button" className="icon-button" onClick={onRefresh} aria-label={t('Check again')} title={t('Check again')}><RotateCw size={13} /></button>
    </div>
    {error && <p className="lm-error" role="alert">{error}</p>}

    {picks.length > 0 && <>
      <p className="lm-subhead">{t('Recommended for this computer')}</p>
      <ul className="lm-list">
        {picks.map(pick => <li key={pick.tag}>
          <span className="lm-name"><code>{pick.tag}</code><small title={t(ROLES[pick.role].hint)}>{t(ROLES[pick.role].label)} · {size(pick.sizeGb)} · {t(pick.runsOn === 'gpu' ? 'about {n} tokens/s on the graphics card' : 'about {n} tokens/s on the processor', { n: pick.tokensPerSecond })}</small></span>
          {row(pick.tag, installed.has(pick.tag)
            ? <span className="lm-installed"><Check size={13} />{t('Installed')}</span>
            : <button type="button" className="button subtle small" onClick={() => onPull(pick.tag)} disabled={!status?.running} aria-label={t('Download {tag}', { tag: pick.tag })}><Download size={13} />{t('Download')}</button>)}
        </li>)}
      </ul>
    </>}

    {status?.running && <>
      <p className="lm-subhead">{t('On this computer')}</p>
      {status.models.length === 0 ? <p className="lm-empty">{t('No models yet. Download one above.')}</p>
        : <ul className="lm-list">
          {status.models.map(model => <li key={model.name}>
            <span className="lm-name"><code>{model.name}</code><small>{[model.parameters, model.quantization, size(model.sizeGb)].filter(Boolean).join(' · ')}</small></span>
            {confirm === model.name
              ? <span className="lm-confirm"><button type="button" className="button danger small" onClick={() => { setConfirm(null); onDelete(model.name) }}>{t('Remove')}</button><button type="button" className="button subtle small" onClick={() => setConfirm(null)}>{t('Keep')}</button></span>
              : <button type="button" className="icon-button" onClick={() => setConfirm(model.name)} aria-label={t('Remove {name}', { name: model.name })} title={t('Remove')}><Trash2 size={14} /></button>}
          </li>)}
        </ul>}
      <form className="lm-custom" onSubmit={event => { event.preventDefault(); if (custom.trim()) { onPull(custom.trim()); setCustom('') } }}>
        <input value={custom} onChange={event => setCustom(event.target.value)} placeholder={t('Other model, e.g. mistral:7b')} spellCheck={false} aria-label={t('Model to download')} />
        <button type="submit" className="button subtle small" disabled={!custom.trim()}><Download size={13} />{t('Download')}</button>
      </form>
      {Object.values(pulls).filter(progress => !progress.done && !picks.some(pick => pick.tag === progress.tag)).map(progress => <div key={progress.tag} className="lm-other"><code>{progress.tag}</code><Progress progress={progress} onCancel={() => onCancel(progress.tag)} /></div>)}
    </>}
  </div>
}

export function LocalModels({ onChanged }: { onChanged?: () => void }) {
  const [advice, setAdvice] = useState<LocalAdvice | null>(null)
  const [status, setStatus] = useState<OllamaStatus | null>(null)
  const [pulls, setPulls] = useState<Record<string, PullProgress>>({})
  const [error, setError] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)
  // The parent passes a new function every render; keep the latest without resubscribing.
  const changed = useRef(onChanged)
  useEffect(() => { changed.current = onChanged })
  const refresh = useCallback(() => { void api.ollamaStatus().then(setStatus).catch(cause => setError(errorText(cause))) }, [])
  useEffect(() => {
    void api.localAdvice().then(setAdvice).catch(() => undefined)
    refresh()
  }, [refresh])
  useEffect(() => {
    const stop = api.onOllamaPull(progress => {
      setPulls(current => ({ ...current, [progress.tag]: progress }))
      if (progress.done) {
        if (progress.error && progress.status !== 'cancelled') setError(t('{tag} did not download: {error}', { tag: progress.tag, error: progress.error }))
        refresh()
        changed.current?.()
      }
    })
    return () => { void stop.then(unlisten => unlisten()).catch(() => undefined) }
  }, [refresh])
  const pull = (tag: string) => {
    setError(null)
    setPulls(current => ({ ...current, [tag]: { tag, status: 'starting', completed: 0, total: 0, done: false, error: null } }))
    void api.ollamaPull(tag).catch(cause => {
      const message = errorText(cause)
      setPulls(current => ({ ...current, [tag]: { ...current[tag], done: true, error: message } }))
      if (message !== 'cancelled') setError(message)
    })
  }
  const remove = (tag: string) => { setError(null); void api.ollamaDelete(tag).then(() => { refresh(); changed.current?.() }).catch(cause => setError(errorText(cause))) }
  const start = () => { setBusy(true); setError(null); void api.ollamaStart().then(refresh).catch(cause => setError(errorText(cause))).finally(() => setBusy(false)) }
  return <LocalModelsView advice={advice} status={status} pulls={pulls} error={error} busy={busy} onPull={pull} onCancel={tag => void api.ollamaCancelPull(tag)} onDelete={remove} onStart={start} onRefresh={refresh} />
}
