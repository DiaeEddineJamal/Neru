import { useEffect, useRef, useState } from 'react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { Apple, ArrowLeft, ArrowRight, ArrowUp, AudioLines, Bell, Check, ChevronDown, Cpu, FileText, Folder, FolderOpen, GitPullRequest, Globe, History, KeyRound, Layers, LoaderCircle, Monitor, Moon, Pause, Plug, ShieldCheck, Slash, Sparkles, Sun, Terminal, X, Zap } from 'lucide-react'
import { VoiceBeam } from 'voice-glow'
import { ToolApproval, ToolApprovalCode } from '@/components/agents/tool-approval'
import { dictationLanguages } from '@/lib/speech/catalog'
import { EASE_OUT } from '@/lib/ease'
import { neruVoiceGlow } from '@/lib/voiceTheme'
import { cn } from '@/lib/utils'
import { api } from '../../api'
import { author } from '../../credits'
import { formatForModel, providerPresets, type ApiFormat } from '../../providerCatalog'
import type { ProjectInfo, ProviderView } from '../../types'
import { Mascot } from './Mascot'
import { SpeechModelCatalog, useSpeechModels } from './SpeechModels'
import './Onboarding.css'

type OnboardingProps = {
  project: ProjectInfo | null
  provider: ProviderView
  isDesktop: boolean
  light: boolean
  language: string
  onLanguageChange: (language: string) => void
  onOpenProject: () => Promise<void>
  onProviderSaved: (value: ProviderView) => void
  onToggleTheme: () => void
  onFinish: () => void
}

const steps = [
  { id: 'welcome', label: 'Welcome', hint: 'What Neru is for', statement: 'Think.' },
  { id: 'project', label: 'Project', hint: 'Choose a folder', statement: 'Understand.' },
  { id: 'model', label: 'Model', hint: 'Connect a provider', statement: 'Build.' },
  { id: 'voice', label: 'Voice', hint: 'Dictate on-device', statement: 'Speak.' },
  { id: 'features', label: 'Toolkit', hint: 'What Neru can do', statement: 'Work.' },
  { id: 'engine', label: 'Engine', hint: 'Built on Rust', statement: 'Fast.' },
  { id: 'review', label: 'You decide', hint: 'Approve every change', statement: 'Refine.' },
] as const

const highlights = [
  { icon: FolderOpen, title: 'Reads your codebase', text: 'Explores files, Git history, and the web, citing its sources.' },
  { icon: ShieldCheck, title: 'Asks before acting', text: 'Every edit and command waits for your approval, with a checkpoint to undo.' },
  { icon: AudioLines, title: 'Listens privately', text: 'Dictate with a speech model that runs entirely on this machine.' },
]

const features = [
  { icon: Layers, title: 'Parallel sessions', text: 'Run several sessions at once; give each its own Git worktree so their changes never collide.' },
  { icon: History, title: 'Rewind', text: 'Step back to before any message, with or without undoing the file changes made since.' },
  { icon: FileText, title: 'Review every change', text: 'One pane shows everything a session changed. Leave line comments and send them back.' },
  { icon: ShieldCheck, title: 'Permission modes', text: 'Ask every time, accept edits, plan read-only, auto for routine work, or bypass when you trust it.' },
  { icon: Plug, title: 'Connectors', text: 'Linear, Notion, Figma, GitHub, Sentry and more over MCP, with browser sign-in for hosted ones.' },
  { icon: Slash, title: 'Slash commands', text: 'Type / for /compact, /review, /plan, /init, or your own templates in .neru/commands.' },
  { icon: Sparkles, title: 'Long sessions', text: 'A context meter, effort control, and automatic compaction when a conversation grows long.' },
  { icon: GitPullRequest, title: 'Ship it', text: 'Push, open pull requests, follow CI checks, and ask Neru to fix what fails.' },
  { icon: FileText, title: 'Attach anything', text: 'PDFs, Word documents, images and screenshots, from anywhere on your computer.' },
  { icon: Bell, title: 'Notifications', text: 'A system notification when a background session finishes or needs your approval.' },
]

