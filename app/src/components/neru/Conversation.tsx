import { useEffect, useRef, useState, type ReactNode } from 'react'
import { Check, ChevronDown, CircleAlert, History, LoaderCircle, Paperclip, ShieldCheck } from 'lucide-react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { ThinkingOrb, type OrbState } from 'thinking-orbs'
import { AgentDisclosure } from '@/components/agents/agent-disclosure'
import { languageForPath } from '@/components/agents/agent-code'
import { Citation, type CitationItem } from '@/components/agents/citations'
import { FileDiff } from '@/components/agents/file-diff'
import { MessageScroller } from '@/components/agents/message-scroller'
import { StreamingResponse, type StreamingResponseFeedback } from '@/components/agents/streaming-response'
import { ToolApproval, ToolApprovalCode, type ToolApprovalStatus } from '@/components/agents/tool-approval'
import { EASE_OUT, SPRING_SWAP } from '@/lib/ease'
import { diffCounts, parseUnifiedDiff } from '@/lib/diff'
import { cn } from '@/lib/utils'
import { Mascot } from './Mascot'
import type { ChatEntry, PendingView, Source, ToolEventStatus } from '../../types'

export interface LiveTool { id: string; label: string; status: ToolEventStatus }
export interface LiveResponse { text: string; tools: LiveTool[]; sources: Source[] }
/** A settled approval, shown after the message that was last when it was decided. */
export interface ResolvedApproval { id: string; after: string | null; pending: PendingView; status: ToolApprovalStatus }
export interface AgentPhase { state: OrbState; label: string }

/** Response typography: Claude-like 16px reading size on top of StreamingResponse's own prose styles. */
const RESPONSE_PROSE = 'text-base leading-7 text-foreground [&_a]:text-[var(--sage)] [&_h1]:mb-2 [&_h1]:mt-6 [&_h1]:text-xl [&_h1]:font-semibold [&_h2]:mb-2 [&_h2]:mt-5 [&_h2]:text-lg [&_h2]:font-semibold [&_h3]:mb-1.5 [&_h3]:mt-4 [&_h3]:text-base [&_h3]:font-semibold [&_pre]:text-[13px] [&_pre]:leading-6 [&_blockquote]:border-l-2 [&_blockquote]:border-border [&_blockquote]:pl-4 [&_blockquote]:text-muted-foreground [&_table]:my-3 [&_table]:w-full [&_table]:text-sm [&_td]:border-b [&_td]:border-border [&_td]:px-2 [&_td]:py-1.5 [&_th]:border-b [&_th]:border-border [&_th]:px-2 [&_th]:py-1.5 [&_th]:text-left [&_th]:font-medium [&_li>p]:my-0 [&>:first-child]:mt-0'

const WEB_TOOL = /^(Searched the web|Read [a-z0-9.-]+\.[a-z]{2,}$)/i

/** Maps what the agent is doing right now onto a thinking-orb state and a short label. */
export function agentPhase(live: LiveResponse | null, mode: string): AgentPhase {
  const running = live?.tools.findLast(tool => tool.status === 'running')
  if (running) {
    if (running.label.startsWith('Searched the web')) return { state: 'searching', label: 'Searching the web…' }
    if (WEB_TOOL.test(running.label)) return { state: 'connecting', label: 'Reading sources…' }
    if (running.label.startsWith('Proposed change')) return { state: 'shaping', label: 'Shaping a change…' }
    if (/^(Listed|Read|Searched project|Checked Git|Read Git)/.test(running.label)) return { state: 'connecting', label: 'Exploring the project…' }
    return { state: 'weaving', label: 'Running command…' }
  }
  if (live?.text) return { state: 'composing', label: 'Composing…' }
  if (live && live.tools.length > 0) return { state: 'shaping', label: 'Shaping…' }
  return mode === 'plan' ? { state: 'solving', label: 'Planning…' } : { state: 'working', label: 'Thinking…' }
}

