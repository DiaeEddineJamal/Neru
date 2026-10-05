import { useCallback, useEffect, useMemo, useState, type ReactNode } from 'react'
import { save as saveDialog } from '@tauri-apps/plugin-dialog'
import { ArrowDown, ArrowUp, BookOpen, Bug, Check, CheckCircle2, ChevronDown, CircleAlert, CircleDashed, Code2, Database, Download, ExternalLink, FlaskConical, GitPullRequest, Globe, LoaderCircle, Palette, Pause, Play, RefreshCw, Rocket, Send, Shield, Sparkles, Square, Trash2, Wrench, X, XCircle, Zap, type LucideIcon } from 'lucide-react'
import { FileDiff } from '@/components/agents/file-diff'
import { languageForPath } from '@/components/agents/agent-code'
import { parseUnifiedDiff } from '@/lib/diff'
import { cn } from '@/lib/utils'
import { api } from '../../../api'
import type { AgentModel, PrStatus, TeamAgent, TeamPost, TeamQueued, TeamTask, TeamTaskFile, TeamTaskSummary, TeamTreeInfo } from '../../../types'
import './TeamPanels.css'

const errorText = (value: unknown) => value instanceof Error ? value.message : String(value)
const tokens = (n: number) => n >= 1_000_000 ? `${(n / 1_000_000).toFixed(1)}M` : n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n)
const readStored = <T,>(key: string, fallback: T): T => { try { const raw = localStorage.getItem(key); return raw ? JSON.parse(raw) as T : fallback } catch { return fallback } }
const writeStored = (key: string, value: unknown) => { try { localStorage.setItem(key, JSON.stringify(value)) } catch { /* storage unavailable */ } }

/** Task icons and colours for the task list. */
export const TASK_ICONS: Record<string, LucideIcon> = { sparkles: Sparkles, bug: Bug, rocket: Rocket, wrench: Wrench, book: BookOpen, flask: FlaskConical, shield: Shield, palette: Palette, database: Database, globe: Globe, zap: Zap, code: Code2 }
export const TASK_COLORS = ['sage', 'sky', 'amber', 'rose', 'violet', 'teal', 'slate'] as const

export function TaskGlyph({ icon, color, size = 13 }: { icon: string; color: string; size?: number }) {
  const Icon = TASK_ICONS[icon]
  if (!Icon && !color) return null
  return <span className={cn('task-glyph', color && `task-${color}`)} aria-hidden>{Icon ? <Icon size={size} strokeWidth={2} /> : <i />}</span>
}

/** The task list's group, icon and colour. */
export function AppearanceMenu({ task, groups, onChange, onClose }: { task: TeamTask; groups: string[]; onChange: (change: { group?: string; icon?: string; color?: string }) => void; onClose: () => void }) {
  const [group, setGroup] = useState(task.group)
  return <div className="menu menu-down team-appearance" role="dialog" aria-label="Task appearance" onKeyDown={event => { if (event.key === 'Escape') onClose() }}>
    <label className="team-field">Group<input autoFocus list="team-groups" value={group} placeholder="No group" onChange={event => setGroup(event.target.value)} onBlur={() => { if (group !== task.group) onChange({ group }) }} onKeyDown={event => { if (event.key === 'Enter') { onChange({ group }); onClose() } }} /></label>
    <datalist id="team-groups">{groups.map(name => <option key={name} value={name} />)}</datalist>
    <div className="menu-caption">Icon</div>
    <div className="team-icon-grid">
      <button type="button" className={cn(!task.icon && 'on')} onClick={() => onChange({ icon: '' })} aria-label="No icon" title="No icon"><X size={14} /></button>
      {Object.entries(TASK_ICONS).map(([name, Icon]) => <button type="button" key={name} className={cn(task.icon === name && 'on')} onClick={() => onChange({ icon: name })} aria-label={name} title={name}><Icon size={15} /></button>)}
    </div>
    <div className="menu-caption">Colour</div>
    <div className="team-color-grid">
      <button type="button" className={cn('task-swatch', !task.color && 'on')} onClick={() => onChange({ color: '' })} aria-label="No colour" title="No colour"><X size={12} /></button>
      {TASK_COLORS.map(color => <button type="button" key={color} className={cn('task-swatch', `task-${color}`, task.color === color && 'on')} onClick={() => onChange({ color })} aria-label={color} title={color}><i /></button>)}
    </div>
  </div>
}

export type TaskFilter = { projects: string[]; agents: string[]; labels: string[]; match: 'all' | 'any'; sort: 'updated' | 'created' | 'title' | 'activity'; unread: boolean }
export const NO_FILTER: TaskFilter = { projects: [], agents: [], labels: [], match: 'all', sort: 'updated', unread: false }
export const filterActive = (filter: TaskFilter) => filter.projects.length + filter.agents.length + filter.labels.length > 0 || filter.unread

