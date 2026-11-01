import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type ReactNode } from 'react'
import { ArrowUp, Check, ChevronDown, FileCode2, FileSearch, FolderSearch, GitCompareArrows, Globe, GraduationCap, LoaderCircle, Mic, Pause, Play, Plus, Upload, X } from '@/components/icons'
import { Liquid } from 'liquid-gooey'
import { useReducedMotion } from 'motion/react'
import { useMicrophone } from 'voice-glow'
import { PromptInput } from '@/components/agents/prompt-input'
import { Select, SelectContent, SelectItem, SelectTrigger } from '@/components/motion/select'
import { SoundBars } from './SoundBars'
import { cn } from '@/lib/utils'
import type { AgentMode, ContextUsage, Effort, ModelInfo, SlashCommand } from '../../types'
import { t } from '../../i18n'
import { ModelSelector } from './ModelSelector'

export type VoiceEngine = 'local' | 'transcription' | 'system'
type VoiceState = 'idle' | 'listening' | 'paused' | 'transcribing'

interface SpeechRecognitionLike {
  lang: string; continuous: boolean; interimResults: boolean
  onresult: ((event: { resultIndex: number; results: ArrayLike<ArrayLike<{ transcript: string }> & { isFinal: boolean }> }) => void) | null
  onerror: ((event: { error: string }) => void) | null
  onend: (() => void) | null
  start: () => void; stop: () => void; abort: () => void
}
type SpeechRecognitionConstructor = new () => SpeechRecognitionLike
const SpeechRecognition = (window as unknown as { SpeechRecognition?: SpeechRecognitionConstructor; webkitSpeechRecognition?: SpeechRecognitionConstructor }).SpeechRecognition
  ?? (window as unknown as { webkitSpeechRecognition?: SpeechRecognitionConstructor }).webkitSpeechRecognition
export const systemSpeechAvailable = Boolean(SpeechRecognition)

