import { useEffect, useRef, useState, type ReactNode } from 'react'
import { ArrowUp, Check, FileCode2, FileSearch, FolderSearch, GitCompareArrows, Globe, LoaderCircle, Mic, Pause, Play, Plus, Search, Upload, X } from 'lucide-react'
import { Liquid } from 'liquid-gooey'
import { useReducedMotion } from 'motion/react'
import { useMicrophone, VoiceBeam } from 'voice-glow'
import { PromptInput, type PromptModel } from '@/components/agents/prompt-input'
import { Select, SelectContent, SelectItem, SelectTrigger } from '@/components/motion/select'
import { neruVoiceGlow } from '@/lib/voiceTheme'
import { cn } from '@/lib/utils'
import type { AgentMode, ContextUsage, Effort, SlashCommand } from '../../types'

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
  const show = () => { window.clearTimeout(timer.current); setMounted(true); requestAnimationFrame(() => requestAnimationFrame(() => setOpen(true))) }
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
      {mounted && items.map((item, index) => <Liquid.Item key={item.id} className="plus-item-slot" x={open ? 0 : collapsedX(item.label)} y={open ? -(BOX_CLEARANCE + ITEM_GAP * (items.length - index)) : origin} scale={open ? 1 : 0} transition={transition} delay={open ? (items.length - 1 - index) * 40 : index * 30}>
        <button type="button" role="menuitem" className="plus-item" tabIndex={open ? 0 : -1} aria-hidden={!open} disabled={item.disabled} title={item.hint} onClick={() => { hide(); item.onSelect() }}>{item.icon}<span>{item.label}</span></button>
      </Liquid.Item>)}
    </Liquid>
    </div>
    <button type="button" className="plus-trigger" aria-label="Add to message" aria-haspopup="menu" aria-expanded={open} disabled={disabled} onClick={() => open ? hide() : show()}><Plus size={17} /></button>
  </div>
}

export const MODES: { value: AgentMode; label: string; hint: string }[] = [
  { value: 'manual', label: 'Ask permissions', hint: 'Neru asks before every edit and command' },
  { value: 'accept_edits', label: 'Accept edits', hint: 'Edits apply right away; commands still ask' },
  { value: 'plan', label: 'Plan mode', hint: 'Read-only: explores and proposes a plan' },
  { value: 'auto', label: 'Auto', hint: 'Edits, build, test and lint run; other commands ask' },
  { value: 'bypass', label: 'Bypass permissions', hint: 'Everything runs without asking. Use with care' },
]

/** Fills a command template: `$ARGUMENTS` becomes what was typed after the name. */
export function expandCommand(command: SlashCommand, args: string) {
  return command.template.includes('$ARGUMENTS') ? command.template.replaceAll('$ARGUMENTS', args) : args ? `${command.template}\n\n${args}` : command.template
}

const EFFORTS: { value: Effort; label: string; hint: string }[] = [
  { value: 'auto', label: 'Auto', hint: 'The model decides how long to think' },
  { value: 'low', label: 'Low', hint: 'Fastest answers, least reasoning' },
  { value: 'medium', label: 'Medium', hint: 'Balanced reasoning' },
  { value: 'high', label: 'High', hint: 'Thinks longest; best for hard problems' },
]

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
  const label = `${usage.measured ? '' : 'About '}${compact(usage.used)} of ${compact(limit)} tokens used (${percent}%)`
  return <span className={cn('context-meter', tone)} tabIndex={0} role="button" aria-label={`Context: ${label}`}>
    <svg width="18" height="18" viewBox="0 0 18 18" aria-hidden><circle cx="9" cy="9" r={radius} className="track" /><circle cx="9" cy="9" r={radius} className="fill" strokeDasharray={`${Math.max(share * length, share > 0 ? 1.5 : 0)} ${length}`} transform="rotate(-90 9 9)" /></svg>
    <span className="context-panel" role="tooltip">
      <strong>{usage.model || 'Context'}</strong>
      <span className="context-row"><span>This conversation</span><b>{usage.measured ? '' : '~'}{count(usage.used)} tokens · {percent}%</b></span>
      <span className="context-row"><span>Context window</span><b>{count(usage.window)}{usage.windowReported ? '' : ' (estimate)'}</b></span>
      {capped && <span className="context-row"><span>Per-request limit</span><b>{count(limit)}</b></span>}
      {capped && <small>This key allows {compact(limit)} tokens a minute, so each request must fit in that. Neru trims and summarizes to stay under it.</small>}
      {quotas.length > 0 && <span className="context-quotas">{quotas.map(quota => {
        const left = quota.limit && quota.remaining !== null ? Math.max(0, Math.min(1, quota.remaining / quota.limit)) : null
        return <span key={quota.label} className="context-quota">
          <span className="context-row"><span>{quota.label}</span><b>{quota.remaining !== null ? count(quota.remaining) : '?'}{quota.limit ? ` of ${count(quota.limit)}` : ''} left</b></span>
          {left !== null && <i className={cn(left < 0.1 && 'low')} style={{ ['--left' as string]: `${left * 100}%` }} />}
          {quota.resetsIn && <small>Resets in {quota.resetsIn}</small>}
        </span>
      })}</span>}
      {quotas.length === 0 && <small>Rate limits appear here after the first reply, when the provider reports them.</small>}
      {share > 0.9 && <small>Nearly full. Start a new session, rewind, or run /compact.</small>}
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
  models: string[]
  onModelChange: (model: string) => void
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
  /** Taller prompt card used on the empty chat page. */
  roomy?: boolean
  /** Permission mode is a coding control. Chat hides it. */
  showMode?: boolean
}