/** Applies the rail's filters: every chosen condition (AND) or any of them (OR), then the sort. */
export function applyFilter(tasks: TeamTaskSummary[], filter: TaskFilter, unread: (task: TeamTaskSummary) => boolean) {
  const base = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path
  const checks: ((task: TeamTaskSummary) => boolean)[] = [
    ...filter.projects.map(project => (task: TeamTaskSummary) => base(task.projectPath) === project),
    ...filter.agents.map(agent => (task: TeamTaskSummary) => task.members.some(member => member.kind === agent)),
    ...filter.labels.map(label => (task: TeamTaskSummary) => task.labels.includes(label)),
    ...(filter.unread ? [unread] : []),
  ]
  const kept = checks.length === 0 ? tasks : tasks.filter(task => filter.match === 'all' ? checks.every(check => check(task)) : checks.some(check => check(task)))
  const order: Record<TaskFilter['sort'], (a: TeamTaskSummary, b: TeamTaskSummary) => number> = {
    updated: (a, b) => b.updatedAt - a.updatedAt,
    created: (a, b) => b.createdAt - a.createdAt,
    title: (a, b) => a.title.localeCompare(b.title),
    activity: (a, b) => b.posts - a.posts,
  }
  return [...kept].sort((a, b) => Number(b.pinned) - Number(a.pinned) || order[filter.sort](a, b))
}

export function FilterMenu({ tasks, filter, names, onChange, onClose }: { tasks: TeamTaskSummary[]; filter: TaskFilter; names: (kind: string) => string; onChange: (filter: TaskFilter) => void; onClose: () => void }) {
  const base = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path
  const projects = [...new Set(tasks.map(task => task.projectPath).filter(Boolean).map(base))].sort()
  const agents = [...new Set(tasks.flatMap(task => task.members.map(member => member.kind)))].sort()
  const labels = [...new Set(tasks.flatMap(task => task.labels))].sort()
  const toggle = (key: 'projects' | 'agents' | 'labels', value: string) => onChange({ ...filter, [key]: filter[key].includes(value) ? filter[key].filter(item => item !== value) : [...filter[key], value] })
  const group = (title: string, key: 'projects' | 'agents' | 'labels', values: string[], label: (value: string) => string = value => value) => values.length > 0 && <>
    <div className="menu-caption">{title}</div>
    <div className="team-chip-row">{values.map(value => <button type="button" key={value} className={cn('team-to-chip', filter[key].includes(value) && 'on')} aria-pressed={filter[key].includes(value)} onClick={() => toggle(key, value)}>{label(value)}</button>)}</div>
  </>
  return <div className="menu menu-down team-filter" role="dialog" aria-label="Filter and sort tasks" onKeyDown={event => { if (event.key === 'Escape') onClose() }}>
    <div className="team-filter-head"><span>Match</span>
      <div className="theme-toggle team-access" role="radiogroup" aria-label="Match">{(['all', 'any'] as const).map(match => <button type="button" key={match} role="radio" aria-checked={filter.match === match} className={filter.match === match ? 'active' : ''} onClick={() => onChange({ ...filter, match })} title={match === 'all' ? 'Tasks that match every choice' : 'Tasks that match any choice'}>{match === 'all' ? 'All (AND)' : 'Any (OR)'}</button>)}</div>
    </div>
    <label className="team-check"><input type="checkbox" checked={filter.unread} onChange={event => onChange({ ...filter, unread: event.target.checked })} />Unread only</label>
    {group('Repository', 'projects', projects)}
    {group('Agent', 'agents', agents, names)}
    {group('Label', 'labels', labels)}
    <label className="team-field">Sort by<select value={filter.sort} onChange={event => onChange({ ...filter, sort: event.target.value as TaskFilter['sort'] })}>
      <option value="updated">Last activity</option><option value="created">Newest first</option><option value="title">Title</option><option value="activity">Most messages</option>
    </select></label>
    <div className="team-filter-foot"><button type="button" className="button subtle small" onClick={() => onChange(NO_FILTER)} disabled={!filterActive(filter) && filter.sort === 'updated'}>Clear</button><button type="button" className="button primary small" onClick={onClose}>Done</button></div>
  </div>
}

/** Messages waiting for busy members: edit, reorder, send now, remove, or pause the queue. */
export function QueueBar({ task, onError }: { task: TeamTask; onError: (message: string) => void }) {
  const [editing, setEditing] = useState<string | null>(null)
  const [text, setText] = useState('')
  if (task.queue.length === 0 && !task.queuePaused) return null
  const act = (change: Parameters<typeof api.updateTeamQueue>[1]) => void api.updateTeamQueue(task.id, change).catch(cause => onError(errorText(cause)))
  return <div className="team-queue-list" role="region" aria-label="Queued messages">
    <div className="team-queue-head"><strong>{task.queue.length ? `Queued (${task.queue.length})` : 'Queue'}</strong><span>{task.queuePaused ? 'Paused: nothing is sent until you resume.' : 'Sent in order as members finish.'}</span>
      <button type="button" className="button subtle small" onClick={() => act({ paused: !task.queuePaused })}>{task.queuePaused ? <><Play size={12} />Resume</> : <><Pause size={12} />Pause</>}</button></div>
    {task.queue.map((item: TeamQueued, index) => <div key={item.id} className="team-queue-item">
      <span className="team-queue-to">{item.to.map(handle => `@${handle}`).join(', ')}</span>
      {editing === item.id
        ? <form className="team-queue-edit" onSubmit={event => { event.preventDefault(); act({ item: item.id, text }); setEditing(null) }}><input autoFocus value={text} onChange={event => setText(event.target.value)} onKeyDown={event => { if (event.key === 'Escape') setEditing(null) }} aria-label="Edit queued message" /><button type="submit" className="icon-button small" aria-label="Save"><Check size={13} /></button></form>
        : <button type="button" className="team-queue-text" onClick={() => { setEditing(item.id); setText(item.text) }} title="Edit">{item.text}</button>}
      <button type="button" className="icon-button small" disabled={index === 0} onClick={() => act({ item: item.id, delta: -1 })} aria-label="Move up" title="Move up"><ArrowUp size={13} /></button>
      <button type="button" className="icon-button small" disabled={index === task.queue.length - 1} onClick={() => act({ item: item.id, delta: 1 })} aria-label="Move down" title="Move down"><ArrowDown size={13} /></button>
      <button type="button" className="icon-button small" onClick={() => act({ item: item.id, sendNow: true })} aria-label="Send now" title="Send now: each member takes it after its current turn"><Send size={13} /></button>
      <button type="button" className="icon-button small" onClick={() => act({ item: item.id, remove: true })} aria-label="Remove" title="Remove"><Trash2 size={13} /></button>
    </div>)}
  </div>
}

