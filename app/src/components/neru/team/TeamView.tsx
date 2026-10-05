import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { isTauri } from '@tauri-apps/api/core'
import { motion, useReducedMotion } from 'motion/react'
import { Activity, ArrowRightLeft, BarChart3, Brush, ChevronRight, CircleAlert, CircleHelp, Copy, Download, ExternalLink, Eye, FileDiff as FileDiffIcon, FileText, FolderOpen, GitBranch, GitFork, GitPullRequest, History, ListFilter, Maximize2, MessageCircleQuestion, Minimize2, MoreHorizontal, Network, PanelRight, Pencil, Pin, Plus, Printer, RefreshCw, Search, Smile, Square, SquareTerminal, Tag, Trash2, Undo2, UserPlus, Users, X } from 'lucide-react'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { MessageScroller } from '@/components/agents/message-scroller'
import { StreamingResponse } from '@/components/agents/streaming-response'
import { SpinnerRing } from '@/components/primitives/TaskRows'
import { EASE_OUT } from '@/lib/ease'
import { cn } from '@/lib/utils'
import { notifyUser } from '@/lib/notify'
import { withMentions } from '@/lib/mentions'
import { api } from '../../../api'
import type { AgentCommand, CustomAgent, ProjectInfo, TeamAgent, TeamArtifact, TeamChanges, TeamEvent, TeamMember, TeamPost, TeamSearchHit, TeamTask, TeamTaskSummary } from '../../../types'
import { Mascot } from '../Mascot'
import { Tour, useTour, type TourStep } from '../Tour'
import { openTerminalTab, type TerminalScope } from '../TerminalPane'
import { ArtifactEditor } from './ArtifactEditor'
import { GridReveal } from '@/components/ui/grid-reveal'
import { markdownToPrintHtml, printDoc } from '@/lib/printDoc'
import { ActivityPanel, AppearanceMenu, ChangesPanel, Checklist, DeleteDialog, FilterMenu, NO_FILTER, PrPanel, QUICKSTARTS, QueueBar, Quickstart, ReplayBar, SetupCard, TaskGlyph, UsageDashboard, applyFilter, filterActive, usageReading, useAgentModels, useDragWidth, useReplay, type TaskFilter } from './TeamPanels'
import { AgentStatus, RESPONSE_PROSE, ResponseMarkdown, ToolSteps, agentPhase, type LiveTool } from '../Conversation'
import { Composer, attachIcons, type VoiceEngine } from '../Composer'
import { Attachments } from '../Attachments'
import claudeLogo from '@/assets/agents/claude.svg?raw'
import codexLogo from '@/assets/agents/codex.svg?raw'
import cursorLogo from '@/assets/agents/cursor.svg?raw'
import geminiLogo from '@/assets/agents/gemini.svg?raw'
import opencodeLogo from '@/assets/agents/opencode.svg?raw'
import vscodeLogo from '@/assets/agents/vscode.svg?raw'
import githubCopilotLogo from '@/assets/agents/githubcopilot.svg?raw'
import qwenLogo from '@/assets/agents/qwen.svg?raw'
import ampLogo from '@/assets/agents/amp.svg?raw'
import gooseLogo from '@/assets/agents/goose.svg?raw'
import kiroLogo from '@/assets/agents/kiro.svg?raw'
import './Team.css'

const errorText = (value: unknown) => value instanceof Error ? value.message : String(value)
const readStored = <T,>(key: string, fallback: T): T => { try { const raw = localStorage.getItem(key); return raw ? JSON.parse(raw) as T : fallback } catch { return fallback } }
const writeStored = (key: string, value: unknown) => { try { localStorage.setItem(key, JSON.stringify(value)) } catch { /* storage unavailable */ } }
const baseName = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path

/** Neru's own agent joins like any other member, on the model set up in Settings. */
const NERU_AGENT: TeamAgent = { kind: 'neru', name: 'Neru', path: 'built in', version: null, signedIn: true, login: '', install: '', resumes: true }
const AGENT_NAMES: Record<string, string> = { claude: 'Claude Code', codex: 'Codex', opencode: 'OpenCode', gemini: 'Gemini CLI', cursor: 'Cursor Agent', neru: 'Neru', copilot: 'GitHub Copilot', vscode: 'VS Code', 'claude-desktop': 'Claude app', shared: 'Shared skills', qwen: 'Qwen Code', amp: 'Amp', droid: 'Factory Droid', goose: 'Goose', crush: 'Crush', aider: 'Aider', auggie: 'Auggie', kiro: 'Kiro CLI', continue: 'Continue CLI' }

/** Headless CLIs cannot stop to ask, so these are what each may do on its own. */
const ACCESS = [
  { mode: 'plan', label: 'Read-only', hint: 'Reads and plans; changes nothing' },
  { mode: 'accept_edits', label: 'Edit files', hint: 'Edits files in the project; risky commands stay blocked by the agent' },
  { mode: 'auto', label: 'Auto', hint: "The agent's own reviewer approves each action (Claude Code's auto mode, Codex's automatic review); other agents may run safe read, build and test commands only" },
  { mode: 'bypass', label: 'Full access', hint: 'Runs anything without asking. Use in a worktree or sandbox' },
]

/** Traycer-style team skills: each writes its output to the task's artifacts so every member can read it.
 * Artifacts start with front matter (kind: spec | ticket | story | review, and a status for tickets). */
const TEAM_COMMANDS: { name: string; description: string; all?: boolean; text: (args: string) => string }[] = [
  { name: 'plan', description: 'Brief, core flows and technical plan, saved as a spec', text: args => `Plan this without changing code yet: ${args || 'the task above'}.\nWrite artifacts/specs/plan.md starting with front matter (kind: spec, title: …), then a short brief, the core user flows and a technical plan (files to touch, risks, open questions). Then ask a teammate to review the plan.` },
  { name: 'phases', description: 'Split the work into phases you can ship one at a time', text: args => `Split ${args || 'the task above'} into 3–6 phases small enough to ship one at a time. Write artifacts/specs/phases.md (front matter kind: spec) with the goal and how to check each phase, then do phase 1 only.` },
  { name: 'tickets', description: 'Break the plan into tickets with dependencies and checks', text: args => `Break artifacts/specs/plan.md into tickets${args ? ` (${args})` : ''}. Write one Markdown file per ticket in artifacts/tickets/, each starting with front matter (kind: ticket, status: todo, title: …), then scope, dependencies and how to verify it. List them in your reply.` },
  { name: 'execute', description: 'Implement the tickets in dependency order and hand off for review', text: args => `Implement the tickets in artifacts/tickets/${args ? ` (${args})` : ''} in dependency order. Set a ticket's front-matter status to in_progress when you start it and done when it passes its checks, with notes on any deviation. When finished, hand the change to a teammate for review.` },
  { name: 'review', description: 'Review the current changes and record findings', text: args => `Review the current changes (git diff)${args ? ` focusing on ${args}` : ''} for correctness, risks and missing tests. Write the findings to artifacts/reviews/review-${new Date().toISOString().slice(0, 10)}.md (front matter kind: review), most severe first.` },
  { name: 'verify', description: 'Check the work against the plan and tickets, by severity', text: args => `Verify the implementation${args ? ` of ${args}` : ''} against artifacts/specs/ and artifacts/tickets/. Write artifacts/reviews/verification-${new Date().toISOString().slice(0, 10)}.md (front matter kind: review) with findings grouped Critical / Major / Minor, each with the fix. Hand the fixes to whoever wrote the code.` },
  { name: 'critique', description: 'Find gaps and contradictions in the specs', text: args => `Critique the specs in artifacts/specs/${args ? ` (${args})` : ''}: gaps, contradictions, untestable requirements and risks. Write artifacts/reviews/critique-${new Date().toISOString().slice(0, 10)}.md (front matter kind: review). Change no code.` },
  { name: 'revise', description: 'Revise the requirements with new information', text: args => `Revise the requirements in artifacts/specs/ with this: ${args || 'what we learned above'}. Update the affected tickets too, and add a short "Changes" section at the end of each file you touch.` },
  { name: 'autobuild', description: 'Spec, rubric, then build-and-evaluate sprints until it passes', text: args => `Autobuild ${args || 'the task above'}: write artifacts/autobuild/spec.md (kind: spec) with a pass/fail rubric, then build in short sprints. After each sprint ask a teammate to evaluate against the rubric and record the result in artifacts/autobuild/sprints.md. Stop when every rubric line passes.` },
  { name: 'debate', description: 'Every member argues a position, then challenges the others', all: true, text: args => `Debate: ${args || 'the best approach for the task above'}.\nState your position with reasons and trade-offs, then challenge the strongest point a teammate made. Record the outcome in artifacts/debates/ (front matter kind: review) with a short synthesis.` },
  { name: 'explain', description: 'A walkthrough for someone new to the code', text: args => `Explain ${args || 'how this project works'} for someone new to the code. Save a walkthrough to artifacts/walkthroughs/ (front matter kind: spec) and summarize it here.` },
  { name: 'walkthrough', description: 'Guide to reviewing the completed change', text: args => `Write a changeset walkthrough${args ? ` for ${args}` : ''}: what changed, why, and the order to review the files in. Save it to artifacts/changeset-walkthroughs/ (front matter kind: spec).` },
  { name: 'housekeeping', description: 'List stale branches and worktrees; change nothing', text: () => `Do housekeeping for this task: list the Git worktrees and branches it created, which ones have landed in the base branch, which have uncommitted work, and what is safe to remove. Change nothing.` },
  { name: 'btw', description: 'Side question to one member; the thread does not move on', text: args => args },
]
const SIDE_COMMANDS = ['btw', 'side']

/** First visit to Team, before any task exists. */
const EMPTY_TOUR: TourStep[] = [
  { title: 'Your agents, one thread', body: <><p>Team puts the coding agents you already pay for — Claude Code, Codex, Cursor, OpenCode, Gemini — in one conversation.</p><p>They read each other’s replies, share files and plans, and hand work to one another. Here’s a one-minute look around.</p></> },
  { target: 'agents', title: 'Found on this machine', body: <><p>Neru looks for each agent’s command-line tool and whether you’re signed in. It never sees your passwords or tokens: signing in happens in the agent’s own login.</p><p>Not installed? <strong>Install</strong> copies the command to run.</p></> },
  { target: '.team-checklist', side: 'right', title: 'Your first steps', body: <p>Four steps to a working team. Each ticks itself off when it happens; hide the list any time.</p> },
  { target: '.team-quickstart', side: 'top', title: 'Start from a template', body: <p>Plan a feature, review your changes, fix a bug together or debate an approach: each sets up the task and sends the first message for you.</p> },
  { target: 'start', title: 'Start a task', body: <><p>Pick two or more agents and what they may do: <strong>Read-only</strong>, <strong>Edit files</strong>, <strong>Auto</strong> (the agent’s own reviewer approves each action) or <strong>Full access</strong>. They work in the project you have open.</p></> },
  { target: 'import', title: 'Bring your history', body: <><p>Import past chats from Claude Code, Codex, Cursor, VS Code Copilot, Gemini and OpenCode. Each becomes a task the agent can carry on from where it left off.</p><p>Skills, MCP servers and rules come across too.</p></> },
  { target: 'rail', side: 'right', title: 'All your tasks', body: <p>Tasks live here, in groups with their own icon and colour. A dot marks new messages, and a red mark a member that failed. Pin, label and search every message across every task.</p> },
]