const RECORDING_TYPES = ['audio/webm;codecs=opus', 'audio/webm', 'audio/ogg;codecs=opus', 'audio/mp4']
const pickRecordingType = () => typeof MediaRecorder === 'undefined' ? '' : RECORDING_TYPES.find(type => MediaRecorder.isTypeSupported(type)) ?? ''
const joinText = (base: string, addition: string) => !addition ? base : !base.trim() ? addition : `${base.replace(/\s+$/, '')} ${addition}`
const clock = (ms: number) => { const seconds = Math.floor(ms / 1000); return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, '0')}` }

export interface PlusMenuItem { id: string; label: string; hint?: string; icon: ReactNode; disabled?: boolean; onSelect: () => void }

const ITEM_GAP = 48
/** The + sits under the prompt box, so the pills start above the box instead of covering it. */
const BOX_CLEARANCE = 50

/** The + button: options flow out of it as separate liquid pills (liquid-gooey) and melt back when it closes. The + itself stays bare. */
function PlusMenu({ items, disabled }: { items: PlusMenuItem[]; disabled: boolean }) {
  const reduce = useReducedMotion() ?? false
  const [mounted, setMounted] = useState(false)
  const [open, setOpen] = useState(false)
  const root = useRef<HTMLDivElement>(null)
  const timer = useRef<number | undefined>(undefined)
  // Where the pills go: up, clearing the prompt box when there is room under the title bar; up but
  // closer when the window is short; down when even that would run past the top.
  const [layout, setLayout] = useState<{ down: boolean; clearance: number; list?: { up: boolean; height: number } }>({ down: false, clearance: BOX_CLEARANCE })
  const measure = () => {
    const rect = root.current?.getBoundingClientRect()
    if (!rect) return
    const ceiling = (document.querySelector('.titlebar')?.getBoundingClientRect().bottom ?? 0) + 8
    const above = rect.top - ceiling
    const stack = ITEM_GAP * items.length + 4
    const below = window.innerHeight - rect.bottom - 12
    if (above >= stack + BOX_CLEARANCE) setLayout({ down: false, clearance: BOX_CLEARANCE })
    else if (above >= stack) setLayout({ down: false, clearance: Math.max(0, above - stack) })
    else if (below >= stack + 40) setLayout({ down: true, clearance: 0 })
    // Too short a window for the pills either way: a plain list on the roomier side, scrolling if it must.
    else setLayout({ down: false, clearance: 0, list: { up: above >= below, height: Math.max(80, Math.max(above, below) - 8) } })
  }
  const show = () => { window.clearTimeout(timer.current); measure(); setMounted(true); requestAnimationFrame(() => requestAnimationFrame(() => setOpen(true))) }
  const hide = () => { setOpen(false); window.clearTimeout(timer.current); timer.current = window.setTimeout(() => setMounted(false), reduce ? 0 : 480) }
  useEffect(() => {
    if (!open) return
    const close = (event: PointerEvent) => { if (!root.current?.contains(event.target as Node)) hide() }
    const escape = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.stopPropagation(); hide() } }
    window.addEventListener('pointerdown', close)
    window.addEventListener('keydown', escape, true)
    return () => { window.removeEventListener('pointerdown', close); window.removeEventListener('keydown', escape, true) }
  })
  useEffect(() => { if (disabled && open) hide() })
  useEffect(() => () => window.clearTimeout(timer.current), [])
  const transition = reduce ? { duration: 0 } : 'smooth' as const
  // Collapsed, every pill shrinks to one point on the prompt box's top edge, straight above the +,
  // so the options grow out of the box as a single drop and melt back into it. Width is estimated from the label.
  const collapsedX = (label: string) => 16 - (49 + label.length * 7) / 2
  const origin = -(BOX_CLEARANCE + 21)
  return <div className={cn('plus-menu', open && 'open')} ref={root}>
    <div className="plus-anchor" style={{ height: 32 + BOX_CLEARANCE + ITEM_GAP * items.length + 12 }}>
    <Liquid className="plus-liquid" blur={6} contrast={20} fill="var(--plus-fill)" shadow="0 10px 28px rgb(0 0 0 / 0.28), inset 0 0 0 1px var(--plus-ring)" filterPadding={48}>
      {mounted && !layout.list && items.map((item, index) => <Liquid.Item key={item.id} className="plus-item-slot" x={open ? 0 : collapsedX(item.label)} y={open ? (layout.down ? 40 + ITEM_GAP * index : -(layout.clearance + ITEM_GAP * (items.length - index))) : origin} scale={open ? 1 : 0} transition={transition} delay={open ? (items.length - 1 - index) * 40 : index * 30}>
        <button type="button" role="menuitem" className="plus-item" tabIndex={open ? 0 : -1} aria-hidden={!open} disabled={item.disabled} title={item.hint} onClick={() => { hide(); item.onSelect() }}>{item.icon}<span>{item.label}</span></button>
      </Liquid.Item>)}
    </Liquid>
    </div>
    {open && layout.list && <div className={cn('plus-list', !layout.list.up && 'down')} role="menu" style={{ maxHeight: layout.list.height }}>
      {items.map(item => <button key={item.id} type="button" role="menuitem" className="plus-list-item" disabled={item.disabled} title={item.hint} onClick={() => { hide(); item.onSelect() }}>{item.icon}<span>{item.label}</span></button>)}
    </div>}
    <button type="button" className="plus-trigger" aria-label={t('Add to message')} aria-haspopup="menu" aria-expanded={open} disabled={disabled} onClick={() => open ? hide() : show()}><Plus size={17} /></button>
  </div>
}

const MIC_KEY = 'neru.voice.microphone'
const HOLD_KEY = 'neru.voice.hold'
const readPref = (key: string) => { try { return localStorage.getItem(key) ?? '' } catch { return '' } }
const writePref = (key: string, value: string) => { try { if (value) localStorage.setItem(key, value); else localStorage.removeItem(key) } catch { /* not saved */ } }

/** Audio inputs, with names. Browsers hide the names until the page may use the microphone, so ask once when they are missing. */
async function listMicrophones(): Promise<MediaDeviceInfo[]> {
  const inputs = async () => (await navigator.mediaDevices.enumerateDevices()).filter(device => device.kind === 'audioinput')
  let found = await inputs()
  if (found.length && found.every(device => !device.label)) {
    try { (await navigator.mediaDevices.getUserMedia({ audio: true })).getTracks().forEach(track => track.stop()); found = await inputs() } catch { /* keep the unnamed list */ }
  }
  return found
}

/** The arrow beside the microphone: which input to record from, and whether the mic button records only while held. */
function MicMenu({ device, onDevice, hold, onHold, disabled, systemEngine }: {
  device: string
  onDevice: (id: string) => void
  hold: boolean
  onHold: (hold: boolean) => void
  disabled: boolean
  /** The system speech engine always listens to the Windows default input. */
  systemEngine: boolean
}) {
  const [open, setOpen] = useState(false)
  const [devices, setDevices] = useState<MediaDeviceInfo[]>([])
  const root = useRef<HTMLDivElement>(null)
  const panel = useRef<HTMLDivElement>(null)
  // Room to the window's right edge: a narrow window gets a narrower panel (names truncate) rather
  // than one that is cut off. Moving it left instead would slide it under the sidebar.
  const [room, setRoom] = useState(0)
  useLayoutEffect(() => {
    const node = panel.current
    if (!open || !node) return
    setRoom(Math.max(220, document.documentElement.clientWidth - 12 - node.getBoundingClientRect().left))
  }, [open])
  useEffect(() => {
    if (!open) return
    let live = true
    const load = () => void listMicrophones().then(list => { if (live) setDevices(list) }).catch(() => undefined)
    load()
    navigator.mediaDevices.addEventListener?.('devicechange', load)
    const close = (event: PointerEvent) => { if (!root.current?.contains(event.target as Node)) setOpen(false) }
    const escape = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.stopPropagation(); setOpen(false) } }
    window.addEventListener('pointerdown', close)
    window.addEventListener('keydown', escape, true)
    return () => { live = false; navigator.mediaDevices.removeEventListener?.('devicechange', load); window.removeEventListener('pointerdown', close); window.removeEventListener('keydown', escape, true) }
  }, [open])
  useEffect(() => { if (disabled) setOpen(false) }, [disabled])
  // A saved device that is gone (headset unplugged) falls back to the system default, so show that as chosen.
  const chosen = devices.some(item => item.deviceId === device) ? device : (devices.find(item => item.deviceId === 'default') ?? devices[0])?.deviceId ?? ''
  return <div className="mic-menu" ref={root}>
    <button type="button" className={cn('mic-menu-trigger', open && 'on')} aria-label={t('Microphone settings')} aria-haspopup="menu" aria-expanded={open} title={t('Microphone settings')} disabled={disabled} onClick={() => setOpen(value => !value)}><ChevronDown size={13} /></button>
    {open && <div ref={panel} className="mic-menu-panel" role="menu" aria-label={t('Microphone')} style={room ? { width: Math.min(380, room) } : undefined}>
      <div className="mic-menu-label">{t('Microphone')}</div>
      {devices.length === 0 && <p className="mic-menu-empty">{t('No microphone found')}</p>}
      {devices.map((item, index) => <button key={item.deviceId || index} type="button" role="menuitemradio" aria-checked={item.deviceId === chosen} title={item.label || undefined}
        onClick={() => { onDevice(item.deviceId === 'default' ? '' : item.deviceId); setOpen(false) }}>
        <span className="truncate">{item.label || t('Microphone {n}', { n: index + 1 })}</span>{item.deviceId === chosen && <Check size={14} />}
      </button>)}
      {systemEngine && <p className="mic-menu-note">{t('System speech always uses the Windows default input. Pick another engine in Settings → Voice to use this choice.')}</p>}
      <div className="mic-menu-sep" />
      <button type="button" role="menuitemcheckbox" aria-checked={hold} onClick={() => onHold(!hold)}>
        <span>{t('Hold to record')}</span><i className={cn('switch', hold && 'on')} aria-hidden />
      </button>
    </div>}
  </div>
}

export const MODES: { value: AgentMode; label: string; hint: string }[] = [
  { value: 'manual', label: 'Ask permissions', hint: 'Neru asks before every edit and command' },
  { value: 'accept_edits', label: 'Accept edits', hint: 'Edits apply right away; commands still ask' },
  { value: 'plan', label: 'Plan mode', hint: 'Read-only: explores and proposes a plan' },
  { value: 'spec', label: 'Spec mode', hint: 'Requirements, design and tasks you review, then the code' },
  { value: 'auto', label: 'Auto', hint: 'Edits, build, test and lint run; other commands ask' },
  { value: 'bypass', label: 'Bypass permissions', hint: 'Everything runs without asking. Use with care' },
]

/** Fills a command template: `$ARGUMENTS` becomes what was typed after the name. */
export function expandCommand(command: SlashCommand, args: string) {
  return command.template.includes('$ARGUMENTS') ? command.template.replaceAll('$ARGUMENTS', args) : args ? `${command.template}\n\n${args}` : command.template
}

const compact = (tokens: number) => tokens >= 1_000_000 ? `${(tokens / 1_000_000).toFixed(tokens % 1_000_000 ? 1 : 0)}M` : tokens >= 1000 ? `${Math.round(tokens / 1000)}k` : String(tokens)

const count = (value: number) => value.toLocaleString()

/** Claude-style context ring: how much of what one request may hold this conversation already uses,
 *  with the model's window and the key's rate limits in a panel on hover or focus. */
function ContextMeter({ usage }: { usage: ContextUsage }) {
  const limit = Math.max(1, usage.limit ?? usage.window)
  const share = Math.min(1, usage.used / limit)
  const percent = Math.round(share * 100)
  const radius = 6.5
  const length = 2 * Math.PI * radius
  const capped = usage.limit !== undefined && usage.limit < usage.window
  const quotas = usage.quotas ?? []
  const low = quotas.some(quota => quota.limit && quota.remaining !== null && quota.remaining / quota.limit < 0.1)
  const tone = share > 0.9 || low ? 'full' : share > 0.7 ? 'high' : ''
  const label = usage.measured ? t('{used} of {limit} tokens used ({percent}%)', { used: compact(usage.used), limit: compact(limit), percent }) : t('About {used} of {limit} tokens used ({percent}%)', { used: compact(usage.used), limit: compact(limit), percent })
  return <span className={cn('context-meter', tone)} tabIndex={0} role="button" aria-label={t('Context: {label}', { label })}>
    <svg width="18" height="18" viewBox="0 0 18 18" aria-hidden><circle cx="9" cy="9" r={radius} className="track" /><circle cx="9" cy="9" r={radius} className="fill" strokeDasharray={`${Math.max(share * length, share > 0 ? 1.5 : 0)} ${length}`} transform="rotate(-90 9 9)" /></svg>
    <span className="context-panel" role="tooltip">
      <strong>{usage.model || t('Context')}</strong>
      <span className="context-row"><span>{t('This conversation')}</span><b>{t('{n} tokens · {percent}%', { n: `${usage.measured ? '' : '~'}${count(usage.used)}`, percent })}</b></span>
      <span className="context-row"><span>{t('Context window')}</span><b>{usage.windowReported ? count(usage.window) : t('{n} (estimate)', { n: count(usage.window) })}</b></span>
      {capped && <span className="context-row"><span>{t('Per-request limit')}</span><b>{count(limit)}</b></span>}
      {capped && <small>{t('This key allows {n} tokens a minute, so each request must fit in that. Neru trims and summarizes to stay under it.', { n: compact(limit) })}</small>}
      {quotas.length > 0 && <span className="context-quotas">{quotas.map(quota => {
        const left = quota.limit && quota.remaining !== null ? Math.max(0, Math.min(1, quota.remaining / quota.limit)) : null
        return <span key={quota.label} className="context-quota">
          <span className="context-row"><span>{quota.label}</span><b>{quota.limit ? t('{n} of {limit} left', { n: quota.remaining !== null ? count(quota.remaining) : '?', limit: count(quota.limit) }) : t('{n} left', { n: quota.remaining !== null ? count(quota.remaining) : '?' })}</b></span>
          {left !== null && <i className={cn(left < 0.1 && 'low')} style={{ ['--left' as string]: `${left * 100}%` }} />}
          {quota.resetsIn && <small>{t('Resets in {time}', { time: quota.resetsIn })}</small>}
        </span>
      })}</span>}
      {quotas.length === 0 && <small>{t('Rate limits appear here after the first reply, when the provider reports them.')}</small>}
      {share > 0.9 && <small>{t('Nearly full. Start a new session, rewind, or run /compact.')}</small>}
    </span>
  </span>
}

export interface ComposerProps {
  value: string
  onValueChange: (value: string) => void
  onSubmit: (value: string) => void
  onStop: () => void
  /** Sends a message into the running reply. */
  onSteer?: (value: string) => void
  /** Ctrl+Enter or "Queue for later": send once the running reply finishes. */
  onQueue?: (value: string) => void
  /** Ctrl+Alt+Enter or "Send in a forked session". */
  onFork?: (value: string) => void
  /** Attachment previews, rendered inside the box above the text. */
  attachments?: ReactNode
  /** An attachment (an image) is enough to send without typing anything. */
  sendWithoutText?: boolean
  loading: boolean
  disabled: boolean
  placeholder: string
  light: boolean
  mode: AgentMode
  onModeChange: (mode: AgentMode) => void
  /** Built-in and custom slash commands offered when the message starts with /. */
  commands: SlashCommand[]
  /** Runs a slash command; return false to send the text as a normal message. */
  onCommand: (command: SlashCommand, args: string) => boolean
  web: boolean
  onWebChange: (web: boolean) => void
  model: string
  models: ModelInfo[]
  /** Which provider serves the list, for its logo and the speed estimates. */
  providerId?: string
  onModelChange: (model: string) => void
  /** Result of checking a model the user picked: shown under the prompt until the next pick. */
  modelNotice?: { tone: 'checking' | 'error' | 'note'; text: string } | null
  attachItems: PlusMenuItem[]
  voiceEngine: VoiceEngine
  /** False when the chosen engine cannot run yet (no local model downloaded). */
  voiceReady: boolean
  onVoiceStart: () => void
  onVoiceUnavailable: () => void
  onTranscribe: (audio: Blob) => Promise<string>
  onError: (message: string) => void
  effort: Effort
  /** False when the selected model does not take an effort setting. */
  effortSupported: boolean
  onEffortChange: (effort: Effort) => void
  context: ContextUsage | null
  projectFiles?: string[]
  onAttachFile?: (path: string) => void
  /** Connector resources offered after @ next to project files; `key` comes back in onAttachResource. */
  resources?: { key: string; name: string; server: string }[]
  onAttachResource?: (key: string) => void
  /** Taller prompt card used on the empty chat page. */
  roomy?: boolean
  /** Team: agents bring their own web tools, so there is no web toggle. */
  showWeb?: boolean
  /** Chat's Research mode: academic sources, a research plan and a cited report. Hidden when undefined. */
  research?: boolean
  onResearchChange?: (research: boolean) => void
  /** Runs before the composer's own keys; preventDefault() stops them. */
  onKeyDown?: (event: ReactKeyboardEvent<HTMLTextAreaElement>) => void
  ariaLabel?: string
  /** Permission mode is a coding control. Chat hides it. */
  showMode?: boolean
}

export function Composer(props: ComposerProps) {
  const { value, onValueChange, loading, disabled, mode, web } = props
  const [micDevice, setMicDevice] = useState(() => readPref(MIC_KEY))
  const [holdToRecord, setHoldToRecord] = useState(() => readPref(HOLD_KEY) === '1')
  // Read when recording starts, so a new choice applies to the next recording. A plain deviceId is
  // a preference: an unplugged device falls back to the default instead of failing.
  const mic = useMicrophone({ constraints: micDevice ? { deviceId: micDevice } : undefined })
  const chooseMic = (id: string) => { setMicDevice(id); writePref(MIC_KEY, id) }
  const chooseHold = (hold: boolean) => { setHoldToRecord(hold); writePref(HOLD_KEY, hold ? '1' : '') }
  // Hold to record: the pointer went down on the mic and has not come up yet.
  const holding = useRef(false)
  // This recording was started by a press (not the keyboard), so letting go ends it.
  const pressed = useRef(false)
  const [voice, setVoice] = useState<VoiceState>('idle')
  const [elapsed, setElapsed] = useState(0)
  const recorder = useRef<MediaRecorder | null>(null)
  const chunks = useRef<Blob[]>([])
  const recognition = useRef<SpeechRecognitionLike | null>(null)
  const baseText = useRef('')
  const valueRef = useRef(value)
  const run = useRef(0)
  const clockState = useRef({ started: 0, banked: 0 })
  useEffect(() => { valueRef.current = value }, [value])

  useEffect(() => {
    if (voice !== 'listening') return
    const timer = window.setInterval(() => setElapsed(clockState.current.banked + performance.now() - clockState.current.started), 250)
    return () => window.clearInterval(timer)
  }, [voice])

  const release = () => { recorder.current = null; recognition.current = null; mic.stop() }

  const startVoice = async () => {
    if (voice !== 'idle' || disabled) return
    if (!props.voiceReady) { props.onVoiceUnavailable(); return }
    const stream = await mic.start()
    if (!stream) { props.onError(mic.error?.message || t('Microphone access was denied. Allow it in Windows privacy settings and try again.')); return }
    baseText.current = valueRef.current
    clockState.current = { started: performance.now(), banked: 0 }
    setElapsed(0)
    props.onVoiceStart()
    if (props.voiceEngine === 'system' && SpeechRecognition) {
      const speech = new SpeechRecognition()
      speech.lang = navigator.language || 'en-US'
      speech.continuous = true
      speech.interimResults = true
      let committed = ''
      speech.onresult = event => {
        let interim = ''
        for (let index = event.resultIndex; index < event.results.length; index += 1) {
          const result = event.results[index]
          if (result.isFinal) committed = joinText(committed, result[0].transcript.trim())
          else interim += result[0].transcript
        }
        onValueChange(joinText(baseText.current, joinText(committed, interim.trim())))
      }
      speech.onerror = event => { if (event.error !== 'aborted' && event.error !== 'no-speech') props.onError(t('Speech recognition failed: {error}. Switch engines in Settings → Voice.', { error: event.error })) }
      speech.onend = () => { setVoice('idle'); release() }
      recognition.current = speech
      speech.start()
      setVoice('listening')
      return
    }
    const type = pickRecordingType()
    try {
      const media = new MediaRecorder(stream, type ? { mimeType: type } : undefined)
      chunks.current = []
      media.ondataavailable = event => { if (event.data.size) chunks.current.push(event.data) }
      recorder.current = media
      media.start(250)
      setVoice('listening')
    } catch (cause) { release(); props.onError(t('Recording failed: {error}', { error: cause instanceof Error ? cause.message : String(cause) })) }
  }

  const togglePause = () => {
    const media = recorder.current
    if (!media) return
    if (media.state === 'recording') {
      media.pause()
      clockState.current.banked += performance.now() - clockState.current.started
      setElapsed(clockState.current.banked)
      setVoice('paused')
    } else if (media.state === 'paused') {
      media.resume()
      clockState.current.started = performance.now()
      setVoice('listening')
    }
  }

  /** Stops recording and transcribes; with `send`, submits the message as soon as the text is in. */
  const finishVoice = (send = false) => {
    if (recognition.current) {
      recognition.current.onend = () => { setVoice('idle'); release(); if (send && valueRef.current.trim()) props.onSubmit(valueRef.current) }
      recognition.current.stop()
      return
    }
    const media = recorder.current
    if (!media) return
    const token = ++run.current
    media.onstop = async () => {
      const blob = new Blob(chunks.current, { type: media.mimeType || 'audio/webm' })
      release()
      setVoice('transcribing')
      try {
        const text = await props.onTranscribe(blob)
        if (token !== run.current) return
        const next = joinText(valueRef.current, text)
        if (text) onValueChange(next)
        if (send && next.trim()) props.onSubmit(next)
      } catch (cause) { if (token === run.current) props.onError(cause instanceof Error ? cause.message : String(cause)) }
      finally { if (token === run.current) setVoice('idle') }
    }
    media.stop()
  }

  // Released while the microphone was still starting: stop as soon as recording begins.
  useEffect(() => { if (voice === 'listening' && pressed.current && !holding.current) { pressed.current = false; finishVoice(false) } })

  const pressMic = () => { if (!holdToRecord || voice !== 'idle') return; holding.current = true; pressed.current = true; void startVoice() }
  const releaseMic = () => {
    if (!holding.current) return
    holding.current = false
    if (voice === 'listening' || voice === 'paused') { pressed.current = false; finishVoice(false) }
  }

  const cancelVoice = () => {
    run.current += 1
    if (recognition.current) { recognition.current.onend = null; recognition.current.abort(); onValueChange(baseText.current) }
    if (recorder.current) { recorder.current.onstop = null; if (recorder.current.state !== 'inactive') recorder.current.stop() }
    release()
    setVoice('idle')
  }

  useEffect(() => () => { recognition.current?.abort(); if (recorder.current && recorder.current.state !== 'inactive') { recorder.current.onstop = null; recorder.current.stop() } }, [])

  const active = voice !== 'idle'
  const recording = voice === 'listening' || voice === 'paused'
  const transcribing = voice === 'transcribing'
  const [slashIndex, setSlashIndex] = useState(0)
  const [slashClosed, setSlashClosed] = useState('')
  const slashMenu = useRef<HTMLDivElement>(null)
  // Keep the highlighted command in view as the arrow keys move through a long list.
  useEffect(() => {
    const menu = slashMenu.current
    const active = menu?.querySelector<HTMLElement>('.slash-item.active')
    if (!menu || !active) return
    if (active.offsetTop < menu.scrollTop) menu.scrollTop = active.offsetTop - 4
    else if (active.offsetTop + active.offsetHeight > menu.scrollTop + menu.clientHeight) menu.scrollTop = active.offsetTop + active.offsetHeight - menu.clientHeight + 4
  }, [slashIndex])
  const slashQuery = /^\/([\w-]*)$/.exec(value)?.[1]
  const slashMatches = slashQuery === undefined || slashClosed === value ? [] : props.commands.filter(command => command.name.startsWith(slashQuery.toLowerCase()))
  const mentionQuery = /@([^\s@]*)$/.exec(value)?.[1]
  const mentionMatches = mentionQuery === undefined ? [] : (props.projectFiles ?? []).filter(path => path.toLowerCase().includes(mentionQuery.toLowerCase())).slice(0, 8)
  const resourceMatches = mentionQuery === undefined ? [] : (props.resources ?? []).filter(item => `${item.server} ${item.name}`.toLowerCase().includes(mentionQuery.toLowerCase())).slice(0, 6)
  const pickFile = (path: string) => { props.onAttachFile?.(path); onValueChange(value.replace(/@[^\s@]*$/, '')) }
  const pickResource = (key: string) => { props.onAttachResource?.(key); onValueChange(value.replace(/@[^\s@]*$/, '')) }
  const chooseCommand = (command: SlashCommand) => {
    if (command.template.includes('$ARGUMENTS')) { onValueChange(`/${command.name} `); setSlashClosed(`/${command.name} `); return }
    if (props.onCommand(command, '')) onValueChange('')
  }
  const submit = (text: string) => {
    const match = /^\/([\w-]+)(?:\s+([\s\S]*))?$/.exec(text.trim())
    const command = match && props.commands.find(item => item.name === match[1].toLowerCase())
    if (command && props.onCommand(command, (match[2] ?? '').trim())) { onValueChange(''); return }
    props.onSubmit(text)
  }

  const runsAsCommand = (text: string) => {
    const match = /^\/([\w-]+)(?:\s+([\s\S]*))?$/.exec(text.trim())
    return Boolean(match && props.commands.find(item => item.name === match[1].toLowerCase()))
  }
  const queue = (text: string) => {
    if (runsAsCommand(text)) { props.onError(t('Commands run right away and cannot be queued. Wait for the reply to finish, then run it.')); return }
    props.onQueue?.(text)
  }
  const fork = (text: string) => {
    if (runsAsCommand(text)) { props.onError(t('Commands cannot be sent in a forked session.')); return }
    props.onFork?.(text)
  }

  const micBusy = disabled || loading || !mic.supported || transcribing

  // The command and file menus open above the composer, or below it when the composer sits high (the
  // chat home screen), and are never taller than the room on that side, so nothing runs off the window.
  const composerRoot = useRef<HTMLDivElement>(null)
  const menuOpen = slashMatches.length > 0 || mentionMatches.length + resourceMatches.length > 0
  const [menuRoom, setMenuRoom] = useState<{ up: boolean; height: number }>({ up: true, height: 320 })
  useLayoutEffect(() => {
    if (!menuOpen || !composerRoot.current) return
    const rect = composerRoot.current.getBoundingClientRect()
    const ceiling = (document.querySelector('.titlebar')?.getBoundingClientRect().bottom ?? 0) + 8
    const above = rect.top - 8 - ceiling
    const below = window.innerHeight - rect.bottom - 16
    const up = above >= 200 || above >= below
    const height = Math.max(96, Math.min(320, up ? above : below))
    setMenuRoom(current => current.up === up && current.height === height ? current : { up, height })
  }, [menuOpen])
  const menuStyle = menuRoom.up ? { maxHeight: menuRoom.height } : { maxHeight: menuRoom.height, top: 'calc(100% + 8px)', bottom: 'auto' }

  return <div ref={composerRoot} className={cn('composer', active && 'voice-active', props.roomy && 'roomy')}>
    {mentionMatches.length + resourceMatches.length > 0 && <div className="slash-menu" style={menuStyle} role="listbox" aria-label={t('Project files and connector resources')}>
      {mentionMatches.map(path => <button key={path} type="button" className="slash-item" onMouseDown={event => { event.preventDefault(); pickFile(path) }}><span>{path}</span></button>)}
      {resourceMatches.map(item => <button key={item.key} type="button" className="slash-item" onMouseDown={event => { event.preventDefault(); pickResource(item.key) }}><span>{item.name}</span><small>{item.server}</small></button>)}
    </div>}
    {slashMatches.length > 0 && <div ref={slashMenu} className="slash-menu" style={menuStyle} role="listbox" aria-label={t('Commands')}>
      {slashMatches.map((command, index) => <button key={command.name} type="button" role="option" aria-selected={index === Math.min(slashIndex, slashMatches.length - 1)} className={cn('slash-item', index === Math.min(slashIndex, slashMatches.length - 1) && 'active')} onMouseEnter={() => setSlashIndex(index)} onMouseDown={event => { event.preventDefault(); chooseCommand(command) }}>
        <code>/{command.name}</code><span>{command.description}</span>{command.source !== 'built-in' && <small>{command.source}</small>}
      </button>)}
    </div>}
    <div className={cn('composer-beam', active && 'voice-active')}>
      <PromptInput
        inline
        minRows={props.roomy ? 3 : 1}
        value={value}
        onValueChange={onValueChange}
        onSubmit={prompt => submit(prompt)}
        loading={loading}
        onStop={props.onStop}
        onSteer={props.onSteer}
        onQueue={props.onQueue ? queue : undefined}
        onFork={props.onFork ? fork : undefined}
        header={props.attachments}
        sendWithoutText={props.sendWithoutText}
        disabled={disabled || transcribing}
        placeholder={voice === 'listening' ? t('Listening…') : voice === 'paused' ? t('Paused') : transcribing ? t('Transcribing…') : props.placeholder}
        aria-label={props.ariaLabel ?? t('Message Neru')}
        className="neru-prompt"
        maxRows={10}
        onKeyDown={event => {
          props.onKeyDown?.(event)
          if (event.defaultPrevented) return
          if (mentionMatches.length > 0 && event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); pickFile(mentionMatches[0]); return }
          if (resourceMatches.length > 0 && event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); pickResource(resourceMatches[0].key); return }
          if (slashMatches.length > 0) {
            if (event.key === 'ArrowDown' || event.key === 'ArrowUp') { event.preventDefault(); setSlashIndex(index => (index + (event.key === 'ArrowDown' ? 1 : -1) + slashMatches.length) % slashMatches.length); return }
            if ((event.key === 'Enter' && !event.shiftKey) || event.key === 'Tab') { event.preventDefault(); chooseCommand(slashMatches[Math.min(slashIndex, slashMatches.length - 1)]); return }
            if (event.key === 'Escape') { event.preventDefault(); setSlashClosed(value); return }
          }
          if (!active) return
          if (event.key === 'Escape') { event.preventDefault(); cancelVoice() }
          if (event.key === 'Enter' && !event.shiftKey && recording) { event.preventDefault(); finishVoice(true) }
        }}
        trailingAction={active ? <div className="voice-controls" role="group" aria-label={t('Dictation')}>
          {transcribing
            ? <span className="voice-status"><LoaderCircle size={14} className="animate-spin" />{t('Transcribing…')}</span>
            : <span className={cn('voice-status', voice === 'paused' && 'paused')}><SoundBars stream={mic.stream} paused={voice === 'paused'} />{clock(elapsed)}</span>}
          {recording && recorder.current && <button type="button" className="voice-button" onClick={togglePause} aria-label={voice === 'paused' ? t('Resume recording') : t('Pause recording')} title={voice === 'paused' ? t('Resume') : t('Pause')}>{voice === 'paused' ? <Play size={14} /> : <Pause size={14} />}</button>}
          <button type="button" className="voice-button" onClick={cancelVoice} aria-label={t('Discard recording')} title={t('Discard (Esc)')}><X size={15} /></button>
          {recording && <button type="button" className="voice-button confirm" onClick={() => finishVoice(false)} aria-label={t('Insert text')} title={t('Insert text to edit before sending')}><Check size={15} /></button>}
          {recording && <button type="button" className="voice-button send" onClick={() => finishVoice(true)} aria-label={t('Transcribe and send')} title={t('Transcribe and send (Enter)')}><ArrowUp size={16} /></button>}
        </div> : undefined}
      />
    </div>
    <div className="composer-toolbar">
      <div className="composer-tools">
        <PlusMenu items={props.attachItems} disabled={disabled || loading || active} />
        <div className="mic-group">
          <button type="button" className={cn('prompt-toggle', recording && 'on')} disabled={micBusy} aria-pressed={recording} aria-label={recording ? t('Stop dictation') : t('Dictate')}
            title={!mic.supported ? t('Microphone not available') : recording ? t('Stop and insert text') : holdToRecord ? t('Hold to dictate') : t('Dictate')}
            onPointerDown={event => { if (event.button === 0 && holdToRecord) { event.currentTarget.setPointerCapture(event.pointerId); pressMic() } }}
            onPointerUp={releaseMic} onPointerCancel={releaseMic}
            // Pointer clicks are handled by press and release in hold mode; Enter and Space still toggle.
            onClick={event => { if (holdToRecord && event.detail > 0) return; pressed.current = false; if (recording) finishVoice(false); else void startVoice() }}><Mic size={16} weight={recording ? 'fill' : 'regular'} /></button>
          <MicMenu device={micDevice} onDevice={chooseMic} hold={holdToRecord} onHold={chooseHold} disabled={!mic.supported || recording || transcribing} systemEngine={props.voiceEngine === 'system'} />
        </div>
        {props.showWeb !== false && <button type="button" className={cn('prompt-toggle', web && 'on')} aria-pressed={web} onClick={() => props.onWebChange(!web)} disabled={disabled} title={web ? t('Web search on: Neru can search and cite public pages') : t('Web search off: answers from the project only')} aria-label={t('Web search')}><Globe size={15} weight={web ? 'fill' : 'regular'} /></button>}
        {props.onResearchChange && <button type="button" className={cn('prompt-toggle', props.research && 'on')} aria-pressed={props.research} onClick={() => props.onResearchChange?.(!props.research)} disabled={disabled} title={props.research ? t('Research on: Neru plans, searches papers and writes a cited report') : t('Research: search papers and write a cited report')} aria-label={t('Research mode')}><GraduationCap size={15} weight={props.research ? 'fill' : 'regular'} /></button>}
        {props.showMode !== false && <Select value={mode} onValueChange={value => props.onModeChange(value as AgentMode)} disabled={disabled || loading} className="composer-mode">
          <SelectTrigger className={cn('h-8 w-auto rounded-lg border-0 bg-transparent px-2 py-0 text-xs hover:bg-muted focus-visible:ring-2', mode === 'bypass' && 'mode-danger')}>
            <span className="truncate text-muted-foreground" title={t(MODES.find(item => item.value === mode)?.hint ?? '')}>{t(MODES.find(item => item.value === mode)?.label ?? '')}</span>
          </SelectTrigger>
          <SelectContent className="w-72 shadow-none">
            {MODES.map(item => <SelectItem key={item.value} value={item.value} className="py-2"><span className="flex min-w-0 flex-col"><span className="text-sm text-foreground">{t(item.label)}</span><span className="text-xs text-muted-foreground">{t(item.hint)}</span></span></SelectItem>)}
          </SelectContent>
        </Select>}
      </div>
      <div className="composer-settings">
      {props.models.length > 0 && <ModelSelector models={props.models} value={props.model} onChange={props.onModelChange} providerId={props.providerId ?? ''}
        effort={props.effort} effortSupported={props.effortSupported} onEffortChange={props.onEffortChange} disabled={disabled || loading} />}
      {props.context && <ContextMeter usage={props.context} />}
      </div>
    </div>
    {props.modelNotice && <p className={cn('model-notice', props.modelNotice.tone)} role={props.modelNotice.tone === 'error' ? 'alert' : 'status'}>{props.modelNotice.tone === 'checking' && <LoaderCircle size={12} className="animate-spin" />}{props.modelNotice.text}</p>}
  </div>
}

export const attachIcons = { upload: <Upload size={15} />, files: <FolderSearch size={15} />, open: <FileCode2 size={15} />, changes: <GitCompareArrows size={15} /> }

export function FilePicker({ files, attached, onPick, onClose }: { files: string[]; attached: string[]; onPick: (path: string) => void; onClose: () => void }) {
  const [query, setQuery] = useState('')
  return <div className="file-picker">
    <div className="file-picker-head"><span>{t('Attach project file')}</span><button className="icon-button" aria-label={t('Close file picker')} onClick={onClose}><X size={14} /></button></div>
    <input autoFocus aria-label={t('Search project files')} placeholder={t('Search by path…')} value={query} onChange={event => setQuery(event.target.value)} onKeyDown={event => { if (event.key === 'Escape') onClose() }} />
    <div className="file-picker-results">{files.filter(path => path.toLowerCase().includes(query.toLowerCase()) && !attached.includes(path)).slice(0, 40).map(path => <button key={path} onClick={() => onPick(path)}><FileSearch size={13} /><span className="truncate">{path}</span></button>)}</div>
  </div>
}