/** A worktree setup run: running, done or failed, with its output and a rerun. */
export function SetupCard({ post, onRerun }: { post: TeamPost; onRerun: () => void }) {
  const [open, setOpen] = useState(post.status === 'failed')
  useEffect(() => { if (post.status === 'failed') setOpen(true) }, [post.status])
  const [head, ...rest] = post.text.split('\n\n')
  const output = rest.join('\n\n').replace(/^```\n?|\n?```$/g, '')
  return <div className={cn('team-setup', `is-${post.status ?? 'ok'}`)} role="status">
    <div className="team-setup-head">
      {post.status === 'running' ? <LoaderCircle size={15} className="animate-spin" /> : post.status === 'failed' ? <XCircle size={15} /> : <CheckCircle2 size={15} />}
      <div><strong>{post.status === 'running' ? `Setting up @${post.author}'s worktree` : post.status === 'failed' ? `Setup failed for @${post.author}` : `@${post.author}'s worktree is ready`}</strong><span>{head.replace(/`/g, '')}</span></div>
      {output && <button type="button" className="icon-button small" onClick={() => setOpen(value => !value)} aria-expanded={open} aria-label="Show output" title="Output"><ChevronDown size={14} style={{ transform: open ? 'rotate(180deg)' : undefined }} /></button>}
      {post.status !== 'running' && <button type="button" className="button subtle small" onClick={onRerun}><RefreshCw size={12} />Run again</button>}
    </div>
    {open && output && <pre className="team-setup-output">{output}</pre>}
  </div>
}

/** Deleting a task: what happens to each member's worktree and branch, graded by what would be lost. */
export function DeleteDialog({ task, onClose, onDeleted, onError }: { task: TeamTaskSummary; onClose: () => void; onDeleted: () => void; onError: (message: string) => void }) {
  const [trees, setTrees] = useState<TeamTreeInfo[] | null>(null)
  const [removeTrees, setRemoveTrees] = useState(true)
  const [branches, setBranches] = useState(false)
  const [confirm, setConfirm] = useState('')
  const [busy, setBusy] = useState(false)
  useEffect(() => { void api.teamCleanupInfo(task.id).then(setTrees).catch(() => setTrees([])) }, [task.id])
  const dirty = trees?.reduce((sum, tree) => sum + tree.dirty, 0) ?? 0
  const ahead = trees?.reduce((sum, tree) => sum + tree.ahead, 0) ?? 0
  // Losing uncommitted files or unmerged commits needs the task's name typed out.
  const risky = removeTrees && (dirty > 0 || (branches && ahead > 0))
  const blocked = risky && confirm.trim() !== task.title.trim()
  const run = async () => {
    setBusy(true)
    try { await api.deleteTeamTask(task.id, { removeWorktrees: removeTrees && (trees?.length ?? 0) > 0, deleteBranches: branches, force: risky }); onDeleted() } catch (cause) { onError(errorText(cause)) } finally { setBusy(false) }
  }
  return <div className="modal-backdrop" onMouseDown={onClose}>
    <form className="clone-dialog team-dialog" onMouseDown={event => event.stopPropagation()} onSubmit={event => { event.preventDefault(); if (!blocked) void run() }}>
      <h2>Delete “{task.title}”?</h2>
      <p>The thread and artifacts are deleted. The agents’ own sessions are kept in each CLI.</p>
      {trees === null ? <p className="settings-note"><LoaderCircle size={13} className="animate-spin" /> Checking worktrees…</p>
        : trees.length > 0 && <>
          <ul className="team-delete-trees">{trees.map(tree => <li key={tree.path}>
            <strong>@{tree.handle}</strong><code>{tree.branch}</code>
            <span className={cn(tree.dirty > 0 && 'warn')}>{tree.dirty > 0 ? `${tree.dirty} uncommitted ${tree.dirty === 1 ? 'file' : 'files'}` : 'clean'}</span>
            <span className={cn(tree.ahead > 0 && 'warn')}>{tree.ahead > 0 ? `${tree.ahead} unmerged ${tree.ahead === 1 ? 'commit' : 'commits'}` : 'merged'}</span>
          </li>)}</ul>
          <label className="team-check"><input type="checkbox" checked={removeTrees} onChange={event => setRemoveTrees(event.target.checked)} />Remove the members’ worktrees</label>
          <label className="team-check"><input type="checkbox" checked={branches} disabled={!removeTrees} onChange={event => setBranches(event.target.checked)} />Delete their branches too</label>
          {risky && <div className="team-error"><CircleAlert size={15} /><span>{dirty > 0 ? `${dirty} uncommitted ${dirty === 1 ? 'file' : 'files'}` : ''}{dirty > 0 && branches && ahead > 0 ? ' and ' : ''}{branches && ahead > 0 ? `${ahead} unmerged ${ahead === 1 ? 'commit' : 'commits'}` : ''} will be lost. Type the task’s name to confirm.</span></div>}
          {risky && <label>Task name<input value={confirm} onChange={event => setConfirm(event.target.value)} placeholder={task.title} aria-label="Type the task name to confirm" /></label>}
        </>}
      <div className="clone-actions"><button type="button" className="button subtle" onClick={onClose}>Cancel</button><button type="submit" className="button danger" disabled={busy || blocked || trees === null}>{busy ? 'Deleting…' : 'Delete task'}</button></div>
    </form>
  </div>
}

/** Every file the task changed, across the project and each member worktree. */
export function ChangesPanel({ task, onError, onSend }: { task: TeamTask; onError: (message: string) => void; onSend: (text: string, to: string[]) => void }) {
  const [files, setFiles] = useState<TeamTaskFile[] | null>(null)
  const [view, setView] = useState<'unified' | 'split'>(() => readStored('neru.diff.view', 'unified'))
  const [reviewer, setReviewer] = useState(task.members[0]?.handle ?? '')
  const changed = task.posts.filter(post => post.changes).length
  const load = useCallback(() => { setFiles(null); void api.teamTaskChanges(task.id).then(setFiles).catch(cause => { setFiles([]); onError(errorText(cause)) }) }, [task.id, onError])
  useEffect(() => { load() }, [load, changed])
  const places = [...new Set((files ?? []).map(file => file.place))]
  const added = (files ?? []).reduce((sum, file) => sum + file.additions, 0)
  const removed = (files ?? []).reduce((sum, file) => sum + file.deletions, 0)
  return <div className="team-panel-body">
    <div className="team-panel-tools"><span>{files === null ? 'Reading changes…' : `${files.length} ${files.length === 1 ? 'file' : 'files'} · +${added} −${removed}`}</span>
      <div className="theme-toggle team-access" role="radiogroup" aria-label="Diff layout">{(['unified', 'split'] as const).map(mode => <button type="button" key={mode} role="radio" aria-checked={view === mode} className={view === mode ? 'active' : ''} onClick={() => { setView(mode); writeStored('neru.diff.view', mode) }}>{mode === 'unified' ? 'Unified' : 'Split'}</button>)}</div>
      <button className="icon-button small" onClick={load} aria-label="Refresh" title="Refresh"><RefreshCw size={13} /></button></div>
    {files?.length === 0 && <p className="empty-small">Nothing changed yet. Turns that edit files show up here as one combined diff.</p>}
    {files && files.length > 0 && task.members.length > 0 && <div className="team-review-all">
      <span>Review everything with</span>
      <select value={reviewer} onChange={event => setReviewer(event.target.value)} aria-label="Reviewer">{task.members.map(member => <option key={member.handle} value={member.handle}>@{member.handle}</option>)}</select>
      <button type="button" className="button subtle small" onClick={() => onSend(`Review all of this task's changes (${files.length} files, +${added} −${removed}${places.length > 1 ? ` across ${places.join(', ')}` : ''}): correctness, risks, missing tests. Write the findings to artifacts/reviews/ (front matter kind: review), most severe first.`, [reviewer])}>Ask</button>
    </div>}
    {places.map(place => <section key={place} className="team-changes-place">
      {places.length > 1 && <h3>{place === 'project' ? 'Project folder' : `@${place}'s worktree`}</h3>}
      {(files ?? []).filter(file => file.place === place).map(file => <FileDiff key={`${place}:${file.path}`} view={view} file={file.path} lines={parseUnifiedDiff(file.diff)} status="complete" collapseOnComplete={false} defaultOpen={(files?.length ?? 0) <= 6} maxHeight={480} language={languageForPath(file.path)} copyText={file.diff} />)}
    </section>)}
  </div>
}

/** The pull request for the task's branch (the project's or a member's), its checks, and "Fix in chat". */
export function PrPanel({ task, onError, onSend }: { task: TeamTask; onError: (message: string) => void; onSend: (text: string, to: string[]) => void }) {
  const sources = [{ id: '', label: 'Project branch' }, ...task.members.filter(member => member.worktree).map(member => ({ id: member.handle, label: `@${member.handle} · ${member.worktree!.branch}` }))]
  const [source, setSource] = useState('')
  const [pr, setPr] = useState<PrStatus | null | undefined>(undefined)
  const [error, setError] = useState('')
  const [fixer, setFixer] = useState(task.members[0]?.handle ?? '')
  const [fixing, setFixing] = useState(false)
  const load = useCallback(() => { setError(''); void api.teamPrStatus(task.id, source || undefined).then(setPr).catch(cause => { setPr(null); setError(errorText(cause)) }) }, [task.id, source])
  useEffect(() => { setPr(undefined); load(); const timer = window.setInterval(load, 30_000); return () => window.clearInterval(timer) }, [load])
  const failing = pr?.checks.filter(check => check.state === 'failure') ?? []
  const fix = async () => {
    setFixing(true)
    try {
      const log = await api.teamFailedLog(task.id, source || undefined)
      onSend(`CI is failing on pull request #${pr?.number} (${failing.map(check => check.name).join(', ')}). Fix it in ${source ? `your worktree (${task.members.find(m => m.handle === source)?.worktree?.branch})` : 'the project'}, run the checks locally, and push.\n\n${log}`, [fixer])
    } catch (cause) { onError(errorText(cause)) } finally { setFixing(false) }
  }
  return <div className="team-panel-body">
    <div className="team-panel-tools"><select value={source} onChange={event => setSource(event.target.value)} aria-label="Branch">{sources.map(item => <option key={item.id} value={item.id}>{item.label}</option>)}</select>
      <button className="icon-button small" onClick={load} aria-label="Refresh" title="Refresh"><RefreshCw size={13} /></button></div>
    {pr === undefined && <p className="empty-small"><LoaderCircle size={13} className="animate-spin" /> Looking for a pull request…</p>}
    {error && <p className={/no git remotes|not a git repository/i.test(error) ? 'empty-small' : 'team-member-error'}>{/no git remotes/i.test(error) ? 'This project has no Git remote yet, so there is no pull request to follow. Add a GitHub remote and push a branch first.' : /not a git repository/i.test(error) ? 'This folder is not a Git repository.' : error}</p>}
    {pr === null && !error && <p className="empty-small">No pull request for this branch yet. Ask a member to push and open one, e.g. “@{task.members[0]?.handle ?? 'claude'} open a pull request for this work”.</p>}
    {pr && <>
      <a className="team-pr-head" href={pr.url} target="_blank" rel="noreferrer"><GitPullRequest size={16} /><span><strong>#{pr.number} {pr.title}</strong><small>{pr.state.toLowerCase()} · {pr.checks.length} {pr.checks.length === 1 ? 'check' : 'checks'}</small></span><ExternalLink size={13} /></a>
      <ul className="team-checks">{pr.checks.map(check => <li key={check.name} className={`is-${check.state}`}>
        {check.state === 'success' ? <CheckCircle2 size={14} /> : check.state === 'failure' ? <XCircle size={14} /> : check.state === 'pending' ? <LoaderCircle size={14} className="animate-spin" /> : <CircleDashed size={14} />}
        {check.url ? <a href={check.url} target="_blank" rel="noreferrer" className="truncate">{check.name}</a> : <span className="truncate">{check.name}</span>}
      </li>)}</ul>
      {failing.length > 0 && <div className="team-review-all"><span>Fix in chat with</span>
        <select value={fixer} onChange={event => setFixer(event.target.value)} aria-label="Who fixes it">{task.members.map(member => <option key={member.handle} value={member.handle}>@{member.handle}</option>)}</select>
        <button type="button" className="button primary small" onClick={() => void fix()} disabled={fixing}>{fixing ? 'Reading logs…' : 'Fix'}</button></div>}
    </>}
  </div>
}

/** What is happening now: working members, the queue, side chats waiting, and smart execution. */
export function ActivityPanel({ task, live, since, routing, onStop, onChange, onError, names }: {
  task: TeamTask; live: Record<string, { text: string; steps: string[]; drawing?: boolean }>; since: Record<string, number>; routing: { from: string; to: string; until: number } | null
  onStop: (handle?: string) => void; onChange: (task: TeamTask) => void; onError: (message: string) => void; names: (kind: string) => string
}) {
  const [now, setNow] = useState(Date.now())
  useEffect(() => { const timer = window.setInterval(() => setNow(Date.now()), 1000); return () => window.clearInterval(timer) }, [])
  const working = task.members.filter(member => member.status === 'working')
  const sides = task.posts.filter((post, index) => post.kind === 'side' && post.author === 'you' && !task.posts.slice(index + 1).some(reply => reply.kind !== 'notice' && post.to.includes(reply.author)))
  const plan = task.execution
  const [executor, setExecutor] = useState(plan.executor || task.members[0]?.handle || '')
  const [reviewer, setReviewer] = useState(plan.reviewer)
  const [rounds, setRounds] = useState(plan.maxRounds || 2)
  const elapsed = (start?: number) => { if (!start) return ''; const s = Math.max(0, Math.round((now - start) / 1000)); return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s` }
  const start = async () => {
    try { onChange(await api.setTeamExecution(task.id, executor, reviewer, rounds)); await api.runTeamExecution(task.id); onChange({ ...task, execution: { executor, reviewer, maxRounds: rounds, running: true } }) } catch (cause) { onError(errorText(cause)) }
  }
  return <div className="team-panel-body">
    <section className="team-activity">
      <h3>Working now</h3>
      {working.length === 0 && <p className="empty-small">Nobody is working. Members show here with their current step while they take a turn.</p>}
      {working.map(member => <div key={member.handle} className="team-activity-row">
        <span className="team-activity-dot" aria-hidden />
        <div><strong>@{member.handle}</strong><span className="truncate">{live[member.handle]?.steps.at(-1) ?? 'Thinking…'}</span></div>
        <time>{elapsed(since[member.handle])}</time>
        <button type="button" className="icon-button small" onClick={() => onStop(member.handle)} aria-label={`Stop @${member.handle}`} title="Stop"><Square size={12} /></button>
      </div>)}
      {routing && <div className="team-activity-row"><span className="team-activity-dot route" aria-hidden /><div><strong>Routing</strong><span>@{routing.from} → @{routing.to} in {Math.max(0, Math.ceil((routing.until - now) / 1000))}s</span></div></div>}
    </section>
    {task.queue.length > 0 && <section className="team-activity"><h3>Queued</h3>{task.queue.map(item => <p key={item.id} className="team-activity-note"><b>{item.to.map(handle => `@${handle}`).join(', ')}</b> {item.text}</p>)}</section>}
    {sides.length > 0 && <section className="team-activity"><h3>Side chats waiting</h3>{sides.map(post => <p key={post.id} className="team-activity-note"><b>@{post.to[0]}</b> {post.text}</p>)}</section>}
    <section className="team-activity team-exec">
      <h3>Smart execution</h3>
      <p className="empty-small">An executor works through the open tickets in artifacts/tickets/ in order; a reviewer checks each one and can send it back.</p>
      <label className="team-field">Executor<select value={executor} onChange={event => setExecutor(event.target.value)} disabled={plan.running}>{task.members.map(member => <option key={member.handle} value={member.handle}>@{member.handle} · {names(member.kind)}</option>)}</select></label>
      <label className="team-field">Reviewer<select value={reviewer} onChange={event => setReviewer(event.target.value)} disabled={plan.running}><option value="">No review</option>{task.members.filter(member => member.handle !== executor).map(member => <option key={member.handle} value={member.handle}>@{member.handle} · {names(member.kind)}</option>)}</select></label>
      <label className="team-field">Rounds per ticket<input type="number" min={0} max={5} value={rounds} disabled={plan.running} onChange={event => setRounds(Math.max(0, Math.min(5, Number(event.target.value) || 0)))} /></label>
      {plan.running
        ? <button type="button" className="button subtle" onClick={() => void api.stopTeamExecution(task.id).then(() => onChange({ ...task, execution: { ...plan, running: false } })).catch(cause => onError(errorText(cause)))}><Square size={13} />Stop after this turn</button>
        : <button type="button" className="button primary" onClick={() => void start()} disabled={!executor}><Play size={13} />Run the tickets</button>}
      {working.length > 0 && <button type="button" className="button subtle small" onClick={() => onStop()}><Square size={12} />Stop everyone</button>}
    </section>
  </div>
}

function Bars({ values, label }: { values: number[]; label: string }) {
  const max = Math.max(1, ...values)
  return <svg className="team-bars" viewBox={`0 0 ${values.length * 8} 40`} preserveAspectRatio="none" role="img" aria-label={label}>
    {values.map((value, index) => <rect key={index} x={index * 8 + 1} y={40 - (value / max) * 38} width={6} height={Math.max(1, (value / max) * 38)} rx={1.5}><title>{tokens(value)}</title></rect>)}
  </svg>
}

/** Tokens, cost and limits per member, the last turns as bars, and a CSV export. */
export function UsageDashboard({ task, names, onError, compact }: { task: TeamTask; names: (kind: string) => string; onError: (message: string) => void; compact?: boolean }) {
  const total = task.members.reduce((sum, member) => sum + member.usage.cost, 0)
  const tokensTotal = task.members.reduce((sum, member) => sum + member.usage.input + member.usage.output, 0)
  const exportCsv = async () => {
    const destination = await saveDialog({ title: 'Export usage as CSV', defaultPath: `${task.title.replace(/[^\w-]+/g, '-').slice(0, 40) || 'team'}-usage.csv`, filters: [{ name: 'CSV', extensions: ['csv'] }] })
    if (!destination) return
    try { const rows = await api.exportTeamUsage(task.id, destination); window.alert(`Exported ${rows} ${rows === 1 ? 'turn' : 'turns'}.`) } catch (cause) { onError(errorText(cause)) }
  }
  return <div className={cn('team-panel-body', compact && 'compact')}>
    <div className="team-usage-summary">
      <div><strong>{tokens(tokensTotal)}</strong><span>tokens</span></div>
      <div><strong>{total > 0 ? `$${total.toFixed(2)}` : '—'}</strong><span>reported cost</span></div>
      <div><strong>{task.members.reduce((sum, member) => sum + member.usage.turns, 0)}</strong><span>turns</span></div>
      {!compact && <button type="button" className="icon-button small" onClick={() => void exportCsv()} aria-label="Export CSV" title="Export every turn as CSV"><Download size={14} /></button>}
    </div>
    {task.members.map(member => {
      const history = member.usage.history ?? []
      return <div className="team-usage" key={member.handle}>
        <div className="team-usage-who"><strong>@{member.handle}</strong><span>{names(member.kind)} · {member.usage.turns} {member.usage.turns === 1 ? 'turn' : 'turns'} · {tokens(member.usage.input)} in · {tokens(member.usage.output)} out{member.usage.cost > 0 ? ` · $${member.usage.cost.toFixed(2)}` : ''}</span></div>
        {member.usage.limits.map(limit => <div className="team-limit" key={limit.label}>
          <span>{limit.label}</span>
          <div className="team-limit-bar" role="meter" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(limit.used)} aria-label={`${limit.label} limit used`}><i style={{ width: `${Math.min(100, limit.used)}%` }} className={limit.used >= 90 ? 'high' : limit.used >= 70 ? 'mid' : ''} /></div>
          <small>{Math.round(limit.used)}%{limit.resetsAt ? ` · resets ${new Date(limit.resetsAt * 1000).toLocaleString(undefined, { weekday: 'short', hour: 'numeric', minute: '2-digit' })}` : ''}</small>
        </div>)}
        {!compact && history.length > 1 && <Bars values={history.slice(-30).map(spend => spend.input + spend.output)} label={`Tokens in @${member.handle}'s last ${Math.min(30, history.length)} turns`} />}
      </div>
    })}
    {!compact && <p className="empty-small">Claude Code and Codex report their 5-hour and weekly windows; other agents report tokens and cost. Open this any time with Ctrl+Shift+U.</p>}
  </div>
}