/** First time a task is open: the thread, the composer and the task panel. */
function taskTourSteps(openPanel: (tab: PanelTab) => void, handle: string): TourStep[] {
  return [
    { target: 'roster', side: 'bottom', title: 'Who’s on the task', body: <p>Each member keeps its own session and memory, and sees what everyone else said since its last turn. Click a chip to send your next message only to that agent.</p> },
    { target: 'thread', title: 'One shared thread', body: <><p>Replies, tool steps and hand-offs land here. When an agent writes <code>@{handle} …</code> on its own line, that teammate takes the next turn.</p><p>Turns that change files get a card with the files and an <strong>Undo</strong> button.</p></> },
    { target: 'composer', side: 'top', title: 'Talk to the team', body: <ul><li><kbd>@</kbd> mentions a teammate, or <code>@all</code> for everyone at once</li><li><kbd>/</kbd> opens team skills (<code>/plan</code>, <code>/tickets</code>, <code>/review</code>…) and each agent’s own commands</li><li><code>/btw @{handle} …</code> asks on the side; <code>/model @{handle} …</code> switches its model</li><li>Messages to a busy member wait in a queue you can edit, reorder or pause</li></ul> },
    { target: 'to', side: 'top', title: 'Choose who answers', body: <p><strong>Auto</strong> sends to whoever you mention, otherwise to whoever spoke last. <strong>All</strong> asks every member in parallel.</p> },
    { target: 'panel', side: 'left', enter: () => openPanel('members'), title: 'Members', body: <><p>Per agent: the model (pick from its list), what it may do, and its own Git worktree so parallel edits never collide.</p><p>Fork a member, open its own CLI in Neru’s terminal, or show only its transcript.</p></> },
    { target: 'panel', side: 'left', enter: () => openPanel('artifacts'), title: 'Artifacts', body: <p>Specs, tickets and reviews the team writes. Edit them with a formatting toolbar and live preview, export as Markdown or PDF, and restore any earlier version.</p> },
    { target: 'panel', side: 'left', enter: () => openPanel('changes'), title: 'Every change, together', body: <p>All the files the task changed, across the project and every member’s worktree, as one diff in unified or split view. Ask a member to review it all.</p> },
    { target: 'panel', side: 'left', enter: () => openPanel('pr'), title: 'Pull request', body: <p>The PR for the project’s branch or a member’s, with its checks. When one fails, <strong>Fix</strong> hands the failing log to a member.</p> },
    { target: 'panel', side: 'left', enter: () => openPanel('activity'), title: 'Activity and smart execution', body: <p>Who is working and for how long, what is queued, and side chats waiting. <strong>Run the tickets</strong> has one member implement each ticket and another review it, round after round.</p> },
    { target: 'panel', side: 'left', enter: () => openPanel('usage'), title: 'Usage and limits', body: <><p>Tokens and cost per turn, and Claude Code’s and Codex’s 5-hour and weekly windows. Export it all as CSV. With <strong>Route on limits</strong> on, a member that runs out hands the request to a teammate.</p></> },
    { target: 'panel', side: 'left', enter: () => openPanel('map'), title: 'Map and replay', body: <p>Who handed work to whom. Press play to replay the hand-offs in order, or click a line to see its messages.</p> },
    { target: 'reading', side: 'bottom', title: 'Limits at a glance', body: <p>The most-used subscription window on the team. Click it, or press <kbd>Ctrl</kbd> <kbd>Shift</kbd> <kbd>U</kbd>, for every member’s usage.</p> },
    { target: 'options', side: 'bottom', title: 'Task options', body: <p>Pin the task, label it, give it a group, icon and colour, sweep worktrees whose work has landed, or delete it. Deleting shows what each worktree would lose first.</p> },
    { target: 'filter', side: 'right', title: 'Filter and sort', body: <p>Narrow the list by repository, agent, label or unread, matching all or any of them, and sort by activity, date or title.</p> },
    { target: 'search', side: 'right', title: 'Find anything', body: <p>Searches task names, labels and every message. Click a result to jump straight to it.</p> },
    { target: 'help', side: 'bottom', title: 'That’s the tour', body: <p>Replay it from here any time. Start with something small, like <code>/plan</code> and a sentence about what you want.</p> },
  ]
}

