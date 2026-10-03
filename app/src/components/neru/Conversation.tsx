import { createContext, isValidElement, useContext, useEffect, useRef, useState, type ReactNode } from 'react'
import { Check, ChevronDown, CircleAlert, Globe, History, ListChecks, LoaderCircle, MessageCircleQuestion, Paperclip, Play, ShieldCheck, X } from 'lucide-react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { ThinkingOrb, type OrbState } from 'thinking-orbs'
import { AgentDisclosure } from '@/components/agents/agent-disclosure'
import { languageForFence, languageForPath } from '@/components/agents/agent-code'
import { CodeBlock } from '@/components/agents/code-block'
import { FileWriteCard, type FileWriteState } from '@/components/agents/file-write-card'
import { Citation, type CitationItem } from '@/components/agents/citations'
import { FileDiff } from '@/components/agents/file-diff'
import { MessageScroller } from '@/components/agents/message-scroller'
import { StreamingResponse, type StreamingResponseFeedback } from '@/components/agents/streaming-response'
import { ToolApproval, ToolApprovalCode, type ToolApprovalStatus } from '@/components/agents/tool-approval'
import { EASE_OUT, SPRING_PANEL, SPRING_SWAP } from '@/lib/ease'
import TaskRows, { SpinnerRing, TaskBadge, type TaskRow } from '@/components/primitives/TaskRows'
import { diffCounts, parseUnifiedDiff } from '@/lib/diff'
import { cn } from '@/lib/utils'
import { remarkMentions } from '@/lib/mentions'
import { api } from '../../api'
import { Mascot } from './Mascot'
import { MermaidBlock, WireframeBlock } from './Diagrams'
import { ErrorNotice } from './ErrorNotice'
import type { ErrorAction } from '@/lib/friendlyError'
import type { AgentMode, ChatEntry, PendingView, Source, SubagentApproval, SubagentProgress, ToolEventStatus } from '../../types'

/** A step of the reply. `agent` carries a sub-agent's live progress; `since` is when it started, for a ticking clock. */
export interface LiveTool { id: string; label: string; status: ToolEventStatus; agent?: SubagentProgress & { since: number } }
/** A file the model is writing, streamed from its unfinished tool call. */
export interface LiveDraft { id: string; path: string; content: string; edit?: boolean; state?: FileWriteState }
export interface LiveResponse { text: string; tools: LiveTool[]; sources: Source[]; drafts: LiveDraft[]; reasoning: number; thinking?: string }
/** A settled approval, shown after the message that was last when it was decided. */
export interface ResolvedApproval { id: string; after: string | null; pending: PendingView; status: ToolApprovalStatus }
/** A line in the thread for something that happened to the session: a model switch, a compaction. */
function NoteLine({ text, working }: { text: string; working?: boolean }) {
  return <div className="session-note" role="status">{working && <LoaderCircle size={12} className="animate-spin" aria-hidden />}<span>{text}</span></div>
}
export interface AgentPhase { state: OrbState; label: string }

/** Response typography: Claude-like 16px reading size on top of StreamingResponse's own prose styles. */
export const RESPONSE_PROSE = 'neru-prose text-base leading-7 text-foreground [&_h1]:mb-2 [&_h1]:mt-6 [&_h1]:text-xl [&_h1]:font-semibold [&_h2]:mb-2 [&_h2]:mt-5 [&_h2]:text-lg [&_h2]:font-semibold [&_h3]:mb-1.5 [&_h3]:mt-4 [&_h3]:text-base [&_h3]:font-semibold [&_pre]:text-[13px] [&_pre]:leading-6 [&_blockquote]:border-l-2 [&_blockquote]:border-border [&_blockquote]:pl-4 [&_blockquote]:text-muted-foreground [&_table]:my-3 [&_table]:w-full [&_table]:text-sm [&_td]:border-b [&_td]:border-border [&_td]:px-2 [&_td]:py-1.5 [&_th]:border-b [&_th]:border-border [&_th]:px-2 [&_th]:py-1.5 [&_th]:text-left [&_th]:font-medium [&_li>p]:my-0 [&>:first-child]:mt-0'

const WEB_TOOL = /^(Searched the web|Read [a-z0-9.-]+\.[a-z]{2,}$)/i