const engine = [
  { icon: Cpu, title: 'A Rust core', text: 'Sessions, tools, streaming, Git and connectors run in compiled Rust on an async runtime, so parallel work stays responsive without a JavaScript server in the background.' },
  { icon: Zap, title: 'Light on memory and disk', text: 'Neru uses the web engine your system already has instead of shipping its own browser, so the app stays small and starts quickly.' },
  { icon: Monitor, title: 'At home on each platform', text: 'Windows: WebView2, Windows-style caption buttons, PowerShell, keys sealed with DPAPI. macOS: WebKit, native traffic lights, Keychain. Linux: WebKitGTK and the Secret Service.' },
  { icon: Terminal, title: 'Local by design', text: 'Speech runs on-device with multithreaded WebAssembly, or WebGPU where available. Your files and history stay on your machine.' },
]

/** A gentle synthetic voice level so the glow preview breathes without a microphone. */
function useDemoLevel(active: boolean) {
  const start = useRef(performance.now())
  return () => {
    if (!active) return 0
    const t = (performance.now() - start.current) / 1000
    return Math.max(0, 0.28 + 0.22 * Math.sin(t * 2.3) + 0.12 * Math.sin(t * 5.1 + 1.2) + 0.08 * Math.sin(t * 9.7))
  }
}