export function Composer(props: ComposerProps) {
  const { value, onValueChange, loading, disabled, mode, web } = props
  const mic = useMicrophone()
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
    if (!stream) { props.onError(mic.error?.message || 'Microphone access was denied. Allow it in Windows privacy settings and try again.'); return }
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
      speech.onerror = event => { if (event.error !== 'aborted' && event.error !== 'no-speech') props.onError(`Speech recognition failed: ${event.error}. Switch engines in Settings → Voice.`) }
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
    } catch (cause) { release(); props.onError(`Recording failed: ${cause instanceof Error ? cause.message : String(cause)}`) }
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

  const cancelVoice = () => {
    run.current += 1
    if (recognition.current) { recognition.current.onend = null; recognition.current.abort(); onValueChange(baseText.current) }
    if (recorder.current) { recorder.current.onstop = null; if (recorder.current.state !== 'inactive') recorder.current.stop() }
    release()
    setVoice('idle')
  }

  useEffect(() => () => { recognition.current?.abort(); if (recorder.current && recorder.current.state !== 'inactive') { recorder.current.onstop = null; recorder.current.stop() } }, [])

  const shortName = (model: string) => model.split('/').pop() || model
  const shortCounts = new Map<string, number>()
  for (const model of props.models) shortCounts.set(shortName(model), (shortCounts.get(shortName(model)) ?? 0) + 1)
  const models: PromptModel[] = props.models.map(model => {
    const short = shortName(model)
    return { value: model, label: <span className="model-label" title={model}>{shortCounts.get(short) === 1 ? short : model}</span> }
  })
  // Providers like OpenRouter and NVIDIA list hundreds of models; search them instead of scrolling.
  const [modelOpen, setModelOpen] = useState(false)
  const [modelQuery, setModelQuery] = useState('')
  const modelSearch = useRef<HTMLInputElement>(null)
  const searchable = models.length > 10
  const modelTerms = modelQuery.toLowerCase().split(/\s+/).filter(Boolean)
  const shownModels = modelTerms.length ? models.filter(option => modelTerms.every(term => option.value.toLowerCase().includes(term))) : models
  const openModels = (open: boolean) => {
    setModelOpen(open)
    if (open && searchable) window.setTimeout(() => modelSearch.current?.focus({ preventScroll: true }), 60)
    if (!open) setModelQuery('')
  }
  const active = voice !== 'idle'
  const recording = voice === 'listening' || voice === 'paused'
  const transcribing = voice === 'transcribing'
  const glow = neruVoiceGlow(props.light)
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
  const pickFile = (path: string) => { props.onAttachFile?.(path); onValueChange(value.replace(/@[^\s@]*$/, '')) }
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

  const currentModel = models.find(option => option.value === props.model)
  const micBusy = disabled || loading || !mic.supported || transcribing

  return <div className={cn('composer', active && 'voice-active', props.roomy && 'roomy')}>
    {mentionMatches.length > 0 && <div className="slash-menu" role="listbox" aria-label="Project files">
      {mentionMatches.map(path => <button key={path} type="button" className="slash-item" onMouseDown={event => { event.preventDefault(); pickFile(path) }}><span>{path}</span></button>)}
    </div>}
    {slashMatches.length > 0 && <div ref={slashMenu} className="slash-menu" role="listbox" aria-label="Commands">
      {slashMatches.map((command, index) => <button key={command.name} type="button" role="option" aria-selected={index === Math.min(slashIndex, slashMatches.length - 1)} className={cn('slash-item', index === Math.min(slashIndex, slashMatches.length - 1) && 'active')} onMouseEnter={() => setSlashIndex(index)} onMouseDown={event => { event.preventDefault(); chooseCommand(command) }}>
        <code>/{command.name}</code><span>{command.description}</span>{command.source !== 'built-in' && <small>{command.source}</small>}
      </button>)}
    </div>}
    <VoiceBeam stream={voice === 'listening' ? mic.stream : null} processing={transcribing} active={active} paused={voice === 'paused'} theme={props.light ? 'light' : 'dark'} {...glow} className={cn('composer-beam', !active && 'is-idle', active && 'voice-active')}>
      <PromptInput
        inline
        minRows={props.roomy ? 3 : 1}
        value={value}
        onValueChange={onValueChange}
        onSubmit={prompt => submit(prompt)}
        loading={loading}
        onStop={props.onStop}
        onSteer={props.onSteer}
        disabled={disabled || transcribing}
        placeholder={voice === 'listening' ? 'Listening…' : voice === 'paused' ? 'Paused' : transcribing ? 'Transcribing…' : props.placeholder}
        aria-label="Message Neru"
        className="neru-prompt"
        maxRows={10}
        onKeyDown={event => {
          if (mentionMatches.length > 0 && event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); pickFile(mentionMatches[0]); return }
          if (slashMatches.length > 0) {
            if (event.key === 'ArrowDown' || event.key === 'ArrowUp') { event.preventDefault(); setSlashIndex(index => (index + (event.key === 'ArrowDown' ? 1 : -1) + slashMatches.length) % slashMatches.length); return }
            if ((event.key === 'Enter' && !event.shiftKey) || event.key === 'Tab') { event.preventDefault(); chooseCommand(slashMatches[Math.min(slashIndex, slashMatches.length - 1)]); return }
            if (event.key === 'Escape') { event.preventDefault(); setSlashClosed(value); return }
          }
          if (!active) return
          if (event.key === 'Escape') { event.preventDefault(); cancelVoice() }
          if (event.key === 'Enter' && !event.shiftKey && recording) { event.preventDefault(); finishVoice(true) }
        }}
        trailingAction={active ? <div className="voice-controls" role="group" aria-label="Dictation">
          {transcribing
            ? <span className="voice-status"><LoaderCircle size={14} className="animate-spin" />Transcribing…</span>
            : <span className={cn('voice-status', voice === 'paused' && 'paused')}><i aria-hidden />{clock(elapsed)}</span>}
          {recording && recorder.current && <button type="button" className="voice-button" onClick={togglePause} aria-label={voice === 'paused' ? 'Resume recording' : 'Pause recording'} title={voice === 'paused' ? 'Resume' : 'Pause'}>{voice === 'paused' ? <Play size={14} /> : <Pause size={14} />}</button>}
          <button type="button" className="voice-button" onClick={cancelVoice} aria-label="Discard recording" title="Discard (Esc)"><X size={15} /></button>
          {recording && <button type="button" className="voice-button confirm" onClick={() => finishVoice(false)} aria-label="Insert text" title="Insert text to edit before sending"><Check size={15} /></button>}
          {recording && <button type="button" className="voice-button send" onClick={() => finishVoice(true)} aria-label="Transcribe and send" title="Transcribe and send (Enter)"><ArrowUp size={16} /></button>}
        </div> : undefined}
      />
    </VoiceBeam>
    <div className="composer-toolbar">
      <div className="composer-tools">
        <PlusMenu items={props.attachItems} disabled={disabled || loading || active} />
        <button type="button" className={cn('prompt-toggle', recording && 'on')} onClick={() => recording ? finishVoice(false) : void startVoice()} disabled={micBusy} aria-pressed={recording} aria-label={recording ? 'Stop dictation' : 'Dictate'} title={!mic.supported ? 'Microphone not available' : recording ? 'Stop and insert text' : 'Dictate'}><Mic size={16} /></button>
        <button type="button" className={cn('prompt-toggle', web && 'on')} aria-pressed={web} onClick={() => props.onWebChange(!web)} disabled={disabled} title={web ? 'Web search on: Neru can search and cite public pages' : 'Web search off: answers from the project only'} aria-label="Web search"><Globe size={15} /></button>
        {props.showMode !== false && <Select value={mode} onValueChange={value => props.onModeChange(value as AgentMode)} disabled={disabled || loading} className="composer-mode">
          <SelectTrigger className={cn('h-8 w-auto rounded-lg border-0 bg-transparent px-2 py-0 text-xs hover:bg-muted focus-visible:ring-2', mode === 'bypass' && 'mode-danger')}>
            <span className="truncate text-muted-foreground" title={MODES.find(item => item.value === mode)?.hint}>{MODES.find(item => item.value === mode)?.label}</span>
          </SelectTrigger>
          <SelectContent className="w-72 shadow-none">
            {MODES.map(item => <SelectItem key={item.value} value={item.value} className="py-2"><span className="flex min-w-0 flex-col"><span className="text-sm text-foreground">{item.label}</span><span className="text-xs text-muted-foreground">{item.hint}</span></span></SelectItem>)}
          </SelectContent>
        </Select>}
      </div>
      <div className="composer-settings">
      {models.length > 0 && <Select value={props.model} onValueChange={props.onModelChange} open={modelOpen} onOpenChange={openModels} disabled={disabled || loading} className="composer-model">
        <SelectTrigger className="h-8 w-auto max-w-56 rounded-lg border-0 bg-transparent px-2 py-0 text-xs hover:bg-muted focus-visible:ring-2">
          <span className="truncate text-muted-foreground">{currentModel?.label ?? 'Choose model'}</span>
        </SelectTrigger>
        <SelectContent className="left-auto right-0 w-72 shadow-none" maxHeight={340} header={searchable && <label className="model-search">
          <Search size={13} aria-hidden />
          <input ref={modelSearch} value={modelQuery} onChange={event => setModelQuery(event.target.value)} placeholder={`Search ${models.length} models`} aria-label="Search models" spellCheck={false}
            onKeyDown={event => { if (event.key === 'Enter' && shownModels[0]) { event.preventDefault(); props.onModelChange(shownModels[0].value); openModels(false) } }} />
        </label>}>
          {shownModels.map(option => <SelectItem key={option.value} value={option.value} className="py-2"><span className="min-w-0 truncate text-sm text-foreground">{option.label}</span></SelectItem>)}
          {shownModels.length === 0 && <p className="model-search-empty">No models match “{modelQuery}”.</p>}
        </SelectContent>
      </Select>}
      {props.effortSupported && <Select value={props.effort} onValueChange={value => props.onEffortChange(value as Effort)} disabled={disabled || loading} className="composer-effort">
        <SelectTrigger className="h-8 w-auto rounded-lg border-0 bg-transparent px-2 py-0 text-xs hover:bg-muted focus-visible:ring-2">
          <span className="truncate text-muted-foreground" title="Reasoning effort">{EFFORTS.find(item => item.value === props.effort)?.label ?? 'Auto'}</span>
        </SelectTrigger>
        <SelectContent className="left-auto right-0 w-60 shadow-none">
          {EFFORTS.map(item => <SelectItem key={item.value} value={item.value} className="py-2"><span className="flex min-w-0 flex-col"><span className="text-sm text-foreground">{item.label}</span><span className="text-xs text-muted-foreground">{item.hint}</span></span></SelectItem>)}
        </SelectContent>
      </Select>}
      {props.context && <ContextMeter usage={props.context} />}
      </div>
    </div>
  </div>
}

export const attachIcons = { upload: <Upload size={15} />, files: <FolderSearch size={15} />, open: <FileCode2 size={15} />, changes: <GitCompareArrows size={15} /> }

export function FilePicker({ files, attached, onPick, onClose }: { files: string[]; attached: string[]; onPick: (path: string) => void; onClose: () => void }) {
  const [query, setQuery] = useState('')
  return <div className="file-picker">
    <div className="file-picker-head"><span>Attach project file</span><button className="icon-button" aria-label="Close file picker" onClick={onClose}><X size={14} /></button></div>
    <input autoFocus aria-label="Search project files" placeholder="Search by path…" value={query} onChange={event => setQuery(event.target.value)} onKeyDown={event => { if (event.key === 'Escape') onClose() }} />
    <div className="file-picker-results">{files.filter(path => path.toLowerCase().includes(query.toLowerCase()) && !attached.includes(path)).slice(0, 40).map(path => <button key={path} onClick={() => onPick(path)}><FileSearch size={13} /><span className="truncate">{path}</span></button>)}</div>
  </div>
}