/** Holds each phase on screen for a moment, so quick tool calls do not make the orb flicker. */
function useSteadyPhase(phase: AgentPhase, dwell = 700): AgentPhase {
  const [shown, setShown] = useState(phase)
  const since = useRef(0)
  useEffect(() => {
    if (phase.state === shown.state && phase.label === shown.label) return
    const wait = Math.max(0, dwell - (performance.now() - since.current))
    const timer = window.setTimeout(() => { since.current = performance.now(); setShown(phase) }, wait)
    return () => window.clearTimeout(timer)
  }, [phase, shown, dwell])
  return shown
}

/** Seconds since the status appeared, shown once a step takes a while. */
function useElapsed() {
  const [seconds, setSeconds] = useState(0)
  useEffect(() => {
    const start = performance.now()
    const timer = window.setInterval(() => setSeconds(Math.floor((performance.now() - start) / 1000)), 1000)
    return () => window.clearInterval(timer)
  }, [])
  return seconds
}

const elapsedText = (seconds: number) => seconds < 60 ? `${seconds}s` : `${Math.floor(seconds / 60)}m ${String(seconds % 60).padStart(2, '0')}s`

export function AgentStatus({ phase }: { phase: AgentPhase }) {
  const reduce = useReducedMotion() ?? false
  const steady = useSteadyPhase(phase)
  const seconds = useElapsed()
  const fade = { duration: reduce ? 0 : 0.5, ease: EASE_OUT }
  return <div className="agent-status" role="status" aria-live="polite" aria-label={steady.label}>
    {/* The same orbs, cross-faded between states instead of swapped. */}
    <span className="agent-orb" aria-hidden>
      <AnimatePresence initial={false}>
        <motion.span key={steady.state} className="agent-orb-layer"
          initial={reduce ? { opacity: 0 } : { opacity: 0, scale: 0.72, filter: 'blur(2px)' }}
          animate={{ opacity: 1, scale: 1, filter: 'blur(0px)' }}
          exit={reduce ? { opacity: 0 } : { opacity: 0, scale: 1.18, filter: 'blur(2px)' }}
          transition={fade}>
          <ThinkingOrb state={steady.state} size={20} speed={0.9} />
        </motion.span>
      </AnimatePresence>
    </span>
    <span className="agent-label" aria-hidden>
      <AnimatePresence mode="popLayout" initial={false}>
        <motion.span key={steady.label} className="agent-shimmer"
          initial={reduce ? { opacity: 0 } : { opacity: 0, y: 7, filter: 'blur(3px)' }}
          animate={{ opacity: 1, y: 0, filter: 'blur(0px)' }}
          exit={reduce ? { opacity: 0 } : { opacity: 0, y: -7, filter: 'blur(3px)' }}
          transition={{ duration: reduce ? 0 : 0.34, ease: EASE_OUT }}>{steady.label}</motion.span>
      </AnimatePresence>
    </span>
    <AnimatePresence>{seconds >= 3 && <motion.span className="agent-elapsed" aria-hidden initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} transition={fade}>{elapsedText(seconds)}</motion.span>}</AnimatePresence>
  </div>
}

const toCitations = (sources: Source[]): CitationItem[] => sources.map(source => ({ id: source.id, title: source.title, domain: source.domain, url: source.url }))

/** Turns [1] / [1, 2] markers into citation links, leaving code untouched. */
function linkCitations(content: string, count: number) {
  return content.split(/(```[\s\S]*?```|`[^`\n]*`)/g).map((part, index) => index % 2 === 1 ? part : part.replace(/\[(\d+(?:\s*,\s*\d+)*)\](?![(:[])/g, (match, list: string) => {
    const numbers = list.split(',').map(value => Number(value.trim()))
    return numbers.every(number => number >= 1 && number <= count) ? numbers.map(number => `[${number}](#cite-${number})`).join('') : match
  })).join('')
}