/** The compact limit readout for the task header: the most-used window across the team. */
export function usageReading(task: TeamTask) {
  const limits = task.members.flatMap(member => member.usage.limits.map(limit => ({ ...limit, handle: member.handle })))
  const top = limits.sort((a, b) => b.used - a.used)[0]
  return top ? { text: `@${top.handle} ${top.label.toLowerCase()} ${Math.round(top.used)}%`, level: top.used >= 90 ? 'high' : top.used >= 70 ? 'mid' : '' } : null
}

/** First steps for a new Team user, ticked off as they happen. */
export function Checklist({ agents, tasks, toured, onAction, onDismiss }: { agents: TeamAgent[]; tasks: TeamTaskSummary[]; toured: boolean; onAction: (id: 'agents' | 'import' | 'task' | 'tour') => void; onDismiss: () => void }) {
  const items: { id: 'agents' | 'import' | 'task' | 'tour'; title: string; text: string; done: boolean; action: string }[] = [
    { id: 'agents', title: 'Sign in to an agent', text: 'Any of Claude Code, Codex, Gemini, Cursor or OpenCode.', done: agents.some(agent => agent.path && agent.signedIn), action: 'Agents settings' },
    { id: 'import', title: 'Bring your history', text: 'Past chats become tasks the agents can continue.', done: tasks.some(task => task.labels.includes('Imported')), action: 'Import' },
    { id: 'task', title: 'Start a task with two agents', text: 'They share one thread and hand work to each other.', done: tasks.some(task => !task.labels.includes('Imported')), action: 'Start' },
    { id: 'tour', title: 'Take the tour', text: 'A one-minute look at every part of Team.', done: toured, action: 'Show me' },
  ]
  const done = items.filter(item => item.done).length
  if (done === items.length) return null
  return <section className="team-checklist" aria-label="Getting started with Team">
    <header><strong>Get started</strong><span>{done} of {items.length}</span><div className="team-checklist-bar" aria-hidden><i style={{ width: `${(done / items.length) * 100}%` }} /></div><button type="button" className="icon-button small" onClick={onDismiss} aria-label="Hide the checklist" title="Hide"><X size={13} /></button></header>
    <ol>{items.map(item => <li key={item.id} className={cn(item.done && 'done')}>
      <span className="team-checklist-mark" aria-hidden>{item.done ? <Check size={12} strokeWidth={3} /> : null}</span>
      <div><strong>{item.title}</strong><small>{item.text}</small></div>
      {!item.done && <button type="button" className="button subtle small" onClick={() => onAction(item.id)}>{item.action}</button>}
    </li>)}</ol>
  </section>
}