function relativeTime(ms: number) {
  const minutes = Math.floor((Date.now() - ms) / 60000)
  if (minutes < 1) return 'now'
  if (minutes < 60) return `${minutes}m`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours}h`
  return new Date(ms).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })
}

/** Markdown without its front matter (kind, status, title), which the panel shows as badges. */
const withoutFront = (text: string) => text.replace(/^---\r?\n[\s\S]*?\r?\n---\r?\n?/, '')


/** Each product's own logo; see assets/agents/NOTICE.md for where they come from. */
const LOGOS: Record<string, string> = { claude: claudeLogo, 'claude-desktop': claudeLogo, codex: codexLogo, opencode: opencodeLogo, gemini: geminiLogo, cursor: cursorLogo, copilot: githubCopilotLogo, vscode: vscodeLogo, qwen: qwenLogo, amp: ampLogo, goose: gooseLogo, kiro: kiroLogo }

const agentName = (kind: string) => AGENT_NAMES[kind] ?? kind.replace(/^custom:/, '')

export function AgentMark({ kind, size = 28, status }: { kind: string; size?: number; status?: string }) {
  const logo = LOGOS[kind]
  return <span className={cn('agent-mark', `agent-${kind}`, status && `is-${status}`)} style={{ width: size, height: size }} title={agentName(kind)} aria-hidden>
    {kind === 'neru' ? <Mascot size={size} />
      : !logo && kind.startsWith('custom:') ? <SquareTerminal size={size * 0.55} strokeWidth={1.75} />
      // No published mark for this CLI: its initial on the same tile.
      : !logo ? <b className="agent-initial" style={{ fontSize: size * 0.46 }}>{agentName(kind).charAt(0)}</b>
      // One-colour logos are inlined to take the tile's ink; colour ones stay images so their gradient ids never clash.
      : logo.includes('currentColor') ? <span className="agent-logo" dangerouslySetInnerHTML={{ __html: logo }} />
      : <img className="agent-logo" src={`data:image/svg+xml;utf8,${encodeURIComponent(logo)}`} alt="" draggable={false} />}
  </span>
}

function stepsAsTools(steps: string[], live: boolean): LiveTool[] {
  return steps.map((label, index) => ({ id: String(index), label, status: live && index === steps.length - 1 ? 'running' as const : 'done' as const }))
}

function ChangesCard({ changes, onUndo }: { changes: TeamChanges; onUndo: () => void }) {
  const [open, setOpen] = useState(false)
  const shown = open ? changes.files : changes.files.slice(0, 4)
  return <div className={cn('team-changes', changes.undone && 'undone')}>
    <div className="team-changes-head">
      <strong>Changes</strong><span>{changes.files.length} {changes.files.length === 1 ? 'file' : 'files'}{changes.undone ? ' · undone' : ''}</span>
      {!changes.undone && <button className="button subtle small" onClick={() => { if (window.confirm('Undo this turn? Its files go back to how they were before it, including later edits to them.')) onUndo() }}><Undo2 size={13} />Undo</button>}
    </div>
    <ul>{shown.map(file => <li key={file.path}><b className={`git-${file.status}`}>{file.status}</b><code className="truncate" title={file.path}>{file.path}</code></li>)}</ul>
    {changes.files.length > 4 && <button className="team-changes-more" onClick={() => setOpen(value => !value)}>{open ? 'Show fewer' : `Show all ${changes.files.length}`}</button>}
  </div>
}

function PostView({ taskId, post, member, handles, onOpenSettings, onUndo, onRerun }: { taskId: string; post: TeamPost; member?: TeamMember; handles: string[]; onOpenSettings: () => void; onUndo: (postId: string) => void; onRerun: (handle: string) => void }) {
  if (post.kind === 'setup') return <SetupCard post={post} onRerun={() => onRerun(post.author)} />
  if (post.kind === 'notice') return <div className="team-notice" role="status"><ResponseMarkdown content={post.text} sources={[]} idPrefix={`notice-${post.id.slice(0, 8)}`} onCite={() => undefined} /></div>
  if (post.author === 'you') return <article className={cn('message user team-user', post.kind === 'side' && 'is-side')} data-from="user">
    <div className="team-post-meta">{post.kind === 'side' && <span className="team-side-tag"><MessageCircleQuestion size={12} />Side chat</span>}{post.to.length > 0 && <span className="team-to">to {post.to.map(handle => `@${handle}`).join(', ')}</span>}<time>{relativeTime(post.at)}</time></div>
    <div className="message-body">{withMentions(post.text, handles)}</div>
  </article>
  const kind = member?.kind ?? (post.author === 'copilot' ? 'copilot' : 'neru')
  return <article className={cn('team-post', post.kind === 'error' && 'is-error', post.kind === 'side' && 'is-side')} data-from="agent">
    <AgentMark kind={kind} size={30} />
    <div className="team-post-body">
      <header className="team-post-meta"><strong>@{post.author}</strong><span>{agentName(kind)}</span><time>{relativeTime(post.at)}</time>
        {post.kind === 'side' && <span className="team-side-tag"><MessageCircleQuestion size={12} />Side chat</span>}
        {post.to.length > 0 && <span className="team-handoff"><ArrowRightLeft size={12} />{post.to.map(handle => `@${handle}`).join(', ')}</span>}</header>
      {post.kind === 'error'
        ? <div className="team-error"><CircleAlert size={15} /><span>{post.text}</span>{/not signed in|Settings → Agents/i.test(post.text) && <button className="button subtle small" onClick={onOpenSettings}>Agents settings</button>}</div>
        : <>
          <ToolSteps tools={stepsAsTools(post.steps, false)} live={false} />
          <StreamingResponse status="complete" copyText={post.text} announce={false} contentClassName={RESPONSE_PROSE}>
            <ResponseMarkdown content={post.text} sources={[]} idPrefix={`team-${post.id.slice(0, 8)}`} onCite={() => undefined} mentions={handles} />
          </StreamingResponse>
          {post.images?.map(name => <PostImage key={name} taskId={taskId} name={name} />)}
          {post.changes && <ChangesCard changes={post.changes} onUndo={() => onUndo(post.id)} />}
        </>}
    </div>
  </article>
}

/** A member at work, drawn like a reply in Code mode: its steps, its reply as it streams, and the
 * thinking orb with what it is doing now. */
/** A generated image: Grid Reveal while it loads, then the picture, which opens at full size. */
function PostImage({ taskId, name }: { taskId: string; name: string }) {
  const [src, setSrc] = useState<string | null>(null)
  useEffect(() => { let live = true; void api.teamImage(taskId, name).then(uri => live && setSrc(uri)).catch(() => {}); return () => { live = false } }, [taskId, name])
  return <a className="team-image" href={src ?? undefined} target="_blank" rel="noreferrer" onClick={event => { if (!src) event.preventDefault() }}>
    <GridReveal src={src} alt="Generated image" caption="Loading image" estimatedDuration={3000} />
  </a>
}

function LivePost({ member, live, handles }: { member: TeamMember; live?: { text: string; steps: string[]; drawing?: boolean }; handles: string[] }) {
  const tools = stepsAsTools(live?.steps ?? [], !live?.text)
  const phase = agentPhase({ text: live?.text ?? '', tools, sources: [], drafts: [], reasoning: 0 }, member.mode === 'plan' ? 'plan' : 'code')
  return <article className="team-post is-live" data-from="agent" aria-busy>
    <AgentMark kind={member.kind} size={30} status="working" />
    <div className="team-post-body">
      <header className="team-post-meta"><strong>@{member.handle}</strong><span>{agentName(member.kind)}</span></header>
      <ToolSteps tools={tools} live />
      {live?.text && <StreamingResponse status="streaming" copyText={live.text} announce={false} showActions={false} contentClassName={RESPONSE_PROSE}>
        <ResponseMarkdown content={live.text} sources={[]} idPrefix={`live-${member.handle}`} onCite={() => undefined} mentions={handles} />
      </StreamingResponse>}
      {live?.drawing && <div className="team-image"><GridReveal src={null} caption="Creating image" estimatedDuration={45000} /></div>}
      <AgentStatus phase={phase} />
    </div>
  </article>
}

function RoutingCard({ routing, onCancel }: { routing: { from: string; to: string; until: number }; onCancel: () => void }) {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => { const timer = window.setInterval(() => setNow(Date.now()), 250); return () => window.clearInterval(timer) }, [])
  const left = Math.max(0, Math.ceil((routing.until - now) / 1000))
  return <div className="team-routing" role="status">
    <ArrowRightLeft size={16} />
    <div><strong>@{routing.from} hit its limit</strong><span>Handing the request to @{routing.to} in {left}s, with the whole thread.</span></div>
    <button className="button subtle small" onClick={onCancel}>Keep it here</button>
  </div>
}

/** Where a new task works: the open project, a recent one, another folder, a new project or none. */
type ProjectChoice = { path: string; create: boolean }

function NewTaskDialog({ agents, project, preset, onClose, onCreate }: { agents: TeamAgent[]; project: ProjectInfo | null; preset?: string; onClose: () => void; onCreate: (title: string, kinds: string[], mode: string, where: ProjectChoice, quickstart?: string) => void }) {
  const ready = agents.filter(agent => agent.path)
  const [recent, setRecent] = useState<string[]>([])
  useEffect(() => { if (isTauri()) void api.recentProjects().then(setRecent).catch(() => undefined) }, [])
  const current = project?.path.replace(/^\\\\\?\\/, '') ?? ''
  const [where, setWhere] = useState<'current' | 'pick' | 'new' | 'none' | string>(current ? 'current' : 'none')
  const [picked, setPicked] = useState('')
  const [newName, setNewName] = useState('')
  const [newParent, setNewParent] = useState(() => current ? current.replace(/[\\/][^\\/]+$/, '') : '')
  const others = recent.map(path => path.replace(/^\\\\\?\\/, '')).filter(path => path && path !== current).slice(0, 8)
  const sep = newParent.includes('/') && !newParent.includes('\\') ? '/' : '\\'
  const choice: ProjectChoice | null = where === 'current' ? { path: current, create: false }
    : where === 'none' ? { path: '', create: false }
    : where === 'pick' ? (picked ? { path: picked, create: false } : null)
    : where === 'new' ? (newName.trim() && newParent ? { path: `${newParent.replace(/[\\/]+$/, '')}${sep}${newName.trim()}`, create: true } : null)
    : { path: where, create: false }
  const browse = async (forNew: boolean) => {
    const folder = await openDialog({ directory: true, multiple: false, title: forNew ? 'Where the new project goes' : 'Choose the project folder' }).catch(() => null)
    if (typeof folder !== 'string') return
    if (forNew) setNewParent(folder); else { setPicked(folder); setWhere('pick') }
  }
  const [kinds, setKinds] = useState<string[]>(() => ready.filter(agent => agent.signedIn).slice(0, 2).map(agent => agent.kind))
  const [title, setTitle] = useState('')
  const [mode, setMode] = useState(preset === 'review' ? 'plan' : 'accept_edits')
  const quick = QUICKSTARTS.find(item => item.id === preset)
  const toggle = (kind: string) => setKinds(current => current.includes(kind) ? current.filter(item => item !== kind) : [...current, kind])
  return <div className="modal-backdrop" onMouseDown={onClose}>
    <form className="clone-dialog team-dialog" onMouseDown={event => event.stopPropagation()} onSubmit={event => { event.preventDefault(); if (choice) onCreate(title || quick?.title || '', kinds, mode, choice, preset) }}>
      <h2>{quick ? `${quick.title}.` : 'Start a team task.'}</h2>
      <p>{quick ? quick.text : 'The agents share one thread and work in the project you choose.'}</p>
      <label>{preset === 'review' ? 'Anything to focus on? (optional)' : preset === 'bug' ? 'What is the bug?' : preset === 'debate' ? 'What should they debate?' : 'What are you working on?'}<input autoFocus value={title} onChange={event => setTitle(event.target.value)} placeholder={preset === 'bug' ? 'Login fails after the session expires' : preset === 'debate' ? 'REST or GraphQL for the new API' : 'Add rate limiting to the API'} /></label>
      <label>Project
        <select value={where === 'pick' ? 'pick' : where} onChange={event => { const value = event.target.value; if (value === 'browse') void browse(false); else setWhere(value) }}>
          {current && <option value="current">{baseName(current)} (open now)</option>}
          {others.map(path => <option key={path} value={path}>{baseName(path)} · {path}</option>)}
          {picked && <option value="pick">{baseName(picked)} · {picked}</option>}
          <option value="browse">Choose another folder…</option>
          <option value="new">New project…</option>
          <option value="none">No project (planning only)</option>
        </select>
      </label>
      {where === 'new' && <div className="team-new-project">
        <label>Project name<input value={newName} onChange={event => setNewName(event.target.value)} placeholder="port-website" /></label>
        <label>Inside<span className="path-field"><input value={newParent} onChange={event => setNewParent(event.target.value)} placeholder="D:\\Projects" /><button type="button" className="icon-button" onClick={() => void browse(true)} aria-label="Choose the parent folder" title="Choose folder"><FolderOpen size={16} /></button></span></label>
        {choice && <p className="settings-note">Neru creates <code>{choice.path}</code> and starts a Git repository there.</p>}
      </div>}
      <fieldset className="team-pick"><legend>Agents</legend>
        {ready.map(agent => <button type="button" key={agent.kind} className={cn('team-pick-item', kinds.includes(agent.kind) && 'on')} aria-pressed={kinds.includes(agent.kind)} onClick={() => toggle(agent.kind)}>
          <AgentMark kind={agent.kind} size={26} /><span><strong>{agent.name}</strong><small>{agent.kind === 'neru' ? 'Your Neru model' : agent.signedIn ? 'Signed in' : 'Not signed in'}</small></span>
        </button>)}
      </fieldset>
      {ready.length <= 1 && <p className="settings-note">No agent CLIs found. Install Claude Code, Codex, OpenCode, Gemini CLI or Cursor Agent, then refresh.</p>}
      <label>What they may do<div className="theme-toggle team-access">{ACCESS.map(item => <button type="button" key={item.mode} className={mode === item.mode ? 'active' : ''} title={item.hint} onClick={() => setMode(item.mode)}>{item.label}</button>)}</div></label>
      <div className="clone-actions"><button type="button" className="button subtle" onClick={onClose}>Cancel</button><button type="submit" className="button primary" disabled={kinds.length === 0 || !choice || (preset === 'bug' && !title.trim())}>Start task</button></div>
    </form>
  </div>
}

const EFFORT_LABELS: Record<string, string> = { minimal: 'Minimal', low: 'Low', medium: 'Medium', high: 'High', xhigh: 'Extra high', max: 'Max', ultra: 'Ultra' }

/** The member's model and reasoning effort: dropdowns of what its CLI offers, plus your own id. */
function ModelField({ member, onChange }: { task: TeamTask; member: TeamMember; onChange: (change: { model?: string; effort?: string }) => void }) {
  const models = useAgentModels(member.kind)
  const [typing, setTyping] = useState(false)
  const chosen = models.find(model => model.id === member.model)
  // Efforts: the chosen model's, or what the CLI takes for any model when on its default.
  const efforts = chosen?.efforts.length ? chosen.efforts : [...new Set(models.flatMap(model => model.efforts))]
  const effort = member.effort ?? ''
  // No list (Neru's own agent, a custom agent, or a CLI that keeps none): type the model id.
  if (typing || models.length === 0) return <label className="team-field">Model<input autoFocus={typing} defaultValue={member.model} key={member.model} placeholder={member.kind === 'neru' ? 'Neru’s model' : 'Agent default'} onBlur={event => { setTyping(false); if (event.target.value.trim() !== member.model) onChange({ model: event.target.value.trim() }) }} onKeyDown={event => { if (event.key === 'Enter') event.currentTarget.blur(); if (event.key === 'Escape') setTyping(false) }} /></label>
  return <div className="team-model-row">
    <label className="team-field">Model
      <select value={member.model} onChange={event => { if (event.target.value === '__other__') { setTyping(true); return } const next = models.find(model => model.id === event.target.value); onChange({ model: event.target.value, ...(effort && next?.efforts.length && !next.efforts.includes(effort) ? { effort: '' } : {}) }) }}>
        <option value="">Agent default</option>
        {models.map(model => <option key={model.id} value={model.id}>{model.label === model.id ? model.id : `${model.label} · ${model.id}`}</option>)}
        {member.model && !chosen && <option value={member.model}>{member.model}</option>}
        <option value={'__other__'}>Another model…</option>
      </select></label>
    {efforts.length > 0 && <label className="team-field">Effort
      <select value={effort} onChange={event => onChange({ effort: event.target.value })}>
        <option value="">{chosen?.defaultEffort ? `Default (${EFFORT_LABELS[chosen.defaultEffort] ?? chosen.defaultEffort})` : 'Default'}</option>
        {efforts.map(level => <option key={level} value={level}>{EFFORT_LABELS[level] ?? level}</option>)}
      </select></label>}
  </div>
}

function MembersPanel({ task, agents, custom, only, onOnly, onChange, onError, onTerminal }: { task: TeamTask; agents: TeamAgent[]; custom: CustomAgent[]; only: string | null; onOnly: (handle: string | null) => void; onChange: (task: TeamTask) => void; onError: (message: string) => void; onTerminal: (handle: string) => void }) {
  const [adding, setAdding] = useState(false)
  const act = (work: Promise<TeamTask>) => void work.then(onChange).catch(cause => onError(errorText(cause)))
  const cli = (kind: string) => !kind.startsWith('custom:') && kind !== 'neru'
  return <div className="team-panel-body">
    {task.members.map(member => <div className="team-member" key={member.handle}>
      <div className="team-member-head">
        <AgentMark kind={member.kind} size={30} status={member.status} />
        <div><strong>@{member.handle}</strong><span>{agentName(member.kind)}{member.forkNext ? ' · fork' : member.upstream ? ' · session kept' : ''}</span></div>
        <span className={cn('team-status', `is-${member.status || 'idle'}`)}>{member.status === 'working' ? 'Working' : member.status === 'failed' ? 'Failed' : member.status === 'stopped' ? 'Stopped' : 'Ready'}</span>
        {member.status === 'working'
          ? <button className="icon-button small" onClick={() => void api.stopTeam(task.id, member.handle).catch(cause => onError(errorText(cause)))} aria-label={`Stop @${member.handle}`} title="Stop"><Square size={13} /></button>
          : <button className="icon-button small" onClick={() => act(api.removeTeamMember(task.id, member.handle))} aria-label={`Remove @${member.handle}`} title="Remove from task"><Trash2 size={13} /></button>}
      </div>
      <div className="team-member-actions">
        {cli(member.kind) && <button className="icon-button small" onClick={() => onTerminal(member.handle)} title="Open its own CLI in Neru's terminal, in the same session" aria-label={`Open @${member.handle} in Neru's terminal`}><SquareTerminal size={14} /></button>}
        {cli(member.kind) && <button className="icon-button small" onClick={() => void api.teamOpenTerminal(task.id, member.handle).catch(cause => onError(errorText(cause)))} title="Open its own CLI in a separate terminal window" aria-label={`Open @${member.handle} in a terminal window`}><ExternalLink size={13} /></button>}
        <button className="icon-button small" onClick={() => act(api.forkTeamMember(task.id, member.handle))} title="Fork: a copy that continues from this member's session" aria-label={`Fork @${member.handle}`}><GitFork size={14} /></button>
        <button className={cn('icon-button small', only === member.handle && 'active')} onClick={() => onOnly(only === member.handle ? null : member.handle)} aria-pressed={only === member.handle} title="Transcript: show only this member's posts" aria-label={`Show only @${member.handle}'s posts`}><Eye size={14} /></button>
      </div>
      {member.error && <p className="team-member-error">{member.error}</p>}
      <ModelField task={task} member={member} onChange={change => act(api.updateTeamMember(task.id, member.handle, change))} />
      {task.projectPath && <div className="team-worktree">
        <GitBranch size={13} /><span className="truncate" title={member.worktree?.path}>{member.worktree ? member.worktree.branch : 'Works in the project folder'}</span>
        <button className={cn('switch', member.worktree && 'on')} role="switch" aria-checked={Boolean(member.worktree)} aria-label={`Own worktree for @${member.handle}`} title="Own Git worktree, so parallel edits do not collide" disabled={member.status === 'working'} onClick={() => act(api.setTeamWorktree(task.id, member.handle, !member.worktree))} />
      </div>}
      <div className="theme-toggle team-access" role="group" aria-label={`What @${member.handle} may do`}>{ACCESS.map(item => <button key={item.mode} className={member.mode === item.mode ? 'active' : ''} title={item.hint} onClick={() => act(api.updateTeamMember(task.id, member.handle, { mode: item.mode }))}>{item.label}</button>)}</div>
    </div>)}
    <div className="menu-anchor">
      <button className="button subtle team-add" onClick={() => setAdding(value => !value)} aria-expanded={adding}><UserPlus size={15} />Add agent</button>
      {adding && <div className="menu menu-down" role="menu">
        {agents.filter(agent => agent.path).map(agent => <button role="menuitem" key={agent.kind} onClick={() => { setAdding(false); act(api.addTeamMember(task.id, { kind: agent.kind, mode: 'accept_edits' })) }}><AgentMark kind={agent.kind} size={18} />{agent.name}{!agent.signedIn && <small>not signed in</small>}</button>)}
        {custom.length > 0 && <div className="menu-separator" />}
        {custom.map(agent => <button role="menuitem" key={agent.kind} title={agent.path} onClick={() => { setAdding(false); act(api.addTeamMember(task.id, { kind: agent.kind, mode: 'accept_edits' })) }}><AgentMark kind={agent.kind} size={18} />{agent.name}<small>your script</small></button>)}
      </div>}
    </div>
    <div className="settings-row team-route"><div><strong>Route on limits</strong><p>When a subscription runs out, hand the request to a teammate.</p></div>
      <button className={cn('switch', task.routeOnLimit && 'on')} role="switch" aria-checked={task.routeOnLimit} aria-label="Route on limits" onClick={() => act(api.setTeamRouting(task.id, !task.routeOnLimit))} /></div>
  </div>
}