function ResponseMarkdown({ content, sources, idPrefix, onCite }: { content: string; sources: Source[]; idPrefix: string; onCite: () => void }) {
  return <ReactMarkdown remarkPlugins={[remarkGfm]} components={{
    a: ({ href, children }) => {
      const cite = href?.match(/^#cite-(\d+)$/)
      const source = cite ? sources[Number(cite[1]) - 1] : undefined
      if (cite && source) return <span onClickCapture={onCite}><Citation citationId={source.id} index={Number(cite[1])} idPrefix={idPrefix} /></span>
      return <a href={href}>{children}</a>
    },
  }}>{sources.length ? linkCitations(content, sources.length) : content}</ReactMarkdown>
}

function ToolIcon({ status }: { status: ToolEventStatus }) {
  const reduce = useReducedMotion() ?? false
  const icon = status === 'running' ? <LoaderCircle size={13} className={cn(!reduce && 'animate-spin')} />
    : status === 'error' ? <CircleAlert size={13} className="text-destructive" />
    : status === 'pending' ? <ShieldCheck size={13} className="text-amber-500" />
    : <Check size={13} className="text-[var(--sage)]" />
  // The spinner settles into its result with a small spring rather than a hard swap.
  return <span className="tool-step-icon"><AnimatePresence mode="popLayout" initial={false}>
    <motion.span key={status} initial={reduce ? { opacity: 0 } : { opacity: 0, scale: 0.4, rotate: -30 }} animate={{ opacity: 1, scale: 1, rotate: 0 }} exit={{ opacity: 0, scale: 0.4 }} transition={reduce ? { duration: 0 } : SPRING_SWAP}>{icon}</motion.span>
  </AnimatePresence></span>
}

/** A one-line recap of finished steps, e.g. "Read 3 files · Edited 1 file · Ran 2 commands". */
function summarize(tools: LiveTool[]) {
  const counts = new Map<string, number>()
  const add = (key: string) => counts.set(key, (counts.get(key) ?? 0) + 1)
  for (const tool of tools) {
    const label = tool.label
    if (/^Read Git|^Checked Git/.test(label)) add('git')
    else if (/^Read [a-z0-9.-]+\.[a-z]{2,}$/i.test(label) || /^Searched the web/.test(label)) add('web')
    else if (/^Read /.test(label)) add('read')
    else if (/^(Searched project|Found files|Listed)/.test(label)) add('search')
    else if (/^(Deleted|Proposed deleting)/.test(label)) add('delete')
    else if (/^(Moved|Proposed moving)/.test(label)) add('move')
    else if (/^(Created folder|Proposed folder)/.test(label)) add('folder')
    else if (/^Created /.test(label)) add('create')
    else if (/^(Edited|Proposed change)/.test(label)) add('edit')
    else if (/^(Ran|Run|Requested)/.test(label)) add('run')
    else add('other')
  }
  const plural = (count: number, one: string, many: string) => `${count} ${count === 1 ? one : many}`
  const parts: string[] = []
  const take = (key: string, text: (count: number) => string) => { const count = counts.get(key); if (count) parts.push(text(count)) }
  take('read', count => `Read ${plural(count, 'file', 'files')}`)
  take('search', count => `${plural(count, 'search', 'searches')}`)
  take('edit', count => `Edited ${plural(count, 'file', 'files')}`)
  take('create', count => `Created ${plural(count, 'file', 'files')}`)
  take('folder', count => `${plural(count, 'new folder', 'new folders')}`)
  take('move', count => `Moved ${plural(count, 'item', 'items')}`)
  take('delete', count => `Deleted ${plural(count, 'item', 'items')}`)
  take('run', count => `Ran ${plural(count, 'command', 'commands')}`)
  take('web', count => `${plural(count, 'web lookup', 'web lookups')}`)
  take('git', count => `${plural(count, 'Git check', 'Git checks')}`)
  take('other', count => `${plural(count, 'other step', 'other steps')}`)
  return parts.join(' · ')
}

function ToolSteps({ tools, live }: { tools: LiveTool[]; live: boolean }) {
  const reduce = useReducedMotion() ?? false
  const [open, setOpen] = useState(false)
  if (tools.length === 0) return null
  const expanded = live || open
  const failed = tools.filter(tool => tool.status === 'error').length
  return <div className="tool-steps">
    {!live && <button type="button" className="tool-steps-toggle" aria-expanded={open} onClick={() => setOpen(value => !value)}>
      <span className="truncate">{summarize(tools) || `${tools.length} ${tools.length === 1 ? 'step' : 'steps'}`}{failed > 0 && <span className="tool-steps-failed"> · {failed} failed</span>}</span>
      <motion.span aria-hidden animate={{ rotate: open ? 180 : 0 }} transition={reduce ? { duration: 0 } : SPRING_SWAP}><ChevronDown size={13} /></motion.span>
    </button>}
    <AgentDisclosure open={expanded}>
      <ol className="tool-steps-list">
        <AnimatePresence initial={false}>
          {tools.map(tool => <motion.li key={tool.id} data-status={tool.status} layout={reduce ? false : 'position'}
            initial={reduce ? { opacity: 0 } : { opacity: 0, x: -6, filter: 'blur(2px)' }}
            animate={{ opacity: 1, x: 0, filter: 'blur(0px)' }}
            transition={{ duration: reduce ? 0 : 0.32, ease: EASE_OUT }}>
            <ToolIcon status={tool.status} /><span className={cn('truncate', tool.status === 'running' && 'agent-shimmer')} title={tool.label}>{tool.label}</span>
          </motion.li>)}
        </AnimatePresence>
      </ol>
    </AgentDisclosure>
  </div>
}

function Message({ from, children }: { from: 'user' | 'assistant'; children: ReactNode }) {
  return <article data-slot="message" data-from={from} className={`message ${from}`}>{children}</article>
}

function AssistantMessage({ content, sources, tools, status, live, feedback, onFeedback, onRetry, footer }: {
  content: string; sources: Source[]; tools: LiveTool[]; status: 'streaming' | 'complete' | 'error'; live: boolean
  feedback?: StreamingResponseFeedback; onFeedback?: (value: StreamingResponseFeedback) => void; onRetry?: () => void; footer?: ReactNode
}) {
  const [sourcesOpen, setSourcesOpen] = useState(false)
  const [idPrefix] = useState(() => `sources-${crypto.randomUUID().slice(0, 8)}`)
  return <Message from="assistant">
    <span className="message-avatar"><Mascot size={22} /></span>
    <div className="message-body" data-slot="message-content">
      <ToolSteps tools={tools} live={live} />
      {(content || status !== 'streaming') && <StreamingResponse status={status} copyText={content} onRetry={onRetry} sources={toCitations(sources)} sourceIdPrefix={idPrefix} sourcesOpen={sourcesOpen} onSourcesOpenChange={setSourcesOpen} feedback={feedback} onFeedbackChange={onFeedback} announce={false} contentClassName={RESPONSE_PROSE}>
        <ResponseMarkdown content={content} sources={sources} idPrefix={idPrefix} onCite={() => setSourcesOpen(true)} />
      </StreamingResponse>}
      {footer}
    </div>
  </Message>
}

export function ApprovalCard({ pending, status, projectPath, onApprove, onAlwaysAllow, onDeny }: {
  pending: PendingView; status: ToolApprovalStatus; projectPath?: string; onApprove?: () => void; onAlwaysAllow?: () => void; onDeny?: () => void
}) {
  if (pending.kind === 'move' || pending.kind === 'mkdir') {
    const [from, to] = pending.label.split(' → ')
    return <ToolApproval tool={pending.kind === 'move' ? 'move_path' : 'create_folder'} title={pending.kind === 'move' ? 'Rename or move this?' : 'Create this folder?'}
      description={pending.kind === 'move' ? 'Missing folders at the destination are created. A checkpoint lets rewind move it back.' : 'An empty folder, with any missing parent folders.'}
      status={status} defaultOpen={status === 'pending'} onApprove={onApprove} onDeny={onDeny}
      parameters={pending.kind === 'move' ? [{ id: 'from', label: 'From', value: <ToolApprovalCode code={from} language="text" /> }, { id: 'to', label: 'To', value: <ToolApprovalCode code={to ?? ''} language="text" /> }] : [{ id: 'path', label: 'Folder', value: <ToolApprovalCode code={pending.label} language="text" /> }]} />
  }
  if (pending.kind === 'delete') {
    const lines = parseUnifiedDiff(pending.diff ?? '')
    const counts = diffCounts(lines)
    return <div className="approval-stack">
      {counts.removed > 0 && <FileDiff file={pending.label} lines={lines} status="complete" collapseOnComplete={false} defaultOpen={false} maxHeight={280} language={languageForPath(pending.label)} copyText={pending.diff ?? ''} />}
      <ToolApproval tool={`delete_file · ${pending.label}`} title="Delete this file?" description={`${counts.removed > 0 ? `${counts.removed} ${counts.removed === 1 ? 'line' : 'lines'} removed with it.` : 'A binary file.'} A checkpoint is saved so rewind can bring it back.`} status={status} onApprove={onApprove} onDeny={onDeny} />
    </div>
  }
  if (pending.kind === 'edit') {
    const lines = parseUnifiedDiff(pending.diff ?? '')
    const counts = diffCounts(lines)
    return <div className="approval-stack">
      <FileDiff file={pending.label} lines={lines} status="complete" collapseOnComplete={false} defaultOpen={status === 'pending'} maxHeight={360} language={languageForPath(pending.label)} copyText={pending.diff ?? ''} />
      <ToolApproval tool={`edit_file · ${pending.label}`} title="Apply this change?" description={`${counts.added} ${counts.added === 1 ? 'line' : 'lines'} added, ${counts.removed} removed. A checkpoint is saved so you can undo it.`} status={status} onApprove={onApprove} onDeny={onDeny} />
    </div>
  }
  const task = /^npm run (build|test|lint)$/.test(pending.label)
  return <ToolApproval tool={task ? 'project.task' : 'terminal.run'} title={task ? 'Allow this project task to run?' : 'Allow this command to run?'} description="Neru wants to run this in the project folder. Output returns to the agent."
    status={status} defaultOpen={status === 'pending'} onApprove={onApprove} onAlwaysAllow={onAlwaysAllow} onDeny={onDeny}
    parameters={[{ id: 'command', label: 'Command', value: <ToolApprovalCode code={pending.label} language={task ? 'bash' : 'powershell'} /> }, ...(projectPath ? [{ id: 'cwd', label: 'Directory', value: projectPath }] : [])]} />
}

/** Rewind control under a sent message: back to before it, optionally restoring files too. */
function RewindMenu({ disabled, onRewind }: { disabled: boolean; onRewind: (restoreCode: boolean) => void }) {
  const [open, setOpen] = useState(false)
  const root = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!open) return
    const close = (event: PointerEvent) => { if (!root.current?.contains(event.target as Node)) setOpen(false) }
    const escape = (event: KeyboardEvent) => { if (event.key === 'Escape') setOpen(false) }
    window.addEventListener('pointerdown', close); window.addEventListener('keydown', escape)
    return () => { window.removeEventListener('pointerdown', close); window.removeEventListener('keydown', escape) }
  }, [open])
  const choose = (restoreCode: boolean) => { setOpen(false); onRewind(restoreCode) }
  return <div className={cn('rewind', open && 'open')} ref={root}>
    <button type="button" className="rewind-trigger" disabled={disabled} onClick={() => setOpen(value => !value)} aria-haspopup="menu" aria-expanded={open} title={disabled ? 'Wait for the response to finish' : 'Rewind to before this message'}><History size={13} /> Rewind</button>
    {open && <div className="rewind-menu" role="menu">
      <button type="button" role="menuitem" onClick={() => choose(false)}><strong>Rewind conversation</strong><span>Remove this message and everything after it. Files stay as they are.</span></button>
      <button type="button" role="menuitem" onClick={() => choose(true)}><strong>Rewind conversation and code</strong><span>Also undo the file changes Neru made since this message.</span></button>
    </div>}
  </div>
}

