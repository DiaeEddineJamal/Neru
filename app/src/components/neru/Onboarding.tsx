import { useEffect, useRef, useState, type ReactNode } from 'react'
import { LISTING_IS_AUTHORITATIVE } from './ModelPicker'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { ArrowLeft, ArrowRight, ArrowUp, AudioLines, Bell, Brain, Check, ChevronDown, CircleAlert, FileText, Folder, FolderOpen, GitFork, GitPullRequest, Globe, History, KeyRound, Layers, ListChecks, LoaderCircle, Moon, MonitorPlay, Network, Paperclip, Pause, PenLine, Plug, Rocket, Search, Shuffle, ShieldCheck, Slash, Sparkles, SquareTerminal, Sun, Users, Webhook, ArrowRightLeft, Download, Undo2, X, CornerDownRight, type LucideIcon } from 'lucide-react'
import { VoiceBeam } from 'voice-glow'
import { ToolApproval, ToolApprovalCode } from '@/components/agents/tool-approval'
import { dictationLanguages } from '@/lib/speech/catalog'
import { EASE_OUT } from '@/lib/ease'
import { neruVoiceGlow } from '@/lib/voiceTheme'
import { cn } from '@/lib/utils'
import { api } from '../../api'
import { author } from '../../credits'
import { formatForModel, providerPresets, type ApiFormat } from '../../providerCatalog'
import type { ProjectInfo, ProviderView, TeamAgent } from '../../types'
import { Mascot } from './Mascot'
import { WindowControls } from './TitleBar'
import { SpeechModelCatalog, useSpeechModels } from './SpeechModels'
import { AgentMark } from './team/TeamView'
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
  { id: 'welcome', label: 'Welcome' },
  { id: 'you', label: 'You' },
  { id: 'project', label: 'Project' },
  { id: 'model', label: 'Model' },
  { id: 'voice', label: 'Voice' },
  { id: 'team', label: 'Team' },
  { id: 'toolkit', label: 'Toolkit' },
  { id: 'decide', label: 'You decide' },
] as const

const STEP = { welcome: 0, you: 1, project: 2, model: 3, voice: 4, team: 5, toolkit: 6, decide: 7 } as const

const genders = [{ value: 'male', label: 'Male' }, { value: 'female', label: 'Female' }] as const

const highlights = [
  { icon: FolderOpen, title: 'Reads your codebase', text: 'Explores files, Git history, and the web, and cites its sources.' },
  { icon: ShieldCheck, title: 'Asks before acting', text: 'Every edit and command waits for your approval, with a checkpoint to undo.' },
  { icon: AudioLines, title: 'Private by design', text: 'Your files and history stay here. Dictation runs on this machine.' },
]

type Feature = { icon: LucideIcon; title: string; text: string; tag?: string }
type FeatureGroup = { id: string; label: string; blurb: string; items: Feature[] }

const teamPoints = [
  { icon: Users, title: 'One thread, many agents', text: 'Claude Code, Codex, Cursor, OpenCode and Gemini read each other’s replies and keep their own sessions.' },
  { icon: ArrowRightLeft, title: 'They hand work over', text: 'An agent writes @codex to pass the next step on. When a subscription hits its limit, a teammate picks it up.' },
  { icon: Download, title: 'Bring your history', text: 'Import chats, skills, MCP servers and rules from Claude Code, Codex, Cursor, VS Code and more.' },
]