/** Ready-made first tasks: a title and the first message, with the skill that fits. */
export const QUICKSTARTS: { id: string; icon: LucideIcon; title: string; text: string; message: (title: string) => string }[] = [
  { id: 'plan', icon: Sparkles, title: 'Plan a feature', text: 'One agent writes the spec, another reviews it.', message: title => `/plan ${title}` },
  { id: 'review', icon: Shield, title: 'Review my changes', text: 'Two agents review the current diff from different angles.', message: () => '/review' },
  { id: 'bug', icon: Bug, title: 'Fix a bug together', text: 'One investigates and fixes, the other verifies.', message: title => `Find and fix this bug: ${title}. When it is fixed, hand it to a teammate to verify.` },
  { id: 'debate', icon: Zap, title: 'Debate an approach', text: 'Every agent argues a position, then you decide.', message: title => `/debate ${title}` },
]

export function Quickstart({ onPick }: { onPick: (id: string) => void }) {
  return <div className="team-quickstart" role="list" aria-label="Start from a template">{QUICKSTARTS.map(item => <button type="button" role="listitem" key={item.id} className="team-quick" onClick={() => onPick(item.id)}>
    <span className="team-quick-icon"><item.icon size={16} /></span><span><strong>{item.title}</strong><small>{item.text}</small></span>
  </button>)}</div>
}