export function Onboarding({ project, provider, isDesktop, light, language, onLanguageChange, onOpenProject, onProviderSaved, onToggleTheme, onFinish }: OnboardingProps) {
  const reduce = useReducedMotion() ?? false
  const [step, setStep] = useState(0)
  const [direction, setDirection] = useState(1)
  const [providerId, setProviderId] = useState(provider.providerId)
  const [apiFormat, setApiFormat] = useState<ApiFormat>(provider.apiFormat)
  const [baseUrl, setBaseUrl] = useState(provider.baseUrl)
  const [model, setModel] = useState(provider.model)
  const [apiKey, setApiKey] = useState('')
  const [detailsOpen, setDetailsOpen] = useState(false)
  const [saving, setSaving] = useState(false)
  const [saved, setSaved] = useState(provider.configured)
  const [error, setError] = useState('')
  const [demo, setDemo] = useState<'listening' | 'processing'>('listening')
  const speech = useSpeechModels()
  const level = useDemoLevel(step === 3 && demo === 'listening')
  const preset = providerPresets.find(item => item.id === providerId)
  const [allProviders, setAllProviders] = useState(false)
  const freePresets = providerPresets.filter(item => item.freeLimit)
  const shownPresets = allProviders ? providerPresets : [...freePresets, ...(preset && !preset.freeLimit ? [preset] : [])]
  const local = /localhost|127\.0\.0\.1/.test(baseUrl)

  useEffect(() => {
    if (step !== 3 || reduce) return
    const timer = window.setInterval(() => setDemo(current => current === 'listening' ? 'processing' : 'listening'), 3600)
    return () => window.clearInterval(timer)
  }, [step, reduce])

  // Left and right arrow keys move between steps, except while typing in a field.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null
      if (target?.closest('input, textarea, select, [contenteditable="true"]') || event.altKey || event.ctrlKey || event.metaKey) return
      if (event.key === 'ArrowRight') moveTo(step + 1)
      if (event.key === 'ArrowLeft') moveTo(step - 1)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })

  const moveTo = (next: number) => {
    if (next < 0 || next >= steps.length) return
    setDirection(next > step ? 1 : -1)
    setError('')
    setStep(next)
  }

  const choosePreset = (id: string) => {
    const next = providerPresets.find(item => item.id === id)
    if (!next) return
    setProviderId(next.id); setApiFormat(formatForModel(next.id, next.model, next.format, next.baseUrl)); setBaseUrl(next.baseUrl); setModel(next.model); setApiKey(''); setSaved(false); setError('')
    if (!next.model) setDetailsOpen(true)
  }

  const saveProvider = async () => {
    if (!isDesktop) { setError('Open the Neru desktop app to connect a provider.'); return }
    setSaving(true); setError('')
    try {
      onProviderSaved(await api.configureProvider(providerId, apiFormat, baseUrl, apiKey, model))
      setApiKey(''); setSaved(true)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause)); setDetailsOpen(true)
    } finally { setSaving(false) }
  }

  const last = step === steps.length - 1
  const slide = reduce ? 0 : direction * 24

  return <main className="onboarding" aria-label="Getting started with Neru">
    <aside className="onboarding-aside">
      <div className="onboarding-brand-row">
        <div className="onboarding-brand"><Mascot size={30} /><span>Neru <span lang="ja">練る</span></span></div>
        <button type="button" className="onboarding-ghost onboarding-theme" onClick={onToggleTheme} aria-label={light ? 'Use dark appearance' : 'Use light appearance'}>{light ? <Moon size={15} /> : <Sun size={15} />}{light ? 'Dark' : 'Light'}</button>
      </div>

      <div className="onboarding-statement" aria-hidden>
        <AnimatePresence mode="wait" initial={false}>
          <motion.span key={steps[step].statement} initial={{ opacity: 0, y: reduce ? 0 : 14, filter: reduce ? 'none' : 'blur(6px)' }} animate={{ opacity: 1, y: 0, filter: 'blur(0px)' }} exit={{ opacity: 0, y: reduce ? 0 : -10, filter: reduce ? 'none' : 'blur(4px)' }} transition={{ duration: reduce ? 0 : 0.45, ease: EASE_OUT }}>{steps[step].statement}</motion.span>
        </AnimatePresence>
        <p>練る · to knead, to refine through repeated work.</p>
      </div>

      <nav className="onboarding-steps" aria-label="Setup steps">
        {steps.map((item, index) => {
          const state = index < step ? 'done' : index === step ? 'current' : 'upcoming'
          return <button key={item.id} type="button" className={cn('onboarding-step-link', state)} aria-current={state === 'current' ? 'step' : undefined} onClick={() => moveTo(index)}>
            <span className="onboarding-step-dot">{state === 'done' ? <Check size={12} strokeWidth={2.5} /> : index + 1}</span>
            <span><strong>{item.label}</strong><small>{item.hint}</small></span>
            {state === 'current' && <motion.i layoutId="onboarding-step-marker" className="onboarding-step-marker" transition={{ duration: reduce ? 0 : 0.35, ease: EASE_OUT }} />}
          </button>
        })}
      </nav>

      <div className="onboarding-aside-footer">
        <a className="onboarding-credit" href={author.url} target="_blank" rel="noreferrer">Kneaded by <strong>{author.handle}</strong></a>
        <button type="button" className="onboarding-ghost" onClick={onFinish}>Skip setup</button>
      </div>
    </aside>

    <section className="onboarding-content">
      <div className="onboarding-scroll">
        <AnimatePresence mode="wait" initial={false} custom={direction}>
          <motion.div key={step} className="onboarding-panel" initial={{ opacity: 0, x: slide }} animate={{ opacity: 1, x: 0 }} exit={{ opacity: 0, x: -slide }} transition={{ duration: reduce ? 0 : 0.32, ease: EASE_OUT }}>
            <span className="onboarding-eyebrow">Step {step + 1} of {steps.length}</span>

            {step === 0 && <>
              <div className="onboarding-hero"><Mascot size={112} interactive /></div>
              <h1>A quiet place to think <em>with</em> your code.</h1>
              <p className="onboarding-lede">Neru is a local-first coding agent. It explores your project, proposes changes you can read line by line, and never acts without asking.</p>
              <ul className="onboarding-highlights">{highlights.map(item => <li key={item.title}><span className="onboarding-highlight-icon"><item.icon size={17} /></span><div><strong>{item.title}</strong><small>{item.text}</small></div></li>)}</ul>
              <p className="onboarding-footnote">Kneaded into shape by <a href={author.url}>{author.handle}</a>.</p>
            </>}

            {step === 1 && <>
              <h1>Open a project.</h1>
              <p className="onboarding-lede">Pick a local folder. Your files stay where they are; Neru keeps its own data on D:.</p>
              <button type="button" className={cn('onboarding-folder', project && 'chosen')} onClick={() => void onOpenProject()} disabled={!isDesktop}>
                <span className="onboarding-folder-icon">{project ? <FolderOpen size={22} /> : <Folder size={22} />}</span>
                <span className="onboarding-folder-text"><strong>{project ? project.name : 'Choose a folder'}</strong><small>{project ? project.path : isDesktop ? 'Browse to a repository or any code folder' : 'Available in the desktop app'}</small></span>
                {project ? <span className="onboarding-pill"><Check size={13} /> Ready</span> : <ArrowRight size={18} />}
              </button>
              {project && <button type="button" className="onboarding-link" onClick={() => void onOpenProject()}>Choose a different folder</button>}
              <p className="onboarding-footnote">You can open more projects later from the sidebar. Each keeps its own sessions.</p>
            </>}

            {step === 2 && <>
              <h1>Connect a model.</h1>
              <p className="onboarding-lede">NVIDIA, ModelScope, Gemini, Cerebras, Mistral, and OpenRouter have free keys with daily allowances for coding. You can switch models any time from the message box.</p>
              <div className="onboarding-providers" role="radiogroup" aria-label="Model provider">
                {shownPresets.map(item => <button key={item.id} type="button" role="radio" aria-checked={providerId === item.id} className={cn('onboarding-provider', providerId === item.id && 'selected')} onClick={() => choosePreset(item.id)}>
                  <strong>{item.name}</strong><small>{item.description}</small>
                  {item.freeLimit && <span className="onboarding-provider-free">Free · {item.freeLimit}</span>}
                  {providerId === item.id && <motion.span layoutId="provider-check" className="onboarding-provider-check" transition={{ duration: reduce ? 0 : 0.25, ease: EASE_OUT }}><Check size={12} strokeWidth={2.5} /></motion.span>}
                </button>)}
              </div>
              <button type="button" className="onboarding-link" onClick={() => setAllProviders(value => !value)}>{allProviders ? 'Show only free coding providers' : `More providers (${providerPresets.length - freePresets.length}): local models, OpenAI, Anthropic, DeepSeek…`}</button>
              <form className="onboarding-connection" onSubmit={event => { event.preventDefault(); void saveProvider() }}>
                {!local && <label className="onboarding-field"><span><KeyRound size={13} /> API key <small>encrypted on this PC</small></span><input type="password" value={apiKey} onChange={event => { setApiKey(event.target.value); setSaved(false) }} placeholder={saved && provider.hasKey ? 'Key connected' : `Paste your ${preset?.name ?? ''} key`} autoComplete="off" /></label>}
                <button type="button" className="onboarding-disclosure" aria-expanded={detailsOpen} onClick={() => setDetailsOpen(value => !value)}>Connection details <motion.span animate={{ rotate: detailsOpen ? 180 : 0 }} transition={{ duration: reduce ? 0 : 0.2 }}><ChevronDown size={14} /></motion.span></button>
                <AnimatePresence initial={false}>{detailsOpen && <motion.div className="onboarding-details" initial={{ opacity: 0, height: 0 }} animate={{ opacity: 1, height: 'auto' }} exit={{ opacity: 0, height: 0 }} transition={{ duration: reduce ? 0 : 0.25, ease: EASE_OUT }}>
                  <div className="onboarding-field-row">
                    <label className="onboarding-field"><span>Model</span><input value={model} onChange={event => { const next = event.target.value; setModel(next); setApiFormat(formatForModel(providerId, next, apiFormat, baseUrl)); setSaved(false) }} placeholder="Model ID, e.g. auto" /></label>
                    <label className="onboarding-field"><span>API format</span><select value={apiFormat} onChange={event => { setApiFormat(event.target.value as ApiFormat); setSaved(false) }}><option value="openai-chat">Chat Completions</option><option value="openai-responses">Responses</option><option value="anthropic">Anthropic Messages</option></select></label>
                  </div>
                  <label className="onboarding-field"><span>Base URL</span><input value={baseUrl} onChange={event => { setBaseUrl(event.target.value); setSaved(false) }} autoComplete="url" /></label>
                </motion.div>}</AnimatePresence>
                {preset?.keyUrl && !local && <p className="onboarding-key-link">{preset.freeLimit && <span>Free: {preset.freeLimit}. </span>}<a href={preset.keyUrl} target="_blank" rel="noreferrer">Get a {preset.name} API key ↗</a></p>}
                <div className="onboarding-connection-actions">
                  <span>{saved ? <><Check size={14} /> Connected to {preset?.name ?? 'provider'}</> : local ? 'No key needed for local models.' : 'Keys are sent only to this provider.'}</span>
                  <button type="submit" className="onboarding-secondary" disabled={saving}>{saving ? 'Connecting…' : saved ? 'Reconnect' : 'Connect'}</button>
                </div>
                {error && <p className="onboarding-error" role="alert">{error}</p>}
              </form>
            </>}

            {step === 3 && <>
              <h1>Talk to Neru.</h1>
              <p className="onboarding-lede">Press the microphone in the message box and speak. A small model on this machine turns it into text, so your voice never leaves the computer.</p>
              <div className="onboarding-voice-demo">
                <VoiceBeam level={level} processing={demo === 'processing'} theme={light ? 'light' : 'dark'} {...neruVoiceGlow(light)} className="onboarding-beam">
                  <div className="onboarding-demo-input">
                    <span className="onboarding-demo-text">{demo === 'listening' ? 'Refactor the sidebar so it remembers its width…' : 'Transcribing…'}</span>
                    <span className="onboarding-demo-controls"><span className="onboarding-demo-pill">Review</span>
                      {demo === 'listening'
                        ? <span className="onboarding-demo-voice"><span className="voice-status"><i />0:04</span><span className="voice-button"><Pause size={14} /></span><span className="voice-button"><X size={15} /></span><span className="voice-button confirm"><Check size={15} /></span><span className="voice-button send"><ArrowUp size={16} /></span></span>
                        : <span className="onboarding-demo-voice"><span className="voice-status"><LoaderCircle size={14} className="animate-spin" />Transcribing…</span><span className="voice-button"><X size={15} /></span></span>}
                    </span>
                  </div>
                </VoiceBeam>
                <span className="onboarding-demo-caption">{demo === 'listening' ? 'Pause, discard, insert the text to edit, or send it straight away.' : 'A beam sweeps along the edge while the model transcribes.'}</span>
              </div>
              <label className="onboarding-field onboarding-language"><span><Globe size={13} /> Dictation language</span><select value={language} onChange={event => onLanguageChange(event.target.value)}>{dictationLanguages.map(item => <option key={item.value} value={item.value}>{item.label}</option>)}</select></label>
              <SpeechModelCatalog compact />
              <p className="onboarding-footnote">{speech.active ? 'Your model is ready. Change it any time in Settings → Voice.' : 'Optional. Whisper Base is a good start for English, French, and Spanish; Small or Turbo handle Arabic best.'}</p>
            </>}

            {step === 4 && <>
              <h1>Everything you need to <em>ship</em>.</h1>
              <p className="onboarding-lede">Neru grows with the task, from a quick question to a pull request with passing checks.</p>
              <ul className="onboarding-features">{features.map(item => <li key={item.title}><span className="onboarding-highlight-icon"><item.icon size={16} /></span><div><strong>{item.title}</strong><small>{item.text}</small></div></li>)}</ul>
              <p className="onboarding-footnote">Tip: type <kbd>/</kbd> in the message box to see every command.</p>
            </>}

            {step === 5 && <>
              <h1>Built on <em>Rust</em>.</h1>
              <p className="onboarding-lede">Neru is a Tauri app: a native Rust core with a thin interface on top. It stays quick while several sessions work at once, and leaves your machine’s memory for your code.</p>
              <ul className="onboarding-engine">{engine.map(item => <li key={item.title}><span className="onboarding-highlight-icon"><item.icon size={17} /></span><div><strong>{item.title}</strong><small>{item.text}</small></div></li>)}</ul>
              <div className="onboarding-platforms" aria-label="Supported platforms"><span><Monitor size={14} /> Windows</span><span><Apple size={14} /> macOS</span><span><Terminal size={14} /> Linux</span></div>
            </>}

            {step === 6 && <>
              <h1>Nothing changes without you.</h1>
              <p className="onboarding-lede">When Neru wants to edit a file or run a command, it stops and shows you exactly what will happen.</p>
              <div className="onboarding-approval-preview" aria-hidden>
                <ToolApproval tool="terminal.run" title="Allow this command to run?" description="Neru wants to run the test suite in your project." defaultOpen parameters={[{ id: 'cmd', label: 'Command', value: <ToolApprovalCode code="npm run test" /> }]} onApprove={() => undefined} onAlwaysAllow={() => undefined} onDeny={() => undefined} />
              </div>
              <ul className="onboarding-rules">
                <li><Sparkles size={15} /> Allow once, always allow a trusted command, or deny.</li>
                <li><ShieldCheck size={15} /> File edits show a full diff and save a checkpoint first, so you can rewind.</li>
                <li><KeyRound size={15} /> Choose how much Neru may do on its own with the permission mode under the message box.</li>
              </ul>
            </>}
          </motion.div>
        </AnimatePresence>
      </div>

      <footer className="onboarding-controls">
        <button className="onboarding-back" type="button" onClick={() => moveTo(step - 1)} disabled={step === 0}><ArrowLeft size={16} /> Back</button>
        <div className="onboarding-progress" aria-hidden>{steps.map((item, index) => <i key={item.id} className={index <= step ? 'on' : ''} />)}</div>
        <button className="onboarding-next" type="button" onClick={() => last ? onFinish() : moveTo(step + 1)}>{last ? 'Open workspace' : step === 3 && !speech.active ? 'Continue without voice' : 'Continue'} <ArrowRight size={16} /></button>
      </footer>
    </section>
  </main>
}