const featureGroups: FeatureGroup[] = [
  { id: 'team', label: 'Team', blurb: 'Your subscriptions, working together.', items: [
    { icon: Users, title: 'Shared thread', text: 'Every member sees what the others said since its last turn, and resumes its own session.', tag: 'New' },
    { icon: Slash, title: 'Team skills', text: '/plan, /tickets, /execute, /review, /verify, /debate and more write specs and tickets every agent can read.', tag: 'New' },
    { icon: Undo2, title: 'Undo a turn', text: 'Each turn that changes files gets a card with the files and one-click undo.', tag: 'New' },
    { icon: GitFork, title: 'Fork and side chats', text: 'Fork a member to try another approach, or ask /btw on the side without moving the thread.', tag: 'New' },
    { icon: Layers, title: 'Worktree per agent', text: 'Give a member its own Git worktree, with your setup and teardown scripts.', tag: 'New' },
    { icon: Download, title: 'Imports', text: 'Past chats become tasks the agent can continue. Skills, MCP servers and rules come across too.', tag: 'New' },
  ] },
  { id: 'work', label: 'While it works', blurb: 'Watch progress, steer, and never get stuck.', items: [
    { icon: PenLine, title: 'Live code writing', text: 'Files appear as they are written, in VS Code colors, as compact Write and Edit rows you can expand.', tag: 'New' },
    { icon: ListChecks, title: 'To-do list', text: 'For multi-step work, Neru keeps a visible checklist and ticks it off as it goes.', tag: 'New' },
    { icon: CornerDownRight, title: 'Steering', text: 'Type while Neru works. It reads your message at its next step.', tag: 'New' },
    { icon: Network, title: 'Sub-agents', text: 'Parallel research helpers explore big codebases so the main thread stays focused.', tag: 'New' },
    { icon: Shuffle, title: 'Smart model fallback', text: 'If a model is slow, down, or rate-limited, Neru switches to the next best one on its own.', tag: 'New' },
    { icon: Sparkles, title: 'Long sessions', text: 'A context meter, effort control, and automatic compaction keep long chats healthy.' },
  ] },
  { id: 'project', label: 'Your project', blurb: 'See it run, review it, undo it, ship it.', items: [
    { icon: MonitorPlay, title: 'Live preview', text: 'One click starts your dev server and opens the app in Neru’s browser. The agent checks the page for errors itself.', tag: 'New' },
    { icon: FileText, title: 'Review every change', text: 'One pane shows everything a session changed. Leave line comments and send them back.' },
    { icon: History, title: 'Rewind', text: 'Step back to before any message, with or without undoing the file changes since.' },
    { icon: Layers, title: 'Parallel sessions', text: 'Run several sessions at once, each in its own Git worktree so changes never collide.' },
    { icon: Paperclip, title: 'Attach anything', text: 'PDFs, Word, PowerPoint, Excel, and images. Images show right in the conversation.', tag: 'New' },
    { icon: Search, title: 'Terminal tabs and search', text: 'Several terminals side by side, plus search across the whole project.', tag: 'New' },
    { icon: GitPullRequest, title: 'Ship it', text: 'Push, open pull requests, follow CI checks, and ask Neru to fix what fails.' },
  ] },
  { id: 'yours', label: 'Make it yours', blurb: 'Teach it your way of working.', items: [
    { icon: Brain, title: 'Memory', text: 'Remembers your preferences and project conventions across sessions. Manage it with /memory.', tag: 'New' },
    { icon: SquareTerminal, title: 'neru in the terminal', text: 'A full terminal agent with slash commands. It shares sessions and settings with the app.', tag: 'New' },
    { icon: Webhook, title: 'Hooks', text: 'Run your own scripts on preToolUse, postToolUse, and more from .neru/hooks.json.', tag: 'New' },
    { icon: Slash, title: '20+ slash commands', text: '/model, /mode, /resume, /cost, /doctor, /export, and your own templates in .neru/commands.', tag: 'New' },
    { icon: Plug, title: 'Connectors', text: 'Linear, Notion, Figma, GitHub, Sentry, and more over MCP.' },
    { icon: Bell, title: 'Notifications', text: 'A system notification when a background session finishes or needs you.' },
  ] },
]

const modes = ['Ask every time', 'Accept edits', 'Plan (read-only)', 'Auto', 'Bypass']

/** A gentle synthetic voice level so the glow preview breathes without a microphone. */
function useDemoLevel(active: boolean) {
  const start = useRef(performance.now())
  return () => {
    if (!active) return 0
    const t = (performance.now() - start.current) / 1000
    return Math.max(0, 0.28 + 0.22 * Math.sin(t * 2.3) + 0.12 * Math.sin(t * 5.1 + 1.2) + 0.08 * Math.sin(t * 9.7))
  }
}

type Problem = { title: string; help: string; detail?: string }

/** Turns raw provider failures into a plain sentence and a next step. */
function explainProviderError(cause: unknown, providerName: string, local: boolean): Problem {
  const raw = (cause instanceof Error ? cause.message : String(cause ?? '')).trim()
  const text = raw.toLowerCase()
  const detail = raw && raw.length < 400 ? raw : undefined
  if (/401|403|unauthori[sz]ed|forbidden|invalid.*(key|token)|incorrect api|api key/.test(text)) return { title: `${providerName} did not accept that key.`, help: 'Copy the key again from the provider’s page, without extra spaces, and try again. Keys are usually shown only once, so you may need to create a new one.', detail }
  if (/429|rate.?limit|quota|too many/.test(text)) return { title: `${providerName} says you have used up its allowance for now.`, help: 'Wait a minute and retry, or pick a different free provider. You can change this any time from the message box.', detail }
  if (/404|not found|model/.test(text) && !/network|fetch/.test(text)) return { title: 'That model name was not found.', help: 'Open Connection details and check the model ID and base URL, or clear the model field to let Neru choose.', detail }
  if (/timeout|timed out|network|fetch|dns|connect|refused|unreachable|offline|enotfound/.test(text)) return { title: local ? 'Neru could not reach your local model server.' : `Neru could not reach ${providerName}.`, help: local ? 'Make sure the server is running and that the address in Connection details is right, then retry.' : 'Check your internet connection (and any VPN or proxy), then retry. If it keeps failing, confirm the base URL in Connection details.', detail }
  if (/keychain|dpapi|secret|credential|encrypt/.test(text)) return { title: 'Neru could not store the key safely on this computer.', help: 'Retry once. If it keeps failing, restart Neru and try again.', detail }
  return { title: 'Neru could not save this connection.', help: 'Check the key and the details below, then retry. You can also skip this step and connect later in Settings.', detail }
}