export function Pill({ children, tone }: { children: ReactNode; tone?: string }) {
  return <span className={cn('team-pill', tone && `is-${tone}`)}>{children}</span>
}

/** A column the user can drag wider or narrower, remembered across launches. `edge` is the side
 * the handle sits on: a left column grows to the right, a right one to the left. */
export function useDragWidth(key: string, initial: number, min: number, max: number, edge: 'right' | 'left', share = 1) {
  const [width, setWidth] = useState(() => Math.min(max, Math.max(min, readStored(key, initial))))
  const [dragging, setDragging] = useState(false)
  const keep = (value: number) => { const next = Math.round(Math.min(max, Math.max(min, value))); setWidth(next); writeStored(key, next); return next }
  const handle = {
    role: 'separator' as const,
    'aria-orientation': 'vertical' as const,
    'aria-valuemin': min,
    'aria-valuemax': max,
    'aria-valuenow': width,
    tabIndex: 0,
    className: cn('team-resizer', `edge-${edge}`, dragging && 'dragging'),
    title: 'Drag to resize, double-click to reset',
    onPointerDown: (event: React.PointerEvent<HTMLDivElement>) => {
      event.preventDefault()
      const startX = event.clientX
      // Start from the width on screen, and stop where the layout stops it (a share of the view),
      // so the handle never drags through a dead zone.
      const column = event.currentTarget.parentElement
      const view = column?.parentElement?.getBoundingClientRect().width ?? Infinity
      const start = column?.getBoundingClientRect().width ?? width
      const limit = Math.max(min, Math.min(max, Math.floor(view * share)))
      setDragging(true)
      document.body.classList.add('col-resizing')
      const move = (moving: PointerEvent) => keep(Math.min(limit, start + (edge === 'right' ? moving.clientX - startX : startX - moving.clientX)))
      const up = () => { setDragging(false); document.body.classList.remove('col-resizing'); window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up) }
      window.addEventListener('pointermove', move)
      window.addEventListener('pointerup', up)
    },
    onDoubleClick: () => keep(initial),
    onKeyDown: (event: React.KeyboardEvent<HTMLDivElement>) => {
      const step = event.shiftKey ? 48 : 16
      const grow = edge === 'right' ? 'ArrowRight' : 'ArrowLeft'
      const shrink = edge === 'right' ? 'ArrowLeft' : 'ArrowRight'
      if (event.key === grow) { event.preventDefault(); keep(width + step) }
      else if (event.key === shrink) { event.preventDefault(); keep(width - step) }
      else if (event.key === 'Home') { event.preventDefault(); keep(initial) }
    },
  }
  return { width, dragging, handle, set: keep }
}

