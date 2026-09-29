import { useCallback, useEffect, useState, useSyncExternalStore } from 'react'
import { Check, Cpu, Download, Gauge, LoaderCircle, MonitorCog, Trash2 } from 'lucide-react'
import { motion, useReducedMotion } from 'motion/react'
import { findSpeechModel, speechModels, type SpeechModel } from '@/lib/speech/catalog'
import { downloadModel, isModelCached, removeModel, type DownloadProgress } from '@/lib/speech/local'
import { EASE_OUT } from '@/lib/ease'
import { cn } from '@/lib/utils'

const ACTIVE_KEY = 'neru.speech.model'

type ModelState = { status: 'absent' } | { status: 'checking' } | { status: 'downloading'; progress: DownloadProgress | null } | { status: 'installed' } | { status: 'error'; message: string }

// Module-level store so the onboarding and Settings catalogs share one download state.
const states = new Map<string, ModelState>()
let active: string | null = (() => { try { return localStorage.getItem(ACTIVE_KEY) } catch { return null } })()
const subscribers = new Set<() => void>()
let version = 0
const notify = () => { version += 1; subscribers.forEach(callback => callback()) }
const subscribe = (callback: () => void) => { subscribers.add(callback); return () => subscribers.delete(callback) }
const setState = (id: string, state: ModelState) => { states.set(id, state); notify() }

export function activeSpeechModel(): SpeechModel | undefined {
  const model = findSpeechModel(active)
  return model && states.get(model.id)?.status === 'installed' ? model : undefined
}

export function setActiveSpeechModel(id: string | null) {
  active = id
  try { if (id) localStorage.setItem(ACTIVE_KEY, id); else localStorage.removeItem(ACTIVE_KEY) } catch { /* storage unavailable */ }
  notify()
}

let checked = false
export async function refreshInstalledModels() {
  if (checked) return
  checked = true
  await Promise.all(speechModels.map(async model => {
    if (states.get(model.id)?.status === 'downloading') return
    setState(model.id, { status: 'checking' })
    setState(model.id, { status: (await isModelCached(model).catch(() => false)) ? 'installed' : 'absent' })
  }))
}

export async function installSpeechModel(model: SpeechModel) {
  setState(model.id, { status: 'downloading', progress: null })
  try {
    await navigator.storage?.persist?.().catch(() => false)
    await downloadModel(model, progress => setState(model.id, { status: 'downloading', progress }))
    setState(model.id, { status: 'installed' })
    if (!activeSpeechModel()) setActiveSpeechModel(model.id)
  } catch (cause) {
    setState(model.id, { status: 'error', message: cause instanceof Error ? cause.message : String(cause) })
  }
}

/** Subscribes a component to the shared model catalog state. */
export function useSpeechModels() {
  useSyncExternalStore(subscribe, () => version)
  useEffect(() => { void refreshInstalledModels() }, [])
  return { states, active: activeSpeechModel()?.id ?? null }
}

const formatMb = (bytes: number) => `${Math.round(bytes / 1e6)} MB`

function Meter({ value, label }: { value: number; label: string }) {
  return <span className="speech-meter" aria-label={`${label} ${value} of 5`} title={`${label}: ${value}/5`}>{[1, 2, 3, 4, 5].map(step => <i key={step} className={step <= value ? 'on' : ''} />)}</span>
}

export function SpeechModelCatalog({ compact = false, onChange }: { compact?: boolean; onChange?: () => void }) {
  const reduce = useReducedMotion() ?? false
  const { states: modelStates, active: activeId } = useSpeechModels()
  const [gpu, setGpu] = useState<boolean | null>(null)
  useEffect(() => {
    const probe = (navigator as Navigator & { gpu?: { requestAdapter: () => Promise<unknown> } }).gpu
    if (!probe) { setGpu(false); return }
    void probe.requestAdapter().then(adapter => setGpu(Boolean(adapter))).catch(() => setGpu(false))
  }, [])
  const remove = useCallback(async (model: SpeechModel) => {
    await removeModel(model)
    setState(model.id, { status: 'absent' })
    if (active === model.id) setActiveSpeechModel(speechModels.find(item => modelStates.get(item.id)?.status === 'installed')?.id ?? null)
    onChange?.()
  }, [modelStates, onChange])

  return <div className={cn('speech-catalog', compact && 'compact')} role="list">
    {speechModels.map(model => {
      const state = modelStates.get(model.id) ?? { status: 'checking' as const }
      const installed = state.status === 'installed'
      const isActive = installed && activeId === model.id
      const unavailable = model.requiresGpu && gpu === false
      const percent = state.status === 'downloading' && state.progress ? Math.min(100, state.progress.progress) : 0
      return <div role="listitem" key={model.id} className={cn('speech-model', isActive && 'active', unavailable && 'unavailable')}>
        <div className="speech-model-main">
          <div className="speech-model-title">
            <strong>{model.name}</strong>
            {model.recommended && <span className="speech-tag accent">Recommended</span>}
            <span className="speech-tag">{model.languages === 'english' ? 'English' : '99 languages'}</span>
            {model.requiresGpu && <span className="speech-tag" title={gpu === false ? 'No WebGPU adapter was found on this machine' : 'Runs on your GPU'}>{gpu === false ? <Cpu size={11} /> : <MonitorCog size={11} />}{gpu === false ? 'GPU not found' : 'GPU'}</span>}
          </div>
          {!compact && <p>{model.summary}</p>}
          <div className="speech-model-meta">
            <span>{model.sizeMb} MB</span>
            <span className="speech-meter-group"><Gauge size={12} />Speed <Meter value={model.speed} label="Speed" /></span>
            <span className="speech-meter-group">Accuracy <Meter value={model.accuracy} label="Accuracy" /></span>
          </div>
          {state.status === 'downloading' && <div className="speech-progress" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(percent)} aria-label={`Downloading ${model.name}`}>
            <motion.span initial={false} animate={{ scaleX: percent / 100 }} transition={{ duration: reduce ? 0 : 0.25, ease: EASE_OUT }} />
            <small>{state.progress ? `${formatMb(state.progress.loaded)} of ${formatMb(state.progress.total)}` : 'Starting download…'}</small>
          </div>}
          {state.status === 'error' && <p className="speech-error" role="alert">{state.message}</p>}
        </div>
        <div className="speech-model-actions">
          {state.status === 'checking' && <LoaderCircle size={16} className={cn('speech-spinner', !reduce && 'animate-spin')} aria-label="Checking" />}
          {(state.status === 'absent' || state.status === 'error') && <button type="button" className="speech-button" disabled={unavailable} onClick={() => void installSpeechModel(model).then(() => onChange?.())}><Download size={14} />{state.status === 'error' ? 'Retry' : 'Download'}</button>}
          {state.status === 'downloading' && <span className="speech-percent">{Math.round(percent)}%</span>}
          {installed && (isActive
            ? <span className="speech-active"><Check size={14} />In use</span>
            : <button type="button" className="speech-button" onClick={() => { setActiveSpeechModel(model.id); onChange?.() }}>Use</button>)}
          {installed && <button type="button" className="speech-icon" onClick={() => void remove(model)} aria-label={`Remove ${model.name}`} title="Remove from this machine"><Trash2 size={14} /></button>}
        </div>
      </div>
    })}
  </div>
}