function ArtifactsPanel({ task, onError }: { task: TeamTask; onError: (message: string) => void }) {
  const [items, setItems] = useState<TeamArtifact[]>([])
  const [open, setOpen] = useState<string | null>(null)
  const [text, setText] = useState('')
  const [draft, setDraft] = useState<string | null>(null)
  const [version, setVersion] = useState<string | null>(null)
  const viewRef = useRef<HTMLDivElement>(null)
  const refresh = useCallback(() => void api.listTeamArtifacts(task.id).then(setItems).catch(cause => onError(errorText(cause))), [task.id, onError])
  useEffect(() => { refresh() }, [refresh, task.posts.length])
  const show = (path: string) => { setOpen(path); setDraft(null); setVersion(null); void api.readTeamArtifact(task.id, path).then(setText).catch(cause => onError(errorText(cause))) }
  const save = (content: string) => void api.writeTeamArtifact(task.id, open!, content).then(list => { setItems(list); setText(content); setDraft(null); setVersion(null) }).catch(cause => onError(errorText(cause)))
  const [kind, setKind] = useState('all')
  const current = items.find(item => item.path === open)
  const cycle = (item: TeamArtifact) => {
    const next = item.status === 'todo' ? 'in_progress' : item.status === 'in_progress' ? 'done' : 'todo'
    void api.setTeamArtifactStatus(task.id, item.path, next).then(setItems).catch(cause => onError(errorText(cause)))
  }
  const exportAll = async () => {
    const destination = await openDialog({ directory: true, title: 'Export artifacts to…' })
    if (typeof destination !== 'string') return
    try { const count = await api.exportTeamArtifacts(task.id, shownItems.map(item => item.path), destination); window.alert(`Exported ${count} ${count === 1 ? 'artifact' : 'artifacts'} as Markdown.`) } catch (cause) { onError(errorText(cause)) }
  }
  const exportPdf = () => { if (viewRef.current) void printDoc(current?.title ?? baseName(open ?? 'artifact'), markdownToPrintHtml(viewRef.current)).catch(cause => onError(errorText(cause))) }
  const shownItems = items.filter(item => kind === 'all' || (item.kind ?? 'other') === kind)
  const kinds = [...new Set(items.map(item => item.kind ?? 'other'))]
  if (open) return <div className="team-panel-body team-artifact">
    <div className="team-artifact-bar">
      <button className="icon-button small" onClick={() => { if (draft !== null && draft !== text && !window.confirm('Discard your edits?')) return; setOpen(null) }} aria-label="Back to artifacts" title="Back"><ChevronRight size={14} style={{ transform: 'rotate(180deg)' }} /></button>
      <span className="truncate" title={open}>{open}</span>
      {draft === null && <>
        <button className="icon-button small" onClick={exportPdf} aria-label="Export as PDF" title="Export as PDF (print, then Save as PDF)"><Printer size={13} /></button>
        <button className="icon-button small" onClick={() => setDraft(text)} aria-label="Edit" title="Edit"><Pencil size={13} /></button>
      </>}
    </div>
    {current && current.versions.length > 0 && draft === null && <div className="team-versions"><History size={13} /><select value={version ?? ''} onChange={event => { const value = event.target.value || null; setVersion(value); void api.readTeamArtifact(task.id, value ?? open).then(setText).catch(cause => onError(errorText(cause))) }}>
      <option value="">Current</option>
      {current.versions.map(path => <option key={path} value={path}>{new Date(Number(path.split('.').pop())).toLocaleString()}</option>)}
    </select>{version && <button className="button subtle small" onClick={() => save(text)}>Restore</button>}</div>}
    {draft !== null
      ? <ArtifactEditor key={open} value={draft} onChange={setDraft} onSave={() => save(draft)} onCancel={() => setDraft(null)}
        renderPreview={markdown => <div className={RESPONSE_PROSE}><ResponseMarkdown content={markdown} sources={[]} idPrefix="artifact-edit" onCite={() => undefined} /></div>} />
      : <>
        {current && (current.kind || current.status) && <div className="team-artifact-meta">{current.kind && <span className={`team-kind kind-${current.kind}`}>{current.kind}</span>}{current.status && <button className={`team-status-pill status-${current.status}`} onClick={() => cycle(current)} title="Change status">{current.status === 'in_progress' ? 'In progress' : current.status === 'done' ? 'Done' : 'To do'}</button>}</div>}
        <div ref={viewRef} className={cn('team-artifact-view', RESPONSE_PROSE)}><ResponseMarkdown content={withoutFront(text)} sources={[]} idPrefix="artifact" onCite={() => undefined} /></div>
      </>}
  </div>
  return <div className="team-panel-body">
    <div className="team-panel-tools"><span>{items.length} {items.length === 1 ? 'document' : 'documents'}</span>
      <button className="icon-button small" onClick={() => { const name = window.prompt('New artifact (e.g. specs/brief.md)'); if (name?.trim()) void api.writeTeamArtifact(task.id, name.trim(), `# ${baseName(name.trim()).replace(/\.md$/, '')}\n`).then(list => { setItems(list); show(name.trim()) }).catch(cause => onError(errorText(cause))) }} aria-label="New artifact" title="New artifact"><Plus size={14} /></button>
      <button className="icon-button small" onClick={refresh} aria-label="Refresh" title="Refresh"><RefreshCw size={13} /></button>
      <button className="icon-button small" onClick={() => void exportAll()} disabled={shownItems.length === 0} aria-label="Export as Markdown" title="Export these as Markdown files"><Download size={14} /></button>
      <button className="icon-button small" onClick={() => void api.revealPath(`${task.folder}/artifacts`).catch(cause => onError(errorText(cause)))} aria-label="Open folder" title="Open the artifacts folder"><FolderOpen size={14} /></button></div>
    {items.length === 0 && <p className="empty-small">Specs, tickets and reviews the team writes appear here. Try <code>/plan</code>.</p>}
    {kinds.length > 1 && <div className="team-artifact-kinds">{['all', ...kinds].map(name => <button key={name} className={cn('team-to-chip', kind === name && 'on')} onClick={() => setKind(name)}>{name === 'all' ? 'All' : name === 'other' ? 'Other' : `${name[0].toUpperCase()}${name.slice(1)}s`}</button>)}</div>}
    {shownItems.map(item => <div key={item.path} className="team-artifact-row">
      <button className="team-artifact-open" onClick={() => show(item.path)}><FileText size={14} /><span className="truncate" title={item.path}>{item.title ?? item.path}</span></button>
      {item.kind && <span className={`team-kind kind-${item.kind}`}>{item.kind}</span>}
      {item.status && <button className={`team-status-pill status-${item.status}`} onClick={() => cycle(item)} title="Change status">{item.status === 'in_progress' ? 'In progress' : item.status === 'done' ? 'Done' : 'To do'}</button>}
      <time>{relativeTime(item.modified)}</time>
    </div>)}
  </div>
}

/** Who handed work to whom: members on a ring around you, edges weighted by hand-offs, and a
 * replay that walks the hand-offs in order. */