/** Models a member's CLI offers. Read again after a minute, so a newly released model shows up. */
const modelCache = new Map<string, { at: number; list: Promise<AgentModel[]> }>()
export function useAgentModels(kind: string) {
  const [models, setModels] = useState<AgentModel[]>([])
  useEffect(() => {
    if (kind === 'neru' || kind.startsWith('custom:')) return
    const cached = modelCache.get(kind)
    if (!cached || Date.now() - cached.at > 60_000) modelCache.set(kind, { at: Date.now(), list: api.listAgentModels(kind).catch(() => []) })
    let live = true
    void modelCache.get(kind)!.list.then(list => { if (live) setModels(list) })
    return () => { live = false }
  }, [kind])
  return models
}

/** Replays hand-offs in order: each step highlights its edge and shows the message. */
export function useReplay(steps: TeamPost[], onStep: (post: TeamPost) => void) {
  const [index, setIndex] = useState(-1)
  const [playing, setPlaying] = useState(false)
  useEffect(() => {
    if (!playing) return
    if (index >= steps.length - 1) { setPlaying(false); return }
    const timer = window.setTimeout(() => setIndex(value => value + 1), index < 0 ? 0 : 1400)
    return () => window.clearTimeout(timer)
  }, [playing, index, steps.length])
  useEffect(() => { if (index >= 0 && steps[index]) onStep(steps[index]) }, [index]) // eslint-disable-line react-hooks/exhaustive-deps
  return useMemo(() => ({ index, playing, play: () => { if (index >= steps.length - 1) setIndex(-1); setPlaying(true) }, pause: () => setPlaying(false), seek: (value: number) => { setPlaying(false); setIndex(value) } }), [index, playing, steps.length])
}

export function ReplayBar({ replay, total }: { replay: ReturnType<typeof useReplay>; total: number }) {
  if (total === 0) return null
  return <div className="team-replay">
    <button type="button" className="icon-button small" onClick={replay.playing ? replay.pause : replay.play} aria-label={replay.playing ? 'Pause replay' : 'Replay hand-offs'} title={replay.playing ? 'Pause' : 'Replay hand-offs'}>{replay.playing ? <Pause size={14} /> : <Play size={14} />}</button>
    <input type="range" min={-1} max={total - 1} value={replay.index} onChange={event => replay.seek(Number(event.target.value))} aria-label="Replay position" />
    <span>{replay.index < 0 ? `${total} hand-offs` : `${replay.index + 1} / ${total}`}</span>
  </div>
}