export interface ConversationProps {
  messages: ChatEntry[]
  live: LiveResponse | null
  phase: AgentPhase | null
  busy: boolean
  pending: PendingView | null
  pendingStatus: ToolApprovalStatus
  resolved: ResolvedApproval[]
  failed: string | null
  projectPath?: string
  feedback: Record<string, StreamingResponseFeedback>
  onFeedback: (id: string, value: StreamingResponseFeedback) => void
  onRetry: () => void
  onApprove: () => void
  onAlwaysAllow: () => void
  onDeny: () => void
  /** Rewinds to before the n-th request you sent (0-based). */
  onRewind: (userIndex: number, restoreCode: boolean) => void
}

export function Conversation(props: ConversationProps) {
  const { messages, live, phase, busy, pending, resolved, failed } = props
  const settledAfter = (id: string | null) => resolved.filter(item => item.after === id).map(item => <div className="approval-row" key={item.id}><ApprovalCard pending={item.pending} status={item.status} projectPath={props.projectPath} /></div>)
  const known = new Set(messages.map(entry => entry.id))
  let userIndex = -1
  return <MessageScroller navigation="rail" busy={busy} className="chat-scroller" viewportClassName="chat-viewport" contentClassName="chat-content">
    {settledAfter(null)}
    {messages.flatMap(entry => { if (entry.role === 'user') userIndex += 1; const index = userIndex; return [entry.role === 'user'
      ? <Message from="user" key={entry.id}><div className="message-body" data-slot="message-bubble-content">
          {entry.contextPaths && entry.contextPaths.length > 0 && <div className="message-context">{entry.contextPaths.map(path => <span key={path}><Paperclip size={12} />{path}</span>)}</div>}
          <p>{entry.content}</p>
        </div><RewindMenu disabled={busy} onRewind={restoreCode => props.onRewind(index, restoreCode)} /></Message>
      : <AssistantMessage key={entry.id} content={entry.content} sources={entry.sources ?? []} tools={(entry.steps ?? []).map((label, index) => ({ id: `${entry.id}-${index}`, label, status: label.endsWith('(failed)') ? 'error' : 'done' }))} status="complete" live={false} feedback={props.feedback[entry.id] ?? null} onFeedback={value => props.onFeedback(entry.id, value)} />, ...settledAfter(entry.id)] })}
    {resolved.filter(item => item.after !== null && !known.has(item.after)).map(item => <div className="approval-row" key={item.id}><ApprovalCard pending={item.pending} status={item.status} projectPath={props.projectPath} /></div>)}
    {live && <AssistantMessage content={live.text} sources={live.sources} tools={live.tools} status="streaming" live footer={phase && <AgentStatus phase={phase} />} />}
    {!live && busy && phase && <div className="approval-row"><AgentStatus phase={phase} /></div>}
    {failed && !busy && <AssistantMessage content={`**The request failed.** ${failed}`} sources={[]} tools={[]} status="error" live={false} onRetry={props.onRetry} />}
    {pending && <div className="approval-row"><ApprovalCard pending={pending} status={props.pendingStatus} projectPath={props.projectPath} onApprove={props.onApprove} onAlwaysAllow={pending.kind === 'task' ? props.onAlwaysAllow : undefined} onDeny={props.onDeny} /></div>}
  </MessageScroller>
}