function MapPanel({ task, onJump }: { task: TeamTask; onJump: (postId: string) => void }) {
  const [selected, setSelected] = useState<string | null>(null)
  const nodes = useMemo(() => {
    const radius = 92
    return [{ id: 'you', kind: 'you', x: 130, y: 130 }, ...task.members.map((member, index) => {
      const angle = (index / Math.max(1, task.members.length)) * Math.PI * 2 - Math.PI / 2
      return { id: member.handle, kind: member.kind, x: 130 + Math.cos(angle) * radius, y: 130 + Math.sin(angle) * radius }
    })]
  }, [task.members])
  const handoffs = useMemo(() => task.posts.filter(post => post.kind === 'message' && post.to.length > 0 && nodes.some(node => node.id === post.author)), [task.posts, nodes])
  const edges = useMemo(() => {
    const map = new Map<string, TeamPost[]>()
    for (const post of handoffs) for (const to of post.to) {
      if (!nodes.some(node => node.id === to)) continue
      const key = `${post.author}>${to}`
      map.set(key, [...(map.get(key) ?? []), post])
    }
    return [...map.entries()]
  }, [handoffs, nodes])
  const replay = useReplay(handoffs, post => onJump(post.id))
  const playing = replay.index >= 0 ? handoffs[replay.index] : null
  const at = (id: string) => nodes.find(node => node.id === id)!
  const chosen = edges.find(([key]) => key === selected)
  const lit = (key: string) => selected === key || (playing ? playing.to.some(to => key === `${playing.author}>${to}`) : false)
  return <div className="team-panel-body">
    <svg viewBox="0 0 260 260" className="team-map" role="img" aria-label="Hand-offs between members">
      {edges.map(([key, posts]) => { const [from, to] = key.split('>'); const a = at(from), b = at(to); return <line key={key} x1={a.x} y1={a.y} x2={b.x} y2={b.y} className={cn('team-map-edge', lit(key) && 'on')} strokeWidth={Math.min(6, 1 + posts.length)} onClick={() => setSelected(key)}><title>{`${from} → ${to}: ${posts.length}`}</title></line> })}
      {nodes.map(node => <g key={node.id} transform={`translate(${node.x} ${node.y})`} className={cn('team-map-node', `agent-${node.kind}`, playing?.author === node.id && 'speaking')}>
        {node.id === 'you'
          ? <><circle r={20} /><text dy="4" textAnchor="middle">You</text></>
          : <foreignObject x={-16} y={-16} width={32} height={32}><AgentMark kind={node.kind} size={32} /></foreignObject>}
        {node.id !== 'you' && <text className="team-map-label" dy="34" textAnchor="middle">@{node.id}</text>}
      </g>)}
    </svg>
    <ReplayBar replay={replay} total={handoffs.length} />
    {playing && <p className="team-map-caption"><strong>@{playing.author} → {playing.to.map(handle => `@${handle}`).join(', ')}</strong> {playing.text.slice(0, 160)}</p>}
    {edges.length === 0 && <p className="empty-small">Hand-offs show up as lines once members pass work to each other.</p>}
    {chosen && <div className="team-map-replay"><strong>{chosen[0].replace('>', ' → @').replace(/^/, '@')}</strong>
      {chosen[1].map(post => <button key={post.id} className="team-artifact-row" onClick={() => onJump(post.id)}><span className="truncate">{post.text.slice(0, 120)}</span><time>{relativeTime(post.at)}</time></button>)}</div>}
  </div>
}

let composerSink: ((text: string) => void) | null = null
/** Puts text in the open Team task's message box (browser feedback while in Team). False when no task is open. */
export function sendToTeamComposer(text: string) {
  if (!composerSink || !text.trim()) return false
  composerSink(text)
  return true
}

export interface TeamViewProps {
  project: ProjectInfo | null
  onError: (message: string) => void
  onOpenSettings: (tab: 'agents' | 'imports') => void
  /** The open task's terminal scope, for the terminal pane; null when no task is open. */
  onScope?: (scope: TerminalScope | null) => void
  onOpenPane?: (id: 'terminal' | 'files' | 'changes' | 'browser') => void
  /** Dictation, as in Code mode's message box. */
  voice: { light: boolean; engine: VoiceEngine; ready: boolean; onStart: () => void; onUnavailable: () => void; onTranscribe: (audio: Blob) => Promise<string> }
}

type PanelTab = 'members' | 'artifacts' | 'changes' | 'pr' | 'activity' | 'usage' | 'map'
const PANEL_TABS = [['members', 'Members', Users], ['artifacts', 'Artifacts', FileText], ['changes', 'Changes', FileDiffIcon], ['pr', 'Pull request', GitPullRequest], ['activity', 'Activity', Activity], ['usage', 'Usage', BarChart3], ['map', 'Map', Network]] as const

/** $ARGUMENTS, {{args}} and $1…$9 in an agent's own command, filled from what follows it. */
const expandCommand = (body: string, args: string) => {
  const parts = args.split(/\s+/).filter(Boolean)
  return body.replace(/\$ARGUMENTS|\{\{args\}\}/g, args).replace(/\$([1-9])/g, (_, n) => parts[Number(n) - 1] ?? '').trim()
}