function FeatureCard({ item }: { item: Feature }) {
  return <li className="ob-card"><span className="ob-icon"><item.icon size={16} /></span><div><strong>{item.title}{item.tag && <em className="ob-tag">{item.tag}</em>}</strong><small>{item.text}</small></div></li>
}

function Notice({ tone = 'error', title, children, action }: { tone?: 'error' | 'info'; title: string; children?: ReactNode; action?: ReactNode }) {
  return <div className={cn('ob-notice', tone)} role={tone === 'error' ? 'alert' : 'status'}>
    <CircleAlert size={16} aria-hidden />
    <div><strong>{title}</strong>{children && <p>{children}</p>}</div>
    {action}
  </div>
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
  const [problem, setProblem] = useState<Problem | null>(null)
  const [opening, setOpening] = useState(false)
  const [folderNote, setFolderNote] = useState<Problem | null>(null)
  const [group, setGroup] = useState(featureGroups[0].id)
  const [demo, setDemo] = useState<'listening' | 'processing'>('listening')
  const speech = useSpeechModels()
  const [name, setName] = useState('')
  const [gender, setGender] = useState('')
  useEffect(() => { if (isDesktop) void api.getProfile().then(saved => { setName(saved.name); setGender(saved.gender) }).catch(() => undefined) }, [isDesktop])
  const [agents, setAgents] = useState<TeamAgent[] | null>(null)
  useEffect(() => { if (step === STEP.team && isDesktop && agents === null) void api.listTeamAgents().then(setAgents).catch(() => setAgents([])) }, [step, isDesktop, agents])
  const level = useDemoLevel(step === STEP.voice && demo === 'listening')
  const preset = providerPresets.find(item => item.id === providerId)
  const [touched, setTouched] = useState(false)
  const [allProviders, setAllProviders] = useState(false)
  const freePresets = providerPresets.filter(item => item.freeLimit)
  const shownPresets = allProviders ? providerPresets : [...freePresets, ...(preset && !preset.freeLimit ? [preset] : [])]
  const local = /localhost|127\.0\.0\.1/.test(baseUrl)
  const headingRef = useRef<HTMLHeadingElement>(null)
  const projectRef = useRef(project)
  useEffect(() => { projectRef.current = project }, [project])
  const stepLabel = steps[step].label
  const last = step === steps.length - 1
  const hasKeyInput = apiKey.trim().length > 0
  // A key saved earlier for this provider is reused, so switching between providers never asks twice.
  const [savedKeys, setSavedKeys] = useState<string[]>([])
  useEffect(() => { if (isDesktop) void api.savedKeyProviders().then(setSavedKeys).catch(() => undefined) }, [isDesktop])
  const keySaved = savedKeys.includes(providerId) || (provider.hasKey && provider.providerId === providerId)
  const canSave = ((local && touched) || hasKeyInput || (keySaved && !saved))

  useEffect(() => {
    if (step !== STEP.voice || reduce) return
    const timer = window.setInterval(() => setDemo(current => current === 'listening' ? 'processing' : 'listening'), 3600)
    return () => window.clearInterval(timer)
  }, [step, reduce])

  // Move focus to the new step's heading so keyboard and screen-reader users land in the right place.
  useEffect(() => { headingRef.current?.focus({ preventScroll: true }) }, [step])

  const moveTo = (next: number) => {
    if (next < 0 || next >= steps.length) return
    setDirection(next > step ? 1 : -1)
    setProblem(null); setFolderNote(null)
    setStep(next)
  }

  const openFolder = async () => {
    if (opening) return
    setOpening(true); setFolderNote(null)
    try {
      await onOpenProject()
      if (!projectRef.current) setFolderNote({ title: 'No folder is open yet.', help: 'If you closed the picker, choose a folder to continue. If Neru could not open the one you picked, check that it still exists and you can read it, then try another.' })
    } catch (cause) {
      const raw = cause instanceof Error ? cause.message : String(cause ?? '')
      setFolderNote({ title: 'Neru could not open that folder.', help: 'Make sure it still exists and that you have permission to read it. Then try again, or pick a different folder.', detail: raw && raw.length < 300 ? raw : undefined })
    } finally { setOpening(false) }
  }

  const choosePreset = (id: string) => {
    const next = providerPresets.find(item => item.id === id)
    if (!next) return
    setTouched(true); setProviderId(next.id); setApiFormat(formatForModel(next.id, next.model, next.format, next.baseUrl)); setBaseUrl(next.baseUrl); setModel(next.model); setApiKey(''); setSaved(false); setProblem(null)
    if (!next.model) setDetailsOpen(true)
  }

  const saveProvider = async (): Promise<boolean> => {
    if (!isDesktop) { setProblem({ title: 'Connecting a model needs the desktop app.', help: 'Open the Neru desktop window to save a provider. You can skip this step for now.' }); return false }
    if (!local && !hasKeyInput && !(saved || keySaved)) { setProblem({ title: 'Paste an API key first.', help: `Get a free key from ${preset?.name ?? 'your provider'} with the link below, paste it here, then connect.` }); return false }
    setSaving(true); setProblem(null)
    try {
      if (model && !LISTING_IS_AUTHORITATIVE.has(providerId)) {
        // The suggested model may have been retired or not be served to this key; find out now, not on the first message.
        const check = await api.probeModel(providerId, apiFormat, baseUrl, apiKey, model).catch(() => null)
        if (check?.status === 'unavailable') { setProblem({ title: `${model} is not available with this key.`, help: `${check.reason}. Open the details and pick another model.` }); setDetailsOpen(true); return false }
      }
      onProviderSaved(await api.configureProvider(providerId, apiFormat, baseUrl, apiKey, model))
      setApiKey(''); setSaved(true); setSavedKeys(current => current.includes(providerId) ? current : [...current, providerId])
      return true
    } catch (cause) {
      setProblem(explainProviderError(cause, preset?.name ?? 'The provider', local)); setDetailsOpen(true)
      return false
    } finally { setSaving(false) }
  }

  // Each step has one primary action; skippable steps also get a quiet secondary one.
  const primary = async () => {
    if (last) { onFinish(); return }
    if (step === STEP.you && isDesktop) await api.setProfile(name, gender).catch(() => undefined)
    if (step === STEP.project && !project) { await openFolder(); return }
    if (step === STEP.model && !saved && canSave) { if (await saveProvider()) moveTo(step + 1); return }
    moveTo(step + 1)
  }
  const skippable = step === STEP.you || step === STEP.project || step === STEP.model || step === STEP.voice || step === STEP.team
  const busy = saving || opening
  const primaryLabel = last ? 'Open workspace'
    : step === STEP.welcome ? 'Get started'
    : step === STEP.you ? (name.trim() ? `Nice to meet you, ${name.trim().split(/\s+/)[0]}` : 'Continue')
    : step === STEP.project ? (project ? 'Continue' : 'Choose a folder')
    : step === STEP.model ? (saving ? 'Connecting…' : saved ? 'Continue' : canSave ? 'Connect and continue' : 'Continue')
    : step === STEP.voice ? (speech.active ? 'Continue' : 'Continue without voice')
    : step === STEP.team ? 'Continue'
    : 'Continue'
  const skipLabel = step === STEP.project ? 'I’ll do this later' : step === STEP.model ? 'Skip for now' : 'Skip'

  const keyHandler = useRef<(event: KeyboardEvent) => void>(() => undefined)
  const handleKey = (event: KeyboardEvent) => {
    if (event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey) return
    const target = event.target as HTMLElement | null
    const typing = !!target?.closest('input, textarea, select, [contenteditable="true"]')
    if (event.key === 'Escape') {
      if (typing) { (target as HTMLElement).blur(); return }
      if (skippable && !busy) { event.preventDefault(); moveTo(step + 1) }
      return
    }
    if (typing || target?.closest('[role="tablist"], [role="radiogroup"]')) return
    if (event.key === 'ArrowRight') { event.preventDefault(); moveTo(step + 1) }
    else if (event.key === 'ArrowLeft') { event.preventDefault(); moveTo(step - 1) }
    else if (event.key === 'Enter' && !target?.closest('button, a, summary')) { event.preventDefault(); if (!busy) void primary() }
  }
  useEffect(() => { keyHandler.current = handleKey })
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => keyHandler.current(event)
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const onGroupKey = (event: React.KeyboardEvent) => {
    const index = featureGroups.findIndex(item => item.id === group)
    const delta = event.key === 'ArrowRight' ? 1 : event.key === 'ArrowLeft' ? -1 : 0
    if (!delta) return
    event.preventDefault()
    const next = featureGroups[(index + delta + featureGroups.length) % featureGroups.length]
    setGroup(next.id)
    requestAnimationFrame(() => document.getElementById(`ob-tab-${next.id}`)?.focus())
  }

  const slide = reduce ? 0 : direction * 20
  const activeGroup = featureGroups.find(item => item.id === group) ?? featureGroups[0]

  return <main className="onboarding" aria-label="Getting started with Neru">
    <header className={cn('ob-header', /Mac/i.test(navigator.platform) && 'mac')} data-tauri-drag-region>
      <div className="ob-brand" data-tauri-drag-region><Mascot size={26} /><span>Neru <span lang="ja">練る</span></span></div>
      <div className="ob-header-actions">
        <a className="ob-credit" href={author.url} target="_blank" rel="noreferrer">Kneaded by <strong>{author.handle}</strong></a>
        <button type="button" className="ob-quiet" onClick={onToggleTheme} aria-label={light ? 'Use dark appearance' : 'Use light appearance'}>{light ? <Moon size={14} /> : <Sun size={14} />}<span>{light ? 'Dark' : 'Light'}</span></button>
        <button type="button" className="ob-quiet" onClick={onFinish}>Skip setup</button>
      </div>
      <WindowControls />
    </header>

    <nav className="ob-stepper" aria-label={`Setup progress, step ${step + 1} of ${steps.length}: ${stepLabel}`}>
      <ol>
        {steps.map((item, index) => {
          const state = index < step ? 'done' : index === step ? 'current' : 'upcoming'
          return <li key={item.id} className={state}>
            <button type="button" aria-current={state === 'current' ? 'step' : undefined} onClick={() => moveTo(index)}>
              <span className="ob-bar"><i /></span>
              <span className="ob-step-label"><b>{state === 'done' ? <Check size={11} strokeWidth={3} /> : index + 1}</b>{item.label}</span>
            </button>
          </li>
        })}
      </ol>
    </nav>

    <section className="ob-main">
      <div className="ob-scroll">
        <AnimatePresence mode="wait" initial={false}>
          <motion.div key={step} className={cn('ob-panel', step === STEP.toolkit && 'wide')} initial={{ opacity: 0, x: slide }} animate={{ opacity: 1, x: 0 }} exit={{ opacity: 0, x: -slide }} transition={{ duration: reduce ? 0 : 0.28, ease: EASE_OUT }}>
            <p className="ob-eyebrow">Step {step + 1} of {steps.length}<span aria-hidden> · </span><span>{stepLabel}</span></p>

            {step === STEP.welcome && <div className="ob-welcome">
              <div className="ob-welcome-copy">
                <h1 ref={headingRef} tabIndex={-1}>A quiet place to think <em>with</em> your code.</h1>
                <p className="ob-lede">Neru is a local-first coding agent. It explores your project, proposes changes you can read line by line, and never acts without asking.</p>
                <ul className="ob-highlights">{highlights.map(item => <li key={item.title}><span className="ob-icon"><item.icon size={17} /></span><div><strong>{item.title}</strong><small>{item.text}</small></div></li>)}</ul>
                <p className="ob-note">Setup takes about two minutes and every step but the last can be skipped.</p>
              </div>
              <div className="ob-hero" aria-hidden><div className="ob-hero-glow" /><Mascot size={132} interactive /></div>
            </div>}

            {step === STEP.you && <>
              <h1 ref={headingRef} tabIndex={-1}>What should Neru call you?</h1>
              <p className="ob-lede">Every model you work with will know your name and how to address you. Change it any time in Settings, General.</p>
              <label className="ob-field"><span>Your name</span><input value={name} onChange={event => setName(event.target.value)} placeholder="First name" autoComplete="given-name" maxLength={80} /></label>
              <div className="ob-providers" role="radiogroup" aria-label="Gender">
                {genders.map(item => <button key={item.value} type="button" role="radio" aria-checked={gender === item.value} className={cn('ob-provider', gender === item.value && 'selected')} onClick={() => setGender(item.value)}><strong>{item.label}</strong></button>)}
              </div>
              <p className="ob-note">Used for pronouns, and for masculine or feminine forms in languages like French or Arabic.</p>
            </>}

            {step === STEP.project && <>
              <h1 ref={headingRef} tabIndex={-1}>Open a project.</h1>
              <p className="ob-lede">Pick a folder with your code. Your files stay where they are, and each project keeps its own sessions.</p>
              <button type="button" className={cn('ob-folder', project && 'chosen')} onClick={() => void openFolder()} disabled={!isDesktop || opening}>
                <span className="ob-folder-icon">{opening ? <LoaderCircle size={22} className="animate-spin" /> : project ? <FolderOpen size={22} /> : <Folder size={22} />}</span>
                <span className="ob-folder-text"><strong>{project ? project.name : opening ? 'Waiting for the folder picker…' : 'Choose a folder'}</strong><small>{project ? project.path : isDesktop ? 'A repository or any code folder' : 'Available in the desktop app'}</small></span>
                {project ? <span className="ob-pill"><Check size={13} /> Ready</span> : <ArrowRight size={18} aria-hidden />}
              </button>
              {project && <button type="button" className="ob-link" onClick={() => void openFolder()} disabled={opening}>Choose a different folder</button>}
              {!isDesktop && <Notice tone="info" title="You are in the browser preview.">Opening folders works in the Neru desktop app.</Notice>}
              {folderNote && !project && <Notice title={folderNote.title} action={<button type="button" className="ob-notice-btn" onClick={() => void openFolder()} disabled={opening}>Try again</button>}>{folderNote.help}{folderNote.detail && <code>{folderNote.detail}</code>}</Notice>}
              <p className="ob-note">You can open more projects later from the sidebar.</p>
            </>}

            {step === STEP.model && <>
              <h1 ref={headingRef} tabIndex={-1}>Connect a model.</h1>
              <p className="ob-lede">Pick a provider and paste its key. Several have free daily allowances that work well for coding, and you can switch models any time from the message box.</p>
              <div className="ob-providers" role="radiogroup" aria-label="Model provider">
                {shownPresets.map(item => <button key={item.id} type="button" role="radio" aria-checked={providerId === item.id} className={cn('ob-provider', providerId === item.id && 'selected')} onClick={() => choosePreset(item.id)}
                  onKeyDown={event => { if (event.key !== 'ArrowRight' && event.key !== 'ArrowLeft' && event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return; event.preventDefault(); const i = shownPresets.findIndex(p => p.id === providerId); const d = event.key === 'ArrowRight' || event.key === 'ArrowDown' ? 1 : -1; const next = shownPresets[(i + d + shownPresets.length) % shownPresets.length]; choosePreset(next.id); requestAnimationFrame(() => (event.currentTarget.parentElement?.querySelector('[aria-checked="true"]') as HTMLElement | null)?.focus()) }}
                  tabIndex={providerId === item.id || !shownPresets.some(p => p.id === providerId) ? 0 : -1}>
                  <strong>{item.name}</strong><small>{item.description}</small>
                  {item.freeLimit && <span className="ob-provider-free">Free · {item.freeLimit}</span>}
                  {providerId === item.id && <motion.span layoutId="provider-check" className="ob-provider-check" transition={{ duration: reduce ? 0 : 0.25, ease: EASE_OUT }}><Check size={12} strokeWidth={2.5} /></motion.span>}
                </button>)}
              </div>
              <button type="button" className="ob-link" onClick={() => setAllProviders(value => !value)}>{allProviders ? 'Show only free coding providers' : `More providers (${providerPresets.length - freePresets.length}): local models, OpenAI, Anthropic, DeepSeek…`}</button>
              <form className="ob-connection" onSubmit={event => { event.preventDefault(); void (async () => { if (await saveProvider()) moveTo(step + 1) })() }}>
                {!local && <label className="ob-field"><span><KeyRound size={13} /> API key <small>encrypted on this PC</small></span><input type="password" value={apiKey} onChange={event => { setApiKey(event.target.value); setSaved(false); setProblem(null) }} placeholder={keySaved ? 'Key saved on this PC. Paste a new one to replace it.' : `Paste your ${preset?.name ?? ''} key`} autoComplete="off" spellCheck={false} aria-invalid={problem ? true : undefined} /></label>}
                {preset?.keyUrl && !local && <p className="ob-key-link">{preset.freeLimit && <span>Free: {preset.freeLimit}. </span>}<a href={preset.keyUrl} target="_blank" rel="noreferrer">Get a {preset.name} API key ↗</a></p>}
                <button type="button" className="ob-disclosure" aria-expanded={detailsOpen} onClick={() => setDetailsOpen(value => !value)}>Connection details <motion.span animate={{ rotate: detailsOpen ? 180 : 0 }} transition={{ duration: reduce ? 0 : 0.2 }}><ChevronDown size={14} /></motion.span></button>
                <AnimatePresence initial={false}>{detailsOpen && <motion.div className="ob-details" initial={{ opacity: 0, height: 0 }} animate={{ opacity: 1, height: 'auto' }} exit={{ opacity: 0, height: 0 }} transition={{ duration: reduce ? 0 : 0.25, ease: EASE_OUT }}>
                  <div className="ob-field-row">
                    <label className="ob-field"><span>Model</span><input value={model} onChange={event => { const next = event.target.value; setModel(next); setApiFormat(formatForModel(providerId, next, apiFormat, baseUrl)); setSaved(false) }} placeholder="Model ID, e.g. auto" /></label>
                    <label className="ob-field"><span>API format</span><select value={apiFormat} onChange={event => { setApiFormat(event.target.value as ApiFormat); setSaved(false) }}><option value="openai-chat">Chat Completions</option><option value="openai-responses">Responses</option><option value="anthropic">Anthropic Messages</option></select></label>
                  </div>
                  <label className="ob-field"><span>Base URL</span><input value={baseUrl} onChange={event => { setBaseUrl(event.target.value); setSaved(false) }} autoComplete="url" /></label>
                </motion.div>}</AnimatePresence>
                <div className="ob-connection-actions">
                  <span className={cn('ob-status', saved && 'ok')}>{saved ? <><Check size={14} /> Connected to {preset?.name ?? 'provider'}</> : local ? 'No key needed for local models.' : 'Your key is sent only to this provider.'}</span>
                  <button type="submit" className="ob-secondary" disabled={saving || (!canSave && !saved)}>{saving ? <><LoaderCircle size={14} className="animate-spin" /> Connecting…</> : saved ? 'Reconnect' : 'Connect'}</button>
                </div>
                {problem && <Notice title={problem.title} action={<button type="button" className="ob-notice-btn" onClick={() => void saveProvider()} disabled={saving}>{saving ? 'Retrying…' : 'Retry'}</button>}>{problem.help}{problem.detail && <code>{problem.detail}</code>}</Notice>}
              </form>
            </>}

            {step === STEP.voice && <>
              <h1 ref={headingRef} tabIndex={-1}>Talk to Neru. <span className="ob-optional">Optional</span></h1>
              <p className="ob-lede">Press the microphone in the message box and speak. A small model on this machine turns your voice into text, and it never leaves the computer.</p>
              <div className="ob-voice-demo" aria-hidden>
                <VoiceBeam level={level} processing={demo === 'processing'} theme={light ? 'light' : 'dark'} {...neruVoiceGlow(light)} className="ob-beam">
                  <div className="ob-demo-input">
                    <span className="ob-demo-text">{demo === 'listening' ? 'Refactor the sidebar so it remembers its width…' : 'Transcribing…'}</span>
                    <span className="ob-demo-controls"><span className="ob-demo-pill">Review</span>
                      {demo === 'listening'
                        ? <span className="ob-demo-voice"><span className="voice-status"><i />0:04</span><span className="voice-button"><Pause size={14} /></span><span className="voice-button"><X size={15} /></span><span className="voice-button confirm"><Check size={15} /></span><span className="voice-button send"><ArrowUp size={16} /></span></span>
                        : <span className="ob-demo-voice"><span className="voice-status"><LoaderCircle size={14} className="animate-spin" />Transcribing…</span><span className="voice-button"><X size={15} /></span></span>}
                    </span>
                  </div>
                </VoiceBeam>
              </div>
              <label className="ob-field ob-language"><span><Globe size={13} /> Dictation language</span><select value={language} onChange={event => onLanguageChange(event.target.value)}>{dictationLanguages.map(item => <option key={item.value} value={item.value}>{item.label}</option>)}</select></label>
              <SpeechModelCatalog compact />
              <p className="ob-note">{speech.active ? 'Your model is ready. Change it any time in Settings, Voice.' : 'Whisper Base is a good start for English, French, and Spanish. Small or Turbo handle Arabic best. You can also add one later.'}</p>
            </>}

            {step === STEP.team && <>
              <h1 ref={headingRef} tabIndex={-1}>Your agents, <em>one thread</em>. <span className="ob-optional">Optional</span></h1>
              <p className="ob-lede">Already pay for Claude, ChatGPT or Cursor? Team puts their coding agents in one conversation, where they share context and hand work to each other. Neru uses each one’s own sign-in and never sees your tokens.</p>
              <div className="ob-agents" aria-live="polite">
                {!isDesktop ? <p className="ob-note">Agent detection works in the desktop app.</p>
                  : agents === null ? <p className="ob-note"><LoaderCircle size={14} className="animate-spin" /> Looking for agents on this computer…</p>
                  : agents.map(agent => <div key={agent.kind} className={cn('ob-agent', !agent.path && 'missing')}>
                    <AgentMark kind={agent.kind} size={30} />
                    <span><strong>{agent.name}</strong><small>{!agent.path ? 'Not installed' : agent.signedIn ? 'Ready' : 'Installed, sign in from Team'}</small></span>
                    {agent.path && agent.signedIn && <Check size={15} className="ob-agent-ok" aria-label="Ready" />}
                  </div>)}
              </div>
              <ul className="ob-highlights">{teamPoints.map(item => <li key={item.title}><span className="ob-icon"><item.icon size={17} /></span><div><strong>{item.title}</strong><small>{item.text}</small></div></li>)}</ul>
              <p className="ob-note">Find it under <strong>Team</strong> in the sidebar. A short walkthrough plays the first time you open it.</p>
            </>}

            {step === STEP.toolkit && <>
              <h1 ref={headingRef} tabIndex={-1}>What’s <em>inside</em>.</h1>
              <p className="ob-lede">You don’t need to learn any of this now. It’s all there when the task grows, from a quick question to a pull request with passing checks.</p>
              <div className="ob-tabs" role="tablist" aria-label="Feature groups" onKeyDown={onGroupKey}>
                {featureGroups.map(item => <button key={item.id} id={`ob-tab-${item.id}`} type="button" role="tab" aria-selected={group === item.id} aria-controls="ob-tabpanel" tabIndex={group === item.id ? 0 : -1} className={cn(group === item.id && 'on')} onClick={() => setGroup(item.id)}>{item.label}<span>{item.items.length}</span></button>)}
              </div>
              <div id="ob-tabpanel" role="tabpanel" aria-labelledby={`ob-tab-${activeGroup.id}`}>
                <p className="ob-blurb">{activeGroup.blurb}</p>
                <ul className="ob-cards" key={activeGroup.id}>{activeGroup.items.map(item => <FeatureCard key={item.title} item={item} />)}</ul>
              </div>
              <p className="ob-engine"><Rocket size={14} aria-hidden /> Built on a native Rust core (Tauri), so parallel sessions stay quick and light on memory. Speech, files, and history stay on your machine. <span>Tip: type <kbd>/</kbd> in the message box to see every command.</span></p>
            </>}

            {step === STEP.decide && <>
              <h1 ref={headingRef} tabIndex={-1}>Nothing changes <em>without you</em>.</h1>
              <p className="ob-lede">When Neru wants to edit a file or run a command, it stops and shows you exactly what will happen.</p>
              <div className="ob-approval" aria-hidden>
                <ToolApproval tool="terminal.run" title="Allow this command to run?" description="Neru wants to run the test suite in your project." defaultOpen parameters={[{ id: 'cmd', label: 'Command', value: <ToolApprovalCode code="npm run test" /> }]} onApprove={() => undefined} onAlwaysAllow={() => undefined} onDeny={() => undefined} />
              </div>
              <ul className="ob-rules">
                <li><Sparkles size={15} /> Allow once, always allow a trusted command, or deny.</li>
                <li><ShieldCheck size={15} /> File edits show a full diff and save a checkpoint first, so you can rewind.</li>
                <li><KeyRound size={15} /> Choose how much Neru does alone with the permission mode under the message box.</li>
              </ul>
              <div className="ob-modes" aria-label="Permission modes">{modes.map(item => <span key={item}>{item}</span>)}</div>
            </>}
          </motion.div>
        </AnimatePresence>
      </div>

      <footer className="ob-footer">
        <button className="ob-back" type="button" onClick={() => moveTo(step - 1)} disabled={step === 0}><ArrowLeft size={16} /> Back</button>
        <span className="ob-hints" aria-hidden><kbd>Enter</kbd> continue{skippable && <><kbd>Esc</kbd> skip</>}</span>
        <div className="ob-footer-actions">
          {skippable && <button type="button" className="ob-skip" onClick={() => moveTo(step + 1)} disabled={busy}>{skipLabel}</button>}
          <button className="ob-next" type="button" onClick={() => void primary()} disabled={busy}>
            {busy && <LoaderCircle size={15} className="animate-spin" />}{primaryLabel} {!busy && <ArrowRight size={16} />}
          </button>
        </div>
      </footer>
    </section>
  </main>
}