/** Maps what the agent is doing right now onto a thinking-orb state and a short label. */
/** Drafts whose tool call has not been handled yet, i.e. still being written. */
const writing = (live: LiveResponse) => live.drafts.filter(draft => !live.tools.some(tool => tool.id === draft.id))

export function agentPhase(live: LiveResponse | null, mode: string): AgentPhase {
  const draft = live && writing(live).at(-1)
  if (draft) return { state: 'shaping', label: `Writing ${draft.path.split(/[\\/]/).pop() || 'a file'}…` }
  const running = live?.tools.findLast(tool => tool.status === 'running')
  if (running) {
    const agents = live?.tools.filter(tool => tool.status === 'running' && tool.agent).length ?? 0
    if (agents > 0) return { state: 'connecting', label: agents === 1 ? 'A sub-agent is exploring…' : `${agents} sub-agents are exploring…` }
    if (running.label.startsWith('Searched the web')) return { state: 'searching', label: 'Searching the web…' }
    if (WEB_TOOL.test(running.label)) return { state: 'connecting', label: 'Reading sources…' }
    if (running.label.startsWith('Proposed change')) return { state: 'shaping', label: 'Shaping a change…' }
    if (/^(Listed|Read|Searched project|Checked Git|Read Git|Mapped|Looked up|Found)/.test(running.label)) return { state: 'connecting', label: 'Exploring the project…' }
    return { state: 'weaving', label: 'Running command…' }
  }
  if (live?.text) return { state: 'composing', label: 'Composing…' }
  if (live && live.tools.length > 0) return { state: 'shaping', label: 'Shaping…' }
  if (live && live.reasoning > 0) return { state: 'solving', label: 'Reasoning…' }
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

/** `mentions`: handles to highlight as @mentions (Team members). */
export function ResponseMarkdown({ content, sources, idPrefix, onCite, mentions }: { content: string; sources: Source[]; idPrefix: string; onCite: () => void; mentions?: string[] }) {
  return <ReactMarkdown remarkPlugins={mentions ? [remarkGfm, [remarkMentions, { handles: mentions }]] : [remarkGfm]} components={{
    a: ({ href, children }) => {
      const cite = href?.match(/^#cite-(\d+)$/)
      const source = cite ? sources[Number(cite[1]) - 1] : undefined
      if (cite && source) return <span onClickCapture={onCite}><Citation citationId={source.id} index={Number(cite[1])} idPrefix={idPrefix} /></span>
      return <a href={href}>{children}</a>
    },
    // Fenced code renders like the VS Code editor; inline code keeps the prose style.
    pre: ({ children }) => {
      const child = Array.isArray(children) ? children[0] : children
      if (!isValidElement(child)) return <pre>{children}</pre>
      const props = child.props as { className?: string; children?: ReactNode }
      const tag = /language-([\w+#.-]+)/.exec(props.className ?? '')?.[1]
      const source = String(props.children ?? '').replace(/\n$/, '')
      if (tag === 'mermaid') return <MermaidBlock code={source} />
      if (tag === 'wireframe') return <WireframeBlock html={source} />
      return <CodeBlock code={source} language={languageForFence(tag)} />
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
    if (/^Agent/.test(label)) add('agent')
    else if (/^Read Git|^Checked Git/.test(label)) add('git')
    // Web pages show as "Opened example.com"; older sessions said "Read example.com", which must not catch "Read index.html".
    else if (/^Opened |^Searched the web/.test(label) || /^Read (?:[a-z0-9-]+\.)+(?:com|org|net|io|dev|ai|app|co|edu|gov|me|info)$/i.test(label)) add('web')
    else if (/^Read /.test(label)) add('read')
    else if (/^(Searched project|Found files|Listed|Mapped|Looked up)/.test(label)) add('search')
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
  take('agent', count => `${plural(count, 'sub-agent', 'sub-agents')}`)
  take('web', count => `${plural(count, 'web lookup', 'web lookups')}`)
  take('git', count => `${plural(count, 'Git check', 'Git checks')}`)
  take('other', count => `${plural(count, 'other step', 'other steps')}`)
  return parts.join(' · ')
}

/** Counts up while a sub-agent works; settles on the server's final time. */
function useAgentClock(agent: NonNullable<LiveTool['agent']>, running: boolean) {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (!running) return
    const timer = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(timer)
  }, [running])
  return running ? Math.max(0, Math.floor((now - agent.since) / 1000)) : Math.floor(agent.elapsedMs / 1000)
}

/** An edit or command a sub-agent is waiting on, under the same mode and rules as the main agent. */
function SubagentApprovalCard({ approval, agent }: { approval: SubagentApproval; agent: string }) {
  const [answer, setAnswer] = useState<'pending' | 'sending' | 'error'>('pending')
  const reply = (approve: boolean) => {
    setAnswer('sending')
    void api.answerSubagentApproval(approval.requestId, approve).catch(() => setAnswer('error'))
  }
  const command = approval.kind === 'task'
  const verb = command ? 'run' : approval.kind === 'delete' ? 'delete' : approval.kind === 'move' ? 'move' : approval.kind === 'mkdir' ? 'create' : 'change'
  return <div className="subagent-approval" role="group" aria-label={`Approve a sub-agent's request to ${verb} ${approval.label}`}>
    <p><ShieldCheck size={13} aria-hidden /> <strong>{agent}</strong> wants to {verb} {command ? '' : <code>{approval.label}</code>}</p>
    {command ? <ToolApprovalCode code={approval.label} /> : approval.diff ? <FileDiff file={approval.label} lines={parseUnifiedDiff(approval.diff)} status="complete" collapseOnComplete={false} defaultOpen maxHeight={280} language={languageForPath(approval.label)} copyText={approval.diff} /> : null}
    <div className="subagent-approval-actions">
      <button type="button" className="button primary" disabled={answer === 'sending'} onClick={() => reply(true)}><Check size={13} /> Allow</button>
      <button type="button" className="button subtle" disabled={answer === 'sending'} onClick={() => reply(false)}><X size={13} /> Deny</button>
      {answer === 'error' && <span className="subagent-approval-note">The agent stopped before this was answered.</span>}
    </div>
  </div>
}

/** A sub-agent's activity, nested under its task: role, what it is doing now, tool count, time, and its steps. */
function SubagentRow({ tool }: { tool: LiveTool & { agent: NonNullable<LiveTool['agent']> } }) {
  const reduce = useReducedMotion() ?? false
  const { agent } = tool
  const running = tool.status === 'running' && agent.status !== 'done'
  const [open, setOpen] = useState(true)
  const seconds = useAgentClock(agent, running)
  const shown = open && agent.steps.length > 0
  return <div className="subagent" data-status={running ? 'running' : tool.status}>
    <button type="button" className="subagent-head" aria-expanded={shown} onClick={() => setOpen(value => !value)} disabled={agent.steps.length === 0}>
      <span className="subagent-role">{agent.role}</span>
      <span className={cn('subagent-title truncate', running && 'agent-shimmer')} title={agent.description}>{agent.description}</span>
      <span className="subagent-meta">{agent.tools} {agent.tools === 1 ? 'tool' : 'tools'} · {elapsedText(seconds)}</span>
      {agent.steps.length > 0 && <motion.span aria-hidden animate={{ rotate: shown ? 180 : 0 }} transition={reduce ? { duration: 0 } : SPRING_SWAP}><ChevronDown size={12} /></motion.span>}
    </button>
    {running && agent.current && <div className="subagent-current truncate" title={agent.current}>{agent.current}</div>}
    {running && agent.approval && <SubagentApprovalCard key={agent.approval.requestId} approval={agent.approval} agent={agent.description} />}
    <AgentDisclosure open={shown}>
      <ol className="subagent-steps">
        {agent.steps.map((label, index) => <li key={`${index}-${label}`}><span className="truncate" title={label}>{label}</span></li>)}
      </ol>
    </AgentDisclosure>
  </div>
}

export function ToolSteps({ tools, live }: { tools: LiveTool[]; live: boolean }) {
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
          {tools.map(tool => <motion.li key={tool.id} data-status={tool.status} className={tool.agent ? 'has-agent' : undefined} layout={reduce ? false : 'position'}
            initial={reduce ? { opacity: 0 } : { opacity: 0, x: -6, filter: 'blur(2px)' }}
            animate={{ opacity: 1, x: 0, filter: 'blur(0px)' }}
            transition={{ duration: reduce ? 0 : 0.32, ease: EASE_OUT }}>
            <ToolIcon status={tool.status} />
            {tool.agent ? <SubagentRow tool={tool as LiveTool & { agent: NonNullable<LiveTool['agent']> }} /> : <span className={cn('truncate', tool.status === 'running' && 'agent-shimmer')} title={tool.label}>{tool.label}</span>}
          </motion.li>)}
        </AnimatePresence>
      </ol>
    </AgentDisclosure>
  </div>
}

/** Ids of messages added while the conversation is on screen; only those get the entrance animation. */
const FreshContext = createContext<Set<string>>(new Set())

function Message({ from, id, children }: { from: 'user' | 'assistant'; id?: string; children: ReactNode }) {
  const reduce = useReducedMotion() ?? false
  const fresh = useContext(FreshContext)
  const animate = !reduce && id !== undefined && fresh.has(id)
  // A sent message rises out of the composer and settles, like Claude's.
  // The filter ends at 'none' rather than blur(0px): a leftover filter makes every message its own
  // stacking layer, and later messages would then paint over an open menu (the rewind menu).
  const enter = from === 'user'
    ? { initial: { opacity: 0, y: 36, scale: 0.9, filter: 'blur(6px)' }, transition: { type: 'spring' as const, duration: 0.6, bounce: 0.32, opacity: { duration: 0.2, ease: EASE_OUT }, filter: { duration: 0.3, ease: EASE_OUT } } }
    : { initial: { opacity: 0, y: 12, filter: 'blur(3px)' }, transition: { duration: 0.42, ease: EASE_OUT } }
  return <motion.article data-slot="message" data-from={from} className={cn('message', from, animate && 'just-sent')}
    style={{ transformOrigin: from === 'user' ? '100% 100%' : '0% 0%' }}
    initial={animate ? enter.initial : false} animate={{ opacity: 1, y: 0, scale: 1, filter: 'none', transitionEnd: { transform: 'none' } }} transition={enter.transition}>{children}</motion.article>
}

/** Remembers which messages arrived after the conversation was shown (not a whole session loading at once). */
function useFresh(ids: string[]) {
  const seen = useRef<Set<string> | null>(null)
  const fresh = useRef(new Set<string>())
  if (seen.current === null) seen.current = new Set(ids)
  const added = ids.filter(id => !seen.current!.has(id))
  // Many new ids at once means a different session was opened, not a message sent.
  if (added.length > 0 && added.length <= 3) added.forEach(id => fresh.current.add(id))
  added.forEach(id => seen.current!.add(id))
  return fresh.current
}

/** Where a written file stands, from its tool call's status (no status yet: still streaming). */
export const draftState = (draft: LiveDraft, tools: LiveTool[]): FileWriteState => {
  if (draft.state) return draft.state
  const tool = tools.find(item => item.id === draft.id)
  return !tool ? 'writing' : tool.status === 'error' ? 'error' : tool.status === 'pending' ? 'pending' : 'done'
}

/** Files written in this reply, each a Claude Code style Write/Edit row with the newest lines streaming. */
function CodeDrafts({ drafts, tools }: { drafts: LiveDraft[]; tools: LiveTool[] }) {
  if (drafts.length === 0) return null
  return <div className="code-drafts">
    {drafts.map(draft => <FileWriteCard key={draft.id} path={draft.path} content={draft.content} edit={draft.edit} state={draftState(draft, tools)} />)}
  </div>
}

/** The model's reasoning, collapsed like Claude's "Thinking" block; open while it streams. */
function Thinking({ text, live }: { text: string; live: boolean }) {
  const reduce = useReducedMotion() ?? false
  const [open, setOpen] = useState(false)
  const body = useRef<HTMLDivElement>(null)
  useEffect(() => { if (live && open && body.current) body.current.scrollTop = body.current.scrollHeight }, [text, live, open])
  if (!text.trim()) return null
  return <div className="thinking">
    <button type="button" className="thinking-toggle" aria-expanded={open} onClick={() => setOpen(value => !value)}>
      <span className={cn(live && 'agent-shimmer')}>{live ? 'Thinking…' : 'Thought process'}</span>
      <motion.span aria-hidden animate={{ rotate: open ? 180 : 0 }} transition={reduce ? { duration: 0 } : SPRING_SWAP}><ChevronDown size={13} /></motion.span>
    </button>
    <AgentDisclosure open={open}><div ref={body} className="thinking-body">{text.trim()}</div></AgentDisclosure>
  </div>
}

/** The agent's to-do list, pinned above the composer while there is unfinished work, drawn with Beautiful UI's Task Rows. */
export function TodoPanel({ todos }: { todos: { content: string; status: string }[] }) {
  const [open, setOpen] = useState(true)
  const reduce = useReducedMotion() ?? false
  if (todos.length === 0) return null
  const done = todos.filter(todo => todo.status === 'completed').length
  const finished = done === todos.length
  if (finished && !open) return null
  const current = todos.find(todo => todo.status === 'in_progress')
  const rows: TaskRow[] = todos.map((todo, index) => ({
    key: `${index}`,
    label: todo.content,
    step: index + 1,
    status: todo.status === 'completed' ? 'done' : todo.status === 'in_progress' ? 'running' : 'pending',
  }))
  return <motion.div className={cn('todo-panel', finished && 'finished')} initial={reduce ? false : { opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }} transition={reduce ? { duration: 0 } : SPRING_PANEL}>
    <button type="button" className="todo-head" aria-expanded={open} onClick={() => setOpen(value => !value)}>
      <span className="task-row-mark" key={finished ? 'done' : 'ring'}>
        {finished ? <TaskBadge tone="green" /> : <SpinnerRing progress={done / todos.length}>{done}</SpinnerRing>}
      </span>
      <AnimatePresence mode="wait" initial={false}>
        <motion.span key={current?.content ?? (finished ? 'finished' : 'plan')} className={cn('todo-title', current && 'running')}
          initial={reduce ? false : { opacity: 0, y: 5 }} animate={{ opacity: 1, y: 0 }} exit={reduce ? undefined : { opacity: 0, y: -5 }} transition={{ duration: 0.2, ease: EASE_OUT }}>
          {current ? current.content : finished ? 'All tasks done' : 'Plan'}
        </motion.span>
      </AnimatePresence>
      <span className="todo-count">{done} of {todos.length}</span>
      <span className="todo-chevron" aria-hidden><svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round" style={{ transform: open ? 'rotate(180deg)' : 'none' }}><path d="M6 9l6 6 6-6" /></svg></span>
    </button>
    {/* Same expanding grammar as the primitive's detail drop-down: a grid row that grows from 0fr. */}
    <div className={cn('todo-drawer', open && 'open')}><div><TaskRows rows={rows} /></div></div>
  </motion.div>
}

function AssistantMessage({ id, content, sources, tools, drafts = [], thinking = '', status, live, feedback, onFeedback, onRetry, footer }: {
  id?: string; content: string; sources: Source[]; tools: LiveTool[]; drafts?: LiveDraft[]; thinking?: string; status: 'streaming' | 'complete' | 'error'; live: boolean
  feedback?: StreamingResponseFeedback; onFeedback?: (value: StreamingResponseFeedback) => void; onRetry?: () => void; footer?: ReactNode
}) {
  const [sourcesOpen, setSourcesOpen] = useState(false)
  const [idPrefix] = useState(() => `sources-${crypto.randomUUID().slice(0, 8)}`)
  return <Message from="assistant" id={id}>
    <span className="message-avatar"><Mascot size={36} /></span>
    <div className="message-body" data-slot="message-content">
      <Thinking text={thinking} live={live && !content && tools.length === 0} />
      <ToolSteps tools={tools} live={live} />
      <CodeDrafts drafts={drafts} tools={tools} />
      {(content || status !== 'streaming') && <StreamingResponse status={status} copyText={content} onRetry={onRetry} sources={toCitations(sources)} sourceIdPrefix={idPrefix} sourcesOpen={sourcesOpen} onSourcesOpenChange={setSourcesOpen} feedback={feedback} onFeedbackChange={onFeedback} announce={false} contentClassName={RESPONSE_PROSE}>
        <ResponseMarkdown content={content} sources={sources} idPrefix={idPrefix} onCite={() => setSourcesOpen(true)} />
      </StreamingResponse>}
      {footer}
    </div>
  </Message>
}

/** A plan from Plan mode: approve it (and pick how edits are handled) or keep planning with feedback. */
export function PlanCard({ pending, status, onResolve }: { pending: PendingView; status: ToolApprovalStatus; onResolve?: PlanResolver }) {
  const [asking, setAsking] = useState(false)
  const [feedback, setFeedback] = useState('')
  const open = status === 'pending' && Boolean(onResolve)
  const badge = status === 'pending' ? 'Waiting for you' : status === 'approving' ? 'Sending…' : status === 'complete' ? 'Approved' : 'Kept planning'
  return <section className="agent-prompt" data-state={status} aria-label="Neru's plan">
    <header className="agent-prompt-head"><ListChecks size={16} aria-hidden /><strong>Neru's plan</strong><span className="agent-prompt-badge">{badge}</span></header>
    <div className={cn('agent-prompt-plan', RESPONSE_PROSE, !open && 'settled')}><ResponseMarkdown content={pending.diff ?? ''} sources={[]} idPrefix="plan" onCite={() => undefined} /></div>
    {pending.feedback && <p className="agent-prompt-note">You asked: {pending.feedback}</p>}
    {open && onResolve && <>
      <div className="agent-prompt-actions">
        <button type="button" className="button primary" onClick={() => onResolve(true, 'accept_edits', '')}>Approve and let Neru edit</button>
        <button type="button" className="button subtle" onClick={() => onResolve(true, 'manual', '')}>Approve, ask before each change</button>
        <button type="button" className="button subtle" aria-expanded={asking} onClick={() => setAsking(value => !value)}>Keep planning</button>
      </div>
      {asking && <form className="agent-prompt-feedback" onSubmit={event => { event.preventDefault(); onResolve(false, null, feedback) }}>
        <input autoFocus value={feedback} onChange={event => setFeedback(event.target.value)} placeholder="What should change? (optional)" aria-label="What should change in the plan" />
        <button type="submit" className="button subtle">Send</button>
      </form>}
    </>}
  </section>
}

/** Multiple-choice questions from the agent, each with a typed "Other" answer. */
export function QuestionCard({ pending, status, onAnswer }: { pending: PendingView; status: ToolApprovalStatus; onAnswer?: (answers: string[]) => void }) {
  const questions = pending.questions ?? []
  const [picked, setPicked] = useState<string[][]>(() => questions.map(() => []))
  const [other, setOther] = useState<string[]>(() => questions.map(() => ''))
  const open = status === 'pending' && Boolean(onAnswer)
  const answers = questions.map((_, index) => [...(picked[index] ?? []), (other[index] ?? '').trim()].filter(Boolean).join(', '))
  const toggle = (index: number, label: string, multi: boolean) => {
    setPicked(current => current.map((list, at) => at !== index ? list : multi ? (list.includes(label) ? list.filter(item => item !== label) : [...list, label]) : [label]))
    if (!multi) setOther(current => current.map((text, at) => at === index ? '' : text))
  }
  const type = (index: number, text: string, multi: boolean) => {
    setOther(current => current.map((value, at) => at === index ? text : value))
    if (!multi && text) setPicked(current => current.map((list, at) => at === index ? [] : list))
  }
  const badge = status === 'pending' ? 'Waiting for you' : status === 'approving' ? 'Sending…' : status === 'complete' ? 'Answered' : 'Skipped'
  return <section className="agent-prompt" data-state={status} aria-label="Questions from Neru">
    <header className="agent-prompt-head"><MessageCircleQuestion size={16} aria-hidden /><strong>{questions.length === 1 ? 'Neru has a question' : `Neru has ${questions.length} questions`}</strong><span className="agent-prompt-badge">{badge}</span></header>
    {questions.map((question, index) => <fieldset className="agent-question" key={index} disabled={!open}>
      <legend>{question.header && <span className="agent-question-header">{question.header}</span>}{question.question}</legend>
      {pending.answers ? <p className="agent-prompt-note">{pending.answers[index] || 'No answer'}</p> : <>
        <div className="agent-question-options" role={question.multiSelect ? 'group' : 'radiogroup'}>
          {question.options.map(option => {
            const on = picked[index]?.includes(option.label) ?? false
            return <button type="button" key={option.label} role={question.multiSelect ? 'checkbox' : 'radio'} aria-checked={on} className={cn('agent-question-option', on && 'on')} onClick={() => toggle(index, option.label, question.multiSelect)}>
              <span className={cn('agent-question-mark', question.multiSelect ? 'box' : 'dot')} aria-hidden>{on && <Check size={11} strokeWidth={3} />}</span>
              <span className="agent-question-text"><strong>{option.label}</strong>{option.description && <span>{option.description}</span>}</span>
            </button>
          })}
        </div>
        <input className="agent-question-other" value={other[index] ?? ''} onChange={event => type(index, event.target.value, question.multiSelect)} placeholder={question.multiSelect ? 'Something else (optional)' : 'Other: type your own answer'} aria-label={`Other answer to: ${question.question}`} />
      </>}
    </fieldset>)}
    {open && onAnswer && <div className="agent-prompt-actions">
      <button type="button" className="button primary" disabled={!answers.every(Boolean)} onClick={() => onAnswer(answers)}>Submit {questions.length === 1 ? 'answer' : 'answers'}</button>
      {!answers.every(Boolean) && <span className="agent-prompt-hint">Answer every question to continue.</span>}
    </div>}
  </section>
}

export type PlanResolver = (approve: boolean, mode: AgentMode | null, feedback: string) => void

export function ApprovalCard({ pending, status, projectPath, onApprove, onAlwaysAllow, onDeny, onResolvePlan, onAnswer }: {
  pending: PendingView; status: ToolApprovalStatus; projectPath?: string; onApprove?: () => void; onAlwaysAllow?: () => void; onDeny?: () => void
  onResolvePlan?: PlanResolver; onAnswer?: (answers: string[]) => void
}) {
  if (pending.kind === 'plan') return <PlanCard pending={pending} status={status} onResolve={onResolvePlan} />
  if (pending.kind === 'question') return <QuestionCard pending={pending} status={status} onAnswer={onAnswer} />
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

/** Images you sent, as thumbnails in your message (like Claude); click one to see it full size. */
function MessageImages({ images }: { images: string[] }) {
  const [open, setOpen] = useState<number | null>(null)
  const reduce = useReducedMotion() ?? false
  useEffect(() => {
    if (open === null) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setOpen(null)
      if (event.key === 'ArrowRight') setOpen(index => index === null ? null : (index + 1) % images.length)
      if (event.key === 'ArrowLeft') setOpen(index => index === null ? null : (index - 1 + images.length) % images.length)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open, images.length])
  return <>
    <div className={cn('message-images', images.length === 1 && 'single')}>
      {images.map((src, index) => <button type="button" key={index} className="message-image" onClick={() => setOpen(index)} aria-label={`Open image ${index + 1}`}>
        <img src={src} alt="" loading="lazy" decoding="async" />
      </button>)}
    </div>
    <AnimatePresence>{open !== null && <motion.div className="image-lightbox" role="dialog" aria-label="Image" onClick={() => setOpen(null)}
      initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} transition={{ duration: reduce ? 0 : 0.18 }}>
      <motion.img src={images[open]} alt="" onClick={event => event.stopPropagation()}
        initial={reduce ? false : { scale: 0.94, opacity: 0 }} animate={{ scale: 1, opacity: 1 }} exit={reduce ? undefined : { scale: 0.96, opacity: 0 }} transition={{ type: 'spring', duration: 0.35, bounce: 0.15 }} />
      <button type="button" className="image-lightbox-close" aria-label="Close" onClick={() => setOpen(null)}>×</button>
    </motion.div>}</AnimatePresence>
  </>
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
  /** A session event still in progress (“Compacting conversation…”), shown below the thread. */
  progressNote?: string | null
  failed: string | null
  projectPath?: string
  feedback: Record<string, StreamingResponseFeedback>
  onFeedback: (id: string, value: StreamingResponseFeedback) => void
  onRetry: () => void
  onApprove: () => void
  onAlwaysAllow: () => void
  onDeny: () => void
  onResolvePlan?: PlanResolver
  onAnswer?: (answers: string[]) => void
  /** A fix offered by an error notice (open settings, compact, …). */
  onErrorAction?: (action: ErrorAction) => void
  /** A reply built something viewable: offer to open it in the Browser section. */
  previewOffer?: boolean
  onOpenPreview?: () => void
  onDismissPreview?: () => void
  /** Rewinds to before the n-th request you sent (0-based). */
  onRewind: (userIndex: number, restoreCode: boolean) => void
}

export function Conversation(props: ConversationProps) {
  const { messages, live, phase, busy, pending, resolved, failed } = props
  const known = new Set(messages.map(entry => entry.id))
  const settledAfter = (id: string | null) => resolved.filter(item => item.after === id).map(item => <div className="approval-row" key={item.id}><ApprovalCard pending={item.pending} status={item.status} projectPath={props.projectPath} /></div>)
  const fresh = useFresh(messages.map(entry => entry.id))
  let userIndex = -1
  return <FreshContext.Provider value={fresh}><MessageScroller navigation="rail" busy={busy} className="chat-scroller" viewportClassName="chat-viewport" contentClassName="chat-content">
    {settledAfter(null)}
    {messages.flatMap(entry => { if (entry.role === 'note') return [<NoteLine key={entry.id} text={entry.content} />, ...settledAfter(entry.id)]; if (entry.role === 'user') userIndex += 1; const index = userIndex; return [entry.role === 'user'
      ? <Message from="user" id={entry.id} key={entry.id}>{entry.images && entry.images.length > 0 && <MessageImages images={entry.images} />}{(entry.content || (entry.contextPaths?.length ?? 0) > 0) && <div className="message-body" data-slot="message-bubble-content">
          {entry.contextPaths && entry.contextPaths.length > 0 && <div className="message-context">{entry.contextPaths.map(path => <span key={path}><Paperclip size={12} />{path}</span>)}</div>}
          {entry.content && <p>{entry.content}</p>}
        </div>}<RewindMenu disabled={busy} onRewind={restoreCode => props.onRewind(index, restoreCode)} /></Message>
      : <AssistantMessage key={entry.id} id={entry.id} content={entry.content} sources={entry.sources ?? []} tools={(entry.steps ?? []).map((label, index) => ({ id: `${entry.id}-${index}`, label, status: label.endsWith('(failed)') ? 'error' : 'done' }))} drafts={entry.files} thinking={entry.thinking} status="complete" live={false} feedback={props.feedback[entry.id] ?? null} onFeedback={value => props.onFeedback(entry.id, value)} />, ...settledAfter(entry.id)] })}
    {resolved.filter(item => item.after !== null && !known.has(item.after)).map(item => <div className="approval-row" key={item.id}><ApprovalCard pending={item.pending} status={item.status} projectPath={props.projectPath} /></div>)}
    {props.progressNote && <NoteLine text={props.progressNote} working />}
    {live && <AssistantMessage content={live.text} sources={live.sources} tools={live.tools} drafts={live.drafts} thinking={live.thinking} status="streaming" live footer={phase && <AgentStatus phase={phase} />} />}
    {!live && busy && phase && <div className="approval-row"><AgentStatus phase={phase} /></div>}
    {failed && !busy && <Message from="assistant"><span className="message-avatar"><Mascot size={36} /></span><div className="message-body"><ErrorNotice error={failed} onAction={action => action === 'retry' ? props.onRetry() : props.onErrorAction?.(action)} /></div></Message>}
    {props.previewOffer && !pending && <div className="approval-row"><div className="preview-offer" role="status">
      <span className="preview-offer-icon" aria-hidden><Globe size={16} /></span>
      <span className="preview-offer-text"><strong>Your app is ready to try</strong><span>Open it in Neru's browser. The dev server starts automatically.</span></span>
      <button type="button" className="button primary" onClick={props.onOpenPreview}><Play size={14} /> Open preview</button>
      <button type="button" className="icon-button" aria-label="Dismiss" onClick={props.onDismissPreview}><X size={14} /></button>
    </div></div>}
    {pending && <div className="approval-row"><ApprovalCard pending={pending} status={props.pendingStatus} projectPath={props.projectPath} onApprove={props.onApprove} onAlwaysAllow={pending.kind === 'task' ? props.onAlwaysAllow : undefined} onDeny={props.onDeny} onResolvePlan={props.onResolvePlan} onAnswer={props.onAnswer} /></div>}
  </MessageScroller></FreshContext.Provider>
}