export function TeamView({ project, onError, onOpenSettings, onScope, onOpenPane, voice }: TeamViewProps) {
  const reduce = useReducedMotion() ?? false
  const [agents, setAgents] = useState<TeamAgent[]>([])
  const [agentsLoading, setAgentsLoading] = useState(true)
  const [tasks, setTasks] = useState<TeamTaskSummary[]>([])
  const [activeId, setActiveId] = useState<string | null>(() => readStored<string | null>('neru.team.active', null))
  const [task, setTask] = useState<TeamTask | null>(null)
  const [live, setLive] = useState<Record<string, { text: string; steps: string[]; drawing?: boolean }>>({})
  const [since, setSince] = useState<Record<string, number>>({})
  const [routing, setRouting] = useState<{ from: string; to: string; until: number } | null>(null)
  const [draft, setDraft] = useState('')
  // Files for the next message, copied into the task folder (which every member can read).
  const [attached, setAttached] = useState<string[]>([])
  const [attaching, setAttaching] = useState(0)
  const [to, setTo] = useState<string[]>([])
  const [panel, setPanel] = useState<PanelTab>(() => readStored<PanelTab>('neru.team.panel', 'members'))
  const [panelOpen, setPanelOpen] = useState(() => readStored('neru.team.panelOpen', true))
  // The task list and the task panel are drag-resizable columns.
  const railSize = useDragWidth('neru.team.railWidth', 232, 180, 420, 'right', 0.26)
  const panelSize = useDragWidth('neru.team.panelWidth', 316, 260, 760, 'left', 0.44)
  const wide = panelSize.width >= 520
  const [creating, setCreating] = useState<false | 'blank' | string>(false)
  const [renaming, setRenaming] = useState(false)
  const [menuIndex, setMenuIndex] = useState(0)
  const [filter, setFilter] = useState('')
  const [facets, setFacets] = useState<TaskFilter>(() => readStored('neru.team.filter', NO_FILTER))
  const [menu, setMenu] = useState<null | 'filter' | 'appearance' | 'usage' | 'more'>(null)
  const [closedFor, setClosedFor] = useState<string | null>(null)
  const [queued, setQueued] = useState<string[]>([])
  const [custom, setCustom] = useState<CustomAgent[]>([])
  const [commands, setCommands] = useState<AgentCommand[]>([])
  const [hits, setHits] = useState<TeamSearchHit[]>([])
  const [only, setOnly] = useState<string | null>(null)
  const [deleting, setDeleting] = useState<TeamTaskSummary | null>(null)
  const [collapsed, setCollapsed] = useState<string[]>(() => readStored('neru.team.collapsed', []))
  const [seen, setSeen] = useState<Record<string, number>>(() => readStored('neru.team.seen', {}))
  const [checklistHidden, setChecklistHidden] = useState(() => readStored('neru.team.checklistHidden', false))
  const pendingJump = useRef<string | null>(null)
  const pendingFirst = useRef<string | null>(null)
  const activeRef = useRef(activeId)
  useEffect(() => { activeRef.current = activeId }, [activeId])
  const allAgents = useMemo(() => [...agents, NERU_AGENT], [agents])
  const emptyTour = useTour('neru.tour.team.v1')
  const taskTour = useTour('neru.tour.task.v1')

  const refreshTasks = useCallback(() => void api.listTeamTasks().then(setTasks).catch(cause => onError(errorText(cause))), [onError])
  const loadAgents = useCallback((refresh = false) => { setAgentsLoading(true); void api.listTeamAgents(refresh).then(setAgents).catch(cause => onError(errorText(cause))).finally(() => setAgentsLoading(false)) }, [onError])
  useEffect(() => {
    if (!isTauri()) return
    refreshTasks(); loadAgents()
    void api.listCustomAgents().then(setCustom).catch(() => undefined)
    void api.listAgentCommands().then(setCommands).catch(() => undefined)
  }, [refreshTasks, loadAgents])
  // Browser feedback while in Team lands in this task's message box.
  useEffect(() => {
    composerSink = task ? text => setDraft(current => `${current.trim() ? `${current.trim()}\n\n` : ''}${text}`) : null
    return () => { composerSink = null }
  }, [task])
  // Message search across every task, a moment after typing stops.
  useEffect(() => {
    if (!isTauri() || filter.trim().length < 2) { setHits([]); return }
    const timer = window.setTimeout(() => void api.searchTeam(filter).then(setHits).catch(() => setHits([])), 250)
    return () => window.clearTimeout(timer)
  }, [filter])
  useEffect(() => {
    // A search hit opened another task: scroll to the message once it has rendered.
    if (!task || !pendingJump.current) return
    const id = pendingJump.current
    pendingJump.current = null
    window.setTimeout(() => document.getElementById(`team-post-${id}`)?.scrollIntoView({ block: 'center' }), 120)
  }, [task])
  useEffect(() => {
    writeStored('neru.team.active', activeId)
    setLive({}); setRouting(null); setTo([]); setQueued([]); setOnly(null); setMenu(null); setAttached([])
    if (!activeId || !isTauri()) { setTask(null); return }
    void api.teamSnapshot(activeId).then(setTask).catch(() => { setTask(null); setActiveId(null) })
  }, [activeId])
  // The task's own terminal tabs open in its folder.
  useEffect(() => {
    if (!task?.id || !onScope) return
    let live = true
    void api.teamTaskCwd(task.id).then(cwd => { if (live) onScope({ key: `team:${task.id}`, cwd, label: task.title }) }).catch(() => undefined)
    return () => { live = false }
  }, [task?.id, task?.title, onScope])
  useEffect(() => () => onScope?.(null), [onScope])
  // Reading a task marks its messages seen.
  const postCount = task?.posts.filter(post => post.kind !== 'notice').length ?? 0
  useEffect(() => {
    if (!task) return
    setSeen(current => { if (current[task.id] === postCount) return current; const next = { ...current, [task.id]: postCount }; writeStored('neru.team.seen', next); return next })
  }, [task?.id, postCount]) // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (!isTauri()) return
    const unlisten = api.onTeamEvent((event: TeamEvent) => {
      if (event.type === 'idle' || event.type === 'post' || event.type === 'queue') refreshTasks()
      // Tell the user when a team finishes while Neru is in the background.
      if (event.type === 'idle' && document.hidden && readStored('neru.notifications', true)) void notifyUser(`${event.summary.title}: the team is done`, 'Open Neru to read the thread.')
      if (event.taskId !== activeRef.current) return
      if (event.type === 'member') {
        if (event.member.status === 'working') setSince(current => current[event.member.handle] ? current : { ...current, [event.member.handle]: Date.now() })
        else setSince(current => { const next = { ...current }; delete next[event.member.handle]; return next })
        setTask(current => current && { ...current, members: current.members.map(member => member.handle === event.member.handle ? event.member : member).concat(current.members.some(member => member.handle === event.member.handle) ? [] : [event.member]) })
      } else if (event.type === 'live') setLive(current => ({ ...current, [event.handle]: { text: event.text, steps: event.steps, drawing: event.drawing } }))
      else if (event.type === 'post') {
        setTask(current => current && (current.posts.some(post => post.id === event.post.id) ? current : { ...current, posts: [...current.posts, event.post], updatedAt: event.post.at }))
        setLive(current => { const next = { ...current }; delete next[event.post.author]; return next })
      } else if (event.type === 'setup') setTask(current => current && { ...current, posts: current.posts.map(post => post.id === event.post.id ? event.post : post) })
      else if (event.type === 'queue') setTask(current => current && { ...current, queue: event.queue, queuePaused: event.paused })
      else if (event.type === 'execution') setTask(current => current && { ...current, execution: { ...current.execution, running: event.running } })
      else if (event.type === 'routing') setRouting(event.seconds > 0 ? { from: event.from, to: event.to, until: Date.now() + event.seconds * 1000 } : null)
      else if (event.type === 'queued') setQueued(current => event.queued ? [...current.filter(handle => handle !== event.handle), event.handle] : current.filter(handle => handle !== event.handle))
      else if (event.type === 'error') onError(event.error)
    })
    return () => { void unlisten.then(stop => stop()) }
  }, [refreshTasks, onError])

  useEffect(() => {
    // Ctrl+Shift+U: the usage popover, as in Traycer.
    const onKey = (event: KeyboardEvent) => { if ((event.ctrlKey || event.metaKey) && event.shiftKey && event.key.toLowerCase() === 'u' && task) { event.preventDefault(); setMenu(current => current === 'usage' ? null : 'usage') } }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })
  // Popovers close on a click elsewhere or Esc.
  useEffect(() => {
    if (!menu) return
    const onDown = (event: MouseEvent) => { if (!(event.target as HTMLElement).closest('.menu-anchor')) setMenu(null) }
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') setMenu(null) }
    document.addEventListener('mousedown', onDown)
    window.addEventListener('keydown', onKey)
    return () => { document.removeEventListener('mousedown', onDown); window.removeEventListener('keydown', onKey) }
  }, [menu])
  const openPanel = useCallback((tab: PanelTab) => { setPanel(tab); writeStored('neru.team.panel', tab); setPanelOpen(true); writeStored('neru.team.panelOpen', true) }, [])
  const taskSteps = useMemo(() => taskTourSteps(openPanel, task?.members[0]?.handle ?? 'claude'), [openPanel, task?.members[0]?.handle]) // eslint-disable-line react-hooks/exhaustive-deps
  // Each tour plays once on its own; both can be replayed from the ? buttons.
  useEffect(() => { if (isTauri() && !activeId && !agentsLoading && !emptyTour.seen()) emptyTour.start() }, [activeId, agentsLoading]) // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => { if (task && !emptyTour.open && !taskTour.seen()) taskTour.start() }, [task?.id, emptyTour.open]) // eslint-disable-line react-hooks/exhaustive-deps
  const choosePanel = (tab: PanelTab) => { setPanel(tab); writeStored('neru.team.panel', tab); if (!panelOpen) { setPanelOpen(true); writeStored('neru.team.panelOpen', true) } }
  const togglePanel = () => setPanelOpen(value => { writeStored('neru.team.panelOpen', !value); return !value })
  const working = task?.members.filter(member => member.status === 'working') ?? []
  const handles = task?.members.map(member => member.handle) ?? []

  const create = async (title: string, kinds: string[], mode: string, where: ProjectChoice, quickstart?: string) => {
    try {
      const quick = QUICKSTARTS.find(item => item.id === quickstart)
      const created = await api.createTeamTask(title, where.path, kinds.map(kind => ({ kind, mode })), where.create)
      setCreating(false); setTask(created); setActiveId(created.id); refreshTasks()
      if (quick) pendingFirst.current = quick.message(title)
    } catch (cause) { onError(errorText(cause)) }
  }
  // A quickstart's first message goes out once the new task is open.
  useEffect(() => { if (task && pendingFirst.current) { const text = pendingFirst.current; pendingFirst.current = null; void send(text) } }) // eslint-disable-line react-hooks/exhaustive-deps
  const sendTo = (text: string, recipients: string[]) => { if (task) void api.sendTeamMessage(task.id, text, recipients).then(post => { if (post.kind !== 'queued') setTask(current => current && (current.posts.some(item => item.id === post.id) ? current : { ...current, posts: [...current.posts, post] })) }).catch(cause => onError(errorText(cause))) }
  const send = async (value: string) => {
    if (!task) return
    let text = value.trim()
    let recipients = to
    const command = /^\/([\w-]+)(?:\s+([\s\S]*))?$/.exec(text)
    if (command && SIDE_COMMANDS.includes(command[1].toLowerCase())) {
      // /btw @claude question: answered in a copy of that member's session.
      const rest = command[2]?.trim() ?? ''
      const named = /^@([\w-]+)\s+([\s\S]+)$/.exec(rest)
      const lastSpeaker = [...task.posts].reverse().find(post => handles.includes(post.author))?.author
      const handle = named?.[1].toLowerCase() ?? to.find(item => handles.includes(item)) ?? lastSpeaker ?? handles[0]
      const question = named ? named[2] : rest
      if (!question || !handle) { onError('Ask a side question like this: /btw @claude what does this function return?'); return }
      setDraft('')
      try {
        const post = await api.askTeamSide(task.id, handle, question)
        setTask(current => current && (current.posts.some(item => item.id === post.id) ? current : { ...current, posts: [...current.posts, post] }))
      } catch (cause) { setDraft(value); onError(errorText(cause)) }
      return
    }
    // /model @codex gpt-5.5: a shortcut to set a member's model.
    if (command?.[1].toLowerCase() === 'model') {
      const named = /^@([\w-]+)\s*(\S*)$/.exec(command[2]?.trim() ?? '')
      if (!named || !handles.includes(named[1])) { onError('Set a model like this: /model @codex gpt-5.5 (leave the model out for the agent default)'); return }
      setDraft('')
      try { setTask(await api.updateTeamMember(task.id, named[1], { model: named[2] })) } catch (cause) { setDraft(value); onError(errorText(cause)) }
      return
    }
    // @claude /review-pr 42: one of that agent's own commands, expanded here and sent to it.
    const own = /^@([\w-]+)\s+\/([\w:.-]+)(?:\s+([\s\S]*))?$/.exec(text)
    const member = own && task.members.find(item => item.handle === own[1])
    const found = member && commands.find(item => item.agent === member.kind && item.name === own![2])
    if (own && member && found) { text = expandCommand(found.body, own[3]?.trim() ?? ''); recipients = [member.handle] }
    const skill = !found && command && TEAM_COMMANDS.find(item => item.name === command[1].toLowerCase())
    if (skill) { text = skill.text(command?.[2]?.trim() ?? ''); if (skill.all) recipients = ['all'] }
    if (attached.length) text = `${text || 'Look at the attached files.'}\n\nAttached files (open them from these paths):\n${attached.map(path => `- ${path}`).join('\n')}`
    if (!text) return
    const files = attached
    setDraft(''); setAttached([])
    try {
      const post = await api.sendTeamMessage(task.id, text, recipients)
      if (post.kind !== 'queued') setTask(current => current && (current.posts.some(item => item.id === post.id) ? current : { ...current, posts: [...current.posts, post] }))
    } catch (cause) { setDraft(value); setAttached(files); onError(errorText(cause)) }
  }
  const attach = async (paths: string[], data?: [string, string][]) => {
    if (!task || (!paths.length && !data?.length)) return
    setAttaching(count => count + 1)
    try { const saved = await api.teamAttach(task.id, paths, data); setAttached(current => [...current, ...saved.filter(path => !current.includes(path))]) }
    catch (cause) { onError(errorText(cause)) } finally { setAttaching(count => count - 1) }
  }
  const upload = async () => {
    const picked = await openDialog({ multiple: true, title: 'Attach files for the team' }).catch(() => null)
    const paths = Array.isArray(picked) ? picked : typeof picked === 'string' ? [picked] : []
    await attach(paths)
  }
  // A pasted screenshot becomes an attached image.
  const paste = (event: React.ClipboardEvent) => {
    const images = [...event.clipboardData.files].filter(file => file.type.startsWith('image/'))
    if (!images.length) return
    event.preventDefault()
    void Promise.all(images.map(file => new Promise<[string, string]>((resolve, reject) => { const reader = new FileReader(); reader.onload = () => resolve([file.name && file.name !== 'image.png' ? file.name : `pasted-${Date.now()}.png`, String(reader.result)]); reader.onerror = () => reject(reader.error); reader.readAsDataURL(file) }))).then(data => attach([], data)).catch(cause => onError(errorText(cause)))
  }
  const undo = (postId: string) => { if (task) void api.undoTeamTurn(task.id, postId).then(setTask).catch(cause => onError(errorText(cause))) }
  const act = (work: Promise<TeamTask>) => void work.then(next => { setTask(next); refreshTasks() }).catch(cause => onError(errorText(cause)))
  const sweep = async () => {
    if (!task) return
    try {
      const result = await api.sweepTeamWorktrees(task.id)
      window.alert([result.removed.length ? `Removed: ${result.removed.join(', ')}` : 'Nothing to remove.', result.kept.length ? `Kept: ${result.kept.join('; ')}` : ''].filter(Boolean).join('\n'))
      setTask(await api.teamSnapshot(task.id))
    } catch (cause) { onError(errorText(cause)) }
  }
  const editLabels = () => {
    if (!task) return
    const value = window.prompt('Labels, separated by commas', task.labels.join(', '))
    if (value !== null) act(api.setTeamLabels(task.id, value.split(',')))
  }
  const openTerminal = async (handle: string) => {
    if (!task) return
    try {
      const launch = await api.teamTerminalLaunch(task.id, handle)
      onOpenPane?.('terminal')
      openTerminalTab({ title: launch.title, cwd: launch.cwd, launch: { program: launch.program, args: launch.args, env: launch.env }, scope: `team:${task.id}` })
    } catch (cause) { onError(errorText(cause)) }
  }
  const openHit = (hit: TeamSearchHit) => { pendingJump.current = hit.postId; if (hit.taskId === activeId) { setTask(current => current && { ...current }) } else setActiveId(hit.taskId) }
  const jump = (postId: string) => { setOnly(null); window.setTimeout(() => document.getElementById(`team-post-${postId}`)?.scrollIntoView({ behavior: reduce ? 'auto' : 'smooth', block: 'center' }), 30) }
  const unread = (item: TeamTaskSummary) => item.id !== activeId && seen[item.id] !== undefined && item.posts > seen[item.id]
  // Tasks seen for the first time start as read; only messages that arrive later count as new.
  useEffect(() => {
    setSeen(current => {
      const fresh = tasks.filter(item => current[item.id] === undefined)
      if (fresh.length === 0) return current
      const next = { ...current, ...Object.fromEntries(fresh.map(item => [item.id, item.posts])) }
      writeStored('neru.team.seen', next)
      return next
    })
  }, [tasks])

  const menuOpen = draft !== closedFor
  const mention = menuOpen ? /(^|\s)@([\w-]*)$/.exec(draft) : null
  const mentionItems = mention && task ? [...task.members.map(member => ({ id: member.handle, label: AGENT_NAMES[member.kind] ?? member.kind, kind: member.kind, status: member.status })), { id: 'all', label: 'Everyone on the task', kind: 'all', status: '' as const }].filter(item => item.id.startsWith(mention[2].toLowerCase()) || item.label.toLowerCase().startsWith(mention[2].toLowerCase())) : []
  const slash = menuOpen ? /^\/([\w:.-]*)$/.exec(draft.trim()) : null
  const slashItems = slash ? TEAM_COMMANDS.filter(item => item.name.startsWith(slash[1].toLowerCase())) : []
  // The members' own commands: "/" lists them with their agent; picking one addresses that member.
  const ownItems = slash && task ? commands.flatMap(command => task.members.filter(member => member.kind === command.agent).slice(0, 1).map(member => ({ command, member }))).filter(({ command }) => command.name.toLowerCase().includes(slash[1].toLowerCase())).slice(0, 12) : []
  const statusLabel = (status: string) => status === 'working' ? 'Working' : status === 'failed' ? 'Failed' : status === 'stopped' ? 'Stopped' : 'Ready'
  const menuItems: { key: string; pick: () => void; node: React.ReactNode }[] = mentionItems.length
    ? mentionItems.map(item => ({ key: item.id, pick: () => setDraft(draft.replace(/@([\w-]*)$/, `@${item.id} `)), node: <>
      {item.kind === 'all' ? <span className="mention-all"><Users size={15} /></span> : <AgentMark kind={item.kind} size={28} status={item.status} />}
      <span className="mention-text"><strong>{item.label}</strong><small>@{item.id}</small></span>
      {item.kind !== 'all' && <span className={cn('team-status', `is-${item.status || 'idle'}`)}>{statusLabel(item.status)}</span>}
    </> }))
    : [
      ...slashItems.map(item => ({ key: item.name, pick: () => setDraft(`/${item.name} `), node: <><code>/{item.name}</code><span>{item.description}</span></> })),
      ...ownItems.map(({ command, member }) => ({ key: `${member.handle}:${command.name}`, pick: () => setDraft(`@${member.handle} /${command.name} `), node: <><AgentMark kind={member.kind} size={18} /><code>/{command.name}</code><span>{command.description || `@${member.handle}'s own command`}</span></> })),
    ]
  const menuAt = Math.min(menuIndex, Math.max(0, menuItems.length - 1))

  const readyAgents = agents.filter(agent => agent.path)
  const text = filter.trim().toLowerCase()
  const listed = applyFilter(tasks, facets, unread).filter(item => !text || `${item.title} ${item.projectPath} ${item.labels.join(' ')} ${item.group} ${item.members.map(member => member.handle).join(' ')}`.toLowerCase().includes(text))
  const groups = [...new Set(listed.map(item => item.group))].sort((a, b) => (a === '' ? 1 : 0) - (b === '' ? 1 : 0) || a.localeCompare(b))
  const allGroups = [...new Set(tasks.map(item => item.group).filter(Boolean))]
  const toggleGroup = (name: string) => setCollapsed(current => { const next = current.includes(name) ? current.filter(item => item !== name) : [...current, name]; writeStored('neru.team.collapsed', next); return next })
  const row = (item: TeamTaskSummary) => <div key={item.id} className={cn('team-task-row', item.id === activeId && 'active')}>
    <button className="team-task-select" onClick={() => setActiveId(item.id)} title={item.projectPath || 'No project'}>
      <span className="team-task-title"><TaskGlyph icon={item.icon} color={item.color} />{item.pinned && <Pin size={11} className="team-pin" aria-label="Pinned" />}<span className="truncate">{item.title}</span></span>
      <span className="team-task-meta"><span className="team-stack">{item.members.slice(0, 4).map(member => <AgentMark key={member.handle} kind={member.kind} size={16} status={member.status} />)}</span>{item.projectPath && <span className="truncate">{baseName(item.projectPath)}</span>}</span>
      {item.labels.length > 0 && <span className="team-labels">{item.labels.slice(0, 3).map(label => <span key={label} className="team-label">{label}</span>)}</span>}
    </button>
    <span className="team-task-state">
      {item.running ? <SpinnerRing active size={16} /> : item.failed ? <CircleAlert size={13} className="team-task-failed" aria-label="A member failed" /> : <time>{relativeTime(item.updatedAt)}</time>}
      {item.queued > 0 && <span className="team-task-queued" title={`${item.queued} queued`}>{item.queued}</span>}
      {unread(item) && <span className="team-unread" aria-label="New messages" />}
    </span>
    <button className="icon-button small team-task-delete" onClick={() => setDeleting(item)} aria-label={`Delete ${item.title}`} title="Delete task"><Trash2 size={12} /></button>
  </div>
  const rail = <aside className="team-rail" aria-label="Team tasks" data-tour="rail">
    <div {...railSize.handle} aria-label="Resize the task list" />
    <div className="team-rail-head"><span>Tasks</span>
      <div className="menu-anchor"><button className={cn('icon-button small', (filterActive(facets) || facets.sort !== 'updated') && 'active')} onClick={() => setMenu(current => current === 'filter' ? null : 'filter')} aria-label="Filter and sort" aria-expanded={menu === 'filter'} title="Filter and sort" data-tour="filter"><ListFilter size={14} /></button>
        {menu === 'filter' && <FilterMenu tasks={tasks} filter={facets} names={agentName} onChange={value => { setFacets(value); writeStored('neru.team.filter', value) }} onClose={() => setMenu(null)} />}</div>
      <button className="icon-button small" onClick={() => setCreating('blank')} aria-label="New task" title="New task"><Plus size={14} /></button></div>
    {tasks.length > 0 && <label className="team-rail-search" data-tour="search"><Search size={13} /><input value={filter} onChange={event => setFilter(event.target.value)} placeholder="Search tasks and messages" aria-label="Search tasks and messages" /></label>}
    <div className="team-rail-list">
      {tasks.length === 0 && <p className="sidebar-empty">No tasks yet.</p>}
      {tasks.length > 0 && listed.length === 0 && <p className="sidebar-empty">No task matches. <button className="team-link" onClick={() => { setFacets(NO_FILTER); writeStored('neru.team.filter', NO_FILTER); setFilter('') }}>Clear filters</button></p>}
      {groups.length <= 1 && !groups[0] ? listed.map(row) : groups.map(name => {
        const items = listed.filter(item => item.group === name)
        const shut = collapsed.includes(name)
        return <section key={name || 'none'} className="team-group">
          <button className="team-group-head" onClick={() => toggleGroup(name)} aria-expanded={!shut}><ChevronRight size={12} style={{ transform: shut ? undefined : 'rotate(90deg)' }} /><span className="truncate">{name || 'Ungrouped'}</span><small>{items.length}</small></button>
          {!shut && items.map(row)}
        </section>
      })}
    </div>
    {hits.length > 0 && <div className="team-hits"><div className="team-rail-head"><span>In messages</span></div>
      {hits.map(hit => <button key={hit.postId} className="team-hit" onClick={() => openHit(hit)}><strong className="truncate">{hit.taskTitle}</strong><span>{hit.author === 'you' ? 'You' : `@${hit.author}`}: {hit.snippet}</span></button>)}
    </div>}
    <button className="team-rail-import" data-tour="import" onClick={() => onOpenSettings('imports')}><Download size={14} />Import chats & skills</button>
  </aside>

  const empty = <div className="team-empty">
    <div className="team-empty-hero"><Mascot size={64} interactive /><h1>Your agents, <em>one thread</em>.</h1><p>Bring the subscriptions you already pay for. Claude Code, Codex, Gemini, Cursor, OpenCode and more work side by side, see each other’s replies and hand work to one another.</p></div>
    {!checklistHidden && <Checklist agents={agents} tasks={tasks} toured={!emptyTour.open && emptyTour.seen()} onDismiss={() => { setChecklistHidden(true); writeStored('neru.team.checklistHidden', true) }}
      onAction={id => id === 'agents' ? onOpenSettings('agents') : id === 'import' ? onOpenSettings('imports') : id === 'task' ? setCreating('blank') : emptyTour.start()} />}
    <div className="team-agent-grid" data-tour="agents">
      {agentsLoading && agents.length === 0 && <p className="empty-small">Looking for agents on this machine…</p>}
      {agents.filter(agent => agent.path || ['claude', 'codex', 'gemini', 'cursor', 'opencode', 'copilot'].includes(agent.kind)).map(agent => <motion.div key={agent.kind} className={cn('team-agent-card', !agent.path && 'missing')} initial={reduce ? false : { opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.3, ease: EASE_OUT }}>
        <AgentMark kind={agent.kind} size={32} />
        <div><strong>{agent.name}</strong><span>{!agent.path ? 'Not installed' : agent.signedIn ? `Ready${agent.version ? ` · ${agent.version.replace(/\s*\(.*\)$/, '')}` : ''}` : 'Installed, not signed in'}</span></div>
        {!agent.path ? <button className="button subtle small" title={agent.install} onClick={() => void navigator.clipboard.writeText(agent.install)}><Copy size={12} />Install</button> : !agent.signedIn && <button className="button subtle small" onClick={() => onOpenSettings('agents')}>{agent.kind === 'gemini' ? 'Add key' : 'Sign in'}</button>}
      </motion.div>)}
    </div>
    <div className="team-empty-start">
      <h2>Start from</h2>
      <Quickstart onPick={id => setCreating(id)} />
    </div>
    <div className="team-empty-actions">
      <button className="button primary" data-tour="start" onClick={() => setCreating('blank')} disabled={readyAgents.length === 0 && agentsLoading}><Plus size={15} />Start a task</button>
      <button className="button subtle" onClick={() => onOpenSettings('imports')}><Download size={15} />Import chats & skills</button>
      <button className="button subtle" onClick={() => loadAgents(true)} disabled={agentsLoading}><RefreshCw size={15} className={agentsLoading ? 'animate-spin' : undefined} />Look again</button>
      <button className="button subtle" onClick={() => emptyTour.start()}><CircleHelp size={15} />Take the tour</button>
    </div>
  </div>

  const reading = task ? usageReading(task) : null
  const shownPosts = task ? (only ? task.posts.filter(post => post.author === only || post.to.includes(only)) : task.posts) : []
  return <main className={cn('team-view', panelOpen && task && 'with-panel', (railSize.dragging || panelSize.dragging) && 'resizing')} style={{ '--rail-w': `${railSize.width}px`, '--panel-w': `${panelSize.width}px` } as React.CSSProperties}>
    {rail}
    <section className="team-main" aria-label={task?.title ?? 'Team'}>
      {!task ? empty : <>
        <header className="team-head">
          <TaskGlyph icon={task.icon} color={task.color} size={15} />
          {renaming
            ? <input className="session-rename" autoFocus defaultValue={task.title} aria-label="Rename task" onBlur={event => { setRenaming(false); void api.renameTeamTask(task.id, event.target.value).then(setTask).then(refreshTasks).catch(cause => onError(errorText(cause))) }} onKeyDown={event => { if (event.key === 'Enter') event.currentTarget.blur(); if (event.key === 'Escape') setRenaming(false) }} />
            : <button className="team-title" onDoubleClick={() => setRenaming(true)} title="Double-click to rename"><h2 className="truncate">{task.title}</h2></button>}
          <div className="team-roster" data-tour="roster">{task.members.map(member => <button key={member.handle} className={cn('team-chip', to.includes(member.handle) && 'on')} onClick={() => setTo(current => current.includes(member.handle) ? current.filter(item => item !== member.handle) : [...current, member.handle])} title={`Send to @${member.handle}`}>
            <AgentMark kind={member.kind} size={18} status={member.status} /><span>@{member.handle}</span></button>)}</div>
          <div className="menu-anchor">
            <button className={cn('team-reading', reading?.level && `is-${reading.level}`)} onClick={() => setMenu(current => current === 'usage' ? null : 'usage')} aria-expanded={menu === 'usage'} title="Usage and limits (Ctrl+Shift+U)" data-tour="reading"><BarChart3 size={13} /><span>{reading?.text ?? 'Usage'}</span></button>
            {menu === 'usage' && <div className="menu menu-down team-usage-pop" role="dialog" aria-label="Usage and limits" onKeyDown={event => { if (event.key === 'Escape') setMenu(null) }}><UsageDashboard task={task} names={agentName} onError={onError} compact /><button className="button subtle small" onClick={() => { setMenu(null); choosePanel('usage') }}>Open the dashboard</button></div>}
          </div>
          {working.length > 0 && <button className="button subtle small" onClick={() => void api.stopTeam(task.id).catch(cause => onError(errorText(cause)))}><Square size={12} />Stop</button>}
          <div className="menu-anchor"><button className={cn('icon-button', (menu === 'more' || menu === 'appearance') && 'active')} onClick={() => setMenu(current => current === 'more' || current === 'appearance' ? null : 'more')} aria-label="Task options" aria-haspopup="menu" aria-expanded={menu === 'more'} title="Pin, labels, group, icon and colour" data-tour="options"><MoreHorizontal size={16} /></button>
            {menu === 'more' && <div className="menu menu-down team-more" role="menu" onKeyDown={event => { if (event.key === 'Escape') setMenu(null) }}>
              <button role="menuitem" autoFocus onClick={() => { setMenu(null); act(api.setTeamPinned(task.id, !task.pinned)) }}><Pin size={14} />{task.pinned ? 'Unpin' : 'Pin to the top'}</button>
              <button role="menuitem" onClick={() => { setMenu(null); editLabels() }}><Tag size={14} />Labels…{task.labels.length > 0 && <small>{task.labels.join(', ')}</small>}</button>
              <button role="menuitem" onClick={() => setMenu('appearance')}><Smile size={14} />Group, icon and colour…{task.group && <small>{task.group}</small>}</button>
              {task.members.some(member => member.worktree) && <button role="menuitem" onClick={() => { setMenu(null); void sweep() }}><Brush size={14} />Sweep landed worktrees</button>}
              <div className="menu-separator" />
              <button role="menuitem" className="danger" onClick={() => { setMenu(null); const summary = tasks.find(item => item.id === task.id); if (summary) setDeleting(summary) }}><Trash2 size={14} />Delete task…</button>
            </div>}
            {menu === 'appearance' && <AppearanceMenu task={task} groups={allGroups} onChange={change => act(api.setTeamAppearance(task.id, change))} onClose={() => setMenu(null)} />}</div>
          <button className="icon-button" data-tour="help" onClick={() => taskTour.start()} aria-label="Take the tour" title="Take the tour"><CircleHelp size={16} /></button>
          <button className={cn('icon-button', panelOpen && 'active')} onClick={togglePanel} aria-label="Toggle the task panel" aria-pressed={panelOpen} title="Members, artifacts, changes, usage"><PanelRight size={16} /></button>
        </header>
        {only && <div className="team-only" role="status"><Eye size={13} /><span>Showing @{only}’s transcript: its posts and what was sent to it.</span><button className="team-link" onClick={() => void api.revealPath(`${task.folder}/transcripts/${only}.md`).catch(cause => onError(errorText(cause)))}>Open the file</button><button className="team-link" onClick={() => setOnly(null)}>Show everyone</button></div>}
        <MessageScroller data-tour="thread" className="team-thread" label="Team thread" busy={working.length > 0} contentClassName="team-thread-content">
          {task.posts.length === 0 && <div className="team-thread-empty"><p>Say what you need. Mention <code>@{handles[0]}</code> to pick who answers, <code>@all</code> for everyone, or try <code>/plan</code>, <code>/debate</code>, <code>/review</code>. Ask on the side with <code>/btw @{handles[0]} …</code>.</p></div>}
          {shownPosts.map(post => <div key={post.id} id={`team-post-${post.id}`}><PostView taskId={task.id} post={post} member={task.members.find(member => member.handle === post.author)} handles={handles} onOpenSettings={() => onOpenSettings('agents')} onUndo={undo} onRerun={handle => void api.rerunTeamSetup(task.id, handle).catch(cause => onError(errorText(cause)))} /></div>)}
          {working.filter(member => !only || member.handle === only).map(member => <LivePost key={member.handle} member={member} live={live[member.handle]} handles={handles} />)}
          {routing && <RoutingCard routing={routing} onCancel={() => void api.cancelTeamRouting(task.id).catch(cause => onError(errorText(cause)))} />}
        </MessageScroller>
        <div className="team-input composer" data-tour="composer">
          <QueueBar task={task} onError={onError} />
          {queued.length > 0 && <div className="team-queue" role="status">{queued.map(handle => <span key={handle}><History size={11} />Hand-off to @{handle} waits for its current turn</span>)}</div>}
          {menuItems.length > 0 && <div className={cn('slash-menu', mentionItems.length > 0 && 'mention-menu')} role="listbox" aria-label={mentionItems.length ? 'Mention a teammate' : 'Team skills and agent commands'}>
            <div className="menu-caption">{mentionItems.length ? 'Mention a teammate' : ownItems.length && slashItems.length ? 'Team skills, then each agent’s own commands' : ownItems.length ? 'Agent commands' : 'Team skills'}</div>
            {menuItems.map((item, index) => <button key={item.key} type="button" role="option" aria-selected={index === menuAt} className={cn('slash-item', mentionItems.length > 0 && 'mention-item', index === menuAt && 'active')} onMouseEnter={() => setMenuIndex(index)} onMouseDown={event => { event.preventDefault(); item.pick() }}>{item.node}</button>)}
            <div className="menu-hint"><kbd>↑</kbd><kbd>↓</kbd> to move <kbd>Enter</kbd> to pick <kbd>Esc</kbd> to close</div>
          </div>}
          <div onPaste={paste} onDragOver={event => { if (event.dataTransfer.types.includes('Files')) event.preventDefault() }}>
          <Composer value={draft} onValueChange={value => { setDraft(value); setMenuIndex(0) }} onSubmit={value => void send(value)}
            loading={working.length > 0} onStop={() => void api.stopTeam(task.id).catch(cause => onError(errorText(cause)))} onSteer={value => void send(value)}
            sendWithoutText={attached.length > 0}
            light={voice.light} voiceEngine={voice.engine} voiceReady={voice.ready} onVoiceStart={voice.onStart} onVoiceUnavailable={voice.onUnavailable} onTranscribe={voice.onTranscribe} onError={onError}
            attachItems={[{ id: 'upload', label: 'Upload files', hint: 'Images, documents, anything', icon: attachIcons.upload, onSelect: () => void upload() }]}
            mode="manual" onModeChange={() => undefined} showMode={false} web={false} onWebChange={() => undefined} showWeb={false}
            commands={[]} onCommand={() => false} model="" models={[]} onModelChange={() => undefined}
            effort="auto" effortSupported={false} onEffortChange={() => undefined} context={null}
            attachments={<><div className="team-to-row" aria-label="Recipients" data-tour="to">
              <span>To</span>
              <button type="button" className={cn('team-to-chip', to.length === 0 && 'on')} onClick={() => setTo([])} title="Whoever you mention, else whoever spoke last">Auto</button>
              <button type="button" className={cn('team-to-chip', to.includes('all') && 'on')} onClick={() => setTo(['all'])} title="Every member answers in parallel"><Users size={12} />All</button>
              {to.filter(handle => handle !== 'all').map(handle => <span key={handle} className="team-to-chip on">@{handle}<button type="button" onClick={() => setTo(current => current.filter(item => item !== handle))} aria-label={`Remove @${handle}`}><X size={11} /></button></span>)}
            </div>
            {(attached.length > 0 || attaching > 0) && <Attachments paths={attached.map(path => baseName(path))} documents={[]} reading={attaching} onRemovePath={name => setAttached(current => current.filter(path => baseName(path) !== name))} onRemoveDocument={() => undefined} />}</>}
            placeholder={task.members.length ? `Message the team… @${handles[0]} to pick, / for skills and commands` : 'Add an agent to start'}
            disabled={task.members.length === 0} ariaLabel="Message the team"
            onKeyDown={event => {
              if (menuItems.length === 0) return
              if (event.key === 'Escape') { event.preventDefault(); setClosedFor(draft); return }
              if (event.key === 'ArrowDown' || event.key === 'ArrowUp') { event.preventDefault(); setMenuIndex(index => (index + (event.key === 'ArrowDown' ? 1 : -1) + menuItems.length) % menuItems.length) }
              if ((event.key === 'Enter' && !event.shiftKey) || event.key === 'Tab') { event.preventDefault(); menuItems[menuAt].pick() }
            }} />
          </div>
        </div>
      </>}
    </section>
    {task && panelOpen && <aside className="team-panel" aria-label="Task panel" data-tour="panel">
      <div {...panelSize.handle} aria-label="Resize the task panel" />
      <nav className="segmented team-tabs" aria-label="Task panel sections">
        {PANEL_TABS.map(([id, label, Icon]) => <button key={id} className={panel === id ? 'active' : ''} aria-current={panel === id ? 'page' : undefined} aria-label={label} onClick={() => choosePanel(id)} title={label}><Icon size={14} />{panel === id && <span>{label}</span>}</button>)}
        <button className="team-tab-wide" onClick={() => panelSize.set(wide ? 316 : 620)} aria-pressed={wide} aria-label={wide ? 'Narrow panel' : 'Wide panel'} title={wide ? 'Narrow panel (or drag its edge)' : 'Wide panel (or drag its edge)'}>{wide ? <Minimize2 size={13} /> : <Maximize2 size={13} />}</button>
      </nav>
      {panel === 'members' && <MembersPanel task={task} agents={allAgents} custom={custom} only={only} onOnly={setOnly} onChange={setTask} onError={onError} onTerminal={handle => void openTerminal(handle)} />}
      {panel === 'artifacts' && <ArtifactsPanel task={task} onError={onError} />}
      {panel === 'changes' && <ChangesPanel task={task} onError={onError} onSend={sendTo} />}
      {panel === 'pr' && <PrPanel task={task} onError={onError} onSend={sendTo} />}
      {panel === 'activity' && <ActivityPanel task={task} live={live} since={since} routing={routing} names={agentName} onChange={setTask} onError={onError} onStop={handle => void api.stopTeam(task.id, handle).catch(cause => onError(errorText(cause)))} />}
      {panel === 'usage' && <UsageDashboard task={task} names={agentName} onError={onError} />}
      {panel === 'map' && <MapPanel task={task} onJump={jump} />}
    </aside>}
    {emptyTour.open && <Tour label="Team walkthrough" steps={EMPTY_TOUR} onClose={emptyTour.close} />}
    {taskTour.open && task && <Tour label="Task walkthrough" steps={taskSteps} onClose={taskTour.close} />}
    {creating && <NewTaskDialog agents={allAgents} project={project} preset={creating === 'blank' ? undefined : creating} onClose={() => setCreating(false)} onCreate={(title, kinds, mode, where, quickstart) => void create(title, kinds, mode, where, quickstart)} />}
    {deleting && <DeleteDialog task={deleting} onError={onError} onClose={() => setDeleting(null)} onDeleted={() => { if (deleting.id === activeId) setActiveId(null); setDeleting(null); refreshTasks() }} />}
  </main>
}
