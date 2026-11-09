import { Fragment, useEffect, useMemo, useState, type ReactNode } from 'react'
import { isTauri } from '@tauri-apps/api/core'
import { ChevronRight, CircleCheck, CircleX, Clock, LoaderCircle, Moon, PiggyBank, Sun, TriangleAlert, X } from '@/components/icons'
import { api } from '../../api'
import type { AgentMode, Digest, DigestItem, QueuedTask } from '../../types'
import { cn } from '@/lib/utils'
import { dollars, referenceCost, REFERENCE } from '@/lib/savings'
import { t } from '../../i18n'
import { Select } from '../ui/Select'

const SEEN = 'neru.digest.seen'
/** Since the last "Mark as read", or the last 12 hours the first time. */
export const digestSince = () => { try { return Number(localStorage.getItem(SEEN)) || Date.now() - 12 * 3600_000 } catch { return Date.now() - 12 * 3600_000 } }
const markSeen = (at: number) => { try { localStorage.setItem(SEEN, String(at)) } catch { /* storage unavailable */ } }

const baseName = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path
const time = (ms: number) => new Date(ms).toLocaleString(undefined, { weekday: 'short', hour: '2-digit', minute: '2-digit' })
/** A translated sentence with elements put in place of its {placeholders}. */
const rich = (text: string, parts: Record<string, ReactNode>) => text.split(/\{(\w+)\}/).map((piece, i) => i % 2 ? <Fragment key={i}>{parts[piece]}</Fragment> : piece)
const MODES: { value: AgentMode; label: string }[] = [
  { value: 'accept_edits', label: 'Edit files on its own' },
  { value: 'auto', label: 'Auto: routine work runs, risky steps ask' },
  { value: 'manual', label: 'Ask me before every step' },
]

/**
 * Tasks: queue work for tonight (after free plans reset their daily quota) and see what happened while you were
 * away. Queued tasks run one at a time in their own session while Neru is open.
 */
/** A reply's opening as plain prose: code blocks, widget sources and Markdown marks dropped. */
function plainSummary(text?: string | null) {
  if (!text) return ''
  return text.replace(/```[\s\S]*?(```|$)/g, ' ').replace(/[`*#>]+/g, '').replace(/\s+/g, ' ').trim()
}

export function TasksDialog({ open, view, projects, onClose, onOpenSession, onError }: {
  open: boolean
  view: 'queue' | 'digest'
  projects: string[]
  onClose: () => void
  onOpenSession: (projectPath: string, id: string) => void
  onError: (message: string) => void
}) {
  const [queue, setQueue] = useState<QueuedTask[]>([])
  const [digest, setDigest] = useState<Digest | null>(null)
  const [since, setSince] = useState(digestSince)
  const [tonight, setTonight] = useState(0)
  const [prompt, setPrompt] = useState('')
  const [project, setProject] = useState('')
  const [mode, setMode] = useState<AgentMode>('accept_edits')
  const [when, setWhen] = useState<'tonight' | 'hour' | 'now'>('tonight')
  const [offline, setOffline] = useState(false)
  const [repeat, setRepeat] = useState<'' | 'daily' | 'weekly'>('')
  const [adding, setAdding] = useState(false)

  useEffect(() => {
    if (!open || !isTauri()) return
    void api.queueList().then(setQueue).catch(cause => onError(String(cause)))
    void api.queueTonight().then(setTonight).catch(() => undefined)
    void api.digest(since).then(setDigest).catch(cause => onError(String(cause)))
    const unlisten = api.onQueue(setQueue)
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') onClose() }
    window.addEventListener('keydown', onKey)
    return () => { void unlisten.then(stop => stop()); window.removeEventListener('keydown', onKey) }
  }, [open, since, onClose, onError])
  useEffect(() => { if (!project && projects[0]) setProject(projects[0]) }, [projects, project])

  const upcoming = useMemo(() => queue.filter(task => task.status === 'queued' || task.status === 'running').sort((a, b) => a.at - b.at), [queue])
  const past = useMemo(() => queue.filter(task => task.status !== 'queued' && task.status !== 'running').sort((a, b) => b.at - a.at), [queue])
  if (!open) return null

  const add = async () => {
    if (!prompt.trim() || !project) return
    setAdding(true)
    try {
      const at = when === 'now' ? 0 : when === 'tonight' ? tonight : Date.now() + 3600_000
      await api.queueAdd(project, prompt.trim(), at, mode, undefined, offline, repeat || undefined)
      setPrompt('')
    } catch (cause) { onError(String(cause)) } finally { setAdding(false) }
  }
  const items = digest?.items ?? []
  const saved = digest ? referenceCost(digest.usage.free) : 0

  const digestCard = (item: DigestItem) => {
    const facts = [item.edits ? (item.edits === 1 ? t('1 change') : t('{n} changes', { n: item.edits })) : '', item.commands ? (item.commands === 1 ? t('1 command') : t('{n} commands', { n: item.commands })) : '', item.failedCommands ? t('{n} failed', { n: item.failedCommands }) : ''].filter(Boolean).join(' · ')
    const label = { done: t('Done'), waiting: t('Needs you'), failed: t('Stopped'), running: t('Running') }[item.status]
    const Mark = { done: CircleCheck, waiting: TriangleAlert, failed: CircleX, running: LoaderCircle }[item.status]
    const summary = plainSummary(item.summary)
    return <button key={item.sessionId} type="button" className={cn('task-card digest-card', item.status)} onClick={() => { onOpenSession(item.projectPath, item.sessionId); onClose() }}>
      <span className="digest-mark" aria-hidden><Mark size={16} className={item.status === 'running' ? 'animate-spin' : undefined} /></span>
      <span className="digest-main">
        <span className="task-card-head"><strong className="truncate">{item.title}</strong><span className={cn('task-state', item.status)}>{label}</span></span>
        <span className="task-meta">{[item.project, item.queued ? t('from your queue') : '', facts].filter(Boolean).join(' · ')}</span>
        {summary && <span className="task-summary">{summary}</span>}
      </span>
      <ChevronRight size={16} className="digest-go" aria-hidden />
    </button>
  }
  const taskCard = (task: QueuedTask) => {
    const label = { queued: t('Queued'), running: t('Running'), waiting: t('Needs you'), done: t('Done'), failed: t('Failed'), canceled: t('Canceled') }[task.status]
    return <div key={task.id} className={cn('task-card', task.status)}>
      <span className="task-card-head"><strong>{task.title}</strong><span className={cn('task-state', task.status)}>{task.status === 'running' && <LoaderCircle size={11} className="animate-spin" aria-hidden />}{label}</span></span>
      <span className="task-meta">{baseName(task.projectPath)} · {task.status === 'queued' ? t('starts {time}', { time: time(task.at) }) : time(task.at)}{task.repeat ? ` · ${task.repeat === 'daily' ? t('every day') : t('every week')}` : ''}{task.offline ? ` · ${t('even when Neru is closed')}` : ''}{task.images ? ` · ${task.images === 1 ? t('1 photo') : t('{n} photos', { n: task.images })}` : ''}</span>
      {task.error && <span className="task-meta danger">{task.error}</span>}
      <span className="task-actions">
        {task.status === 'queued' && <button type="button" className="button subtle small" onClick={() => void api.queueCancel(task.id).catch(cause => onError(String(cause)))}>{t('Cancel')}</button>}
        {task.sessionId && <button type="button" className="button subtle small" onClick={() => { onOpenSession(task.projectPath, task.sessionId!); onClose() }}>{t('Open session')}</button>}
      </span>
    </div>
  }

  return <div className="whats-new-backdrop" onMouseDown={onClose}>
    <section className="whats-new tasks-dialog" role="dialog" aria-modal="true" aria-labelledby="tasks-title" onMouseDown={event => event.stopPropagation()}>
      <header>
        <span className="whats-new-eyebrow">{view === 'digest' ? <><Sun size={14} /> {t('While you were away')}</> : <><Clock size={14} /> {t('Tasks')}</>}</span>
        <button type="button" className="icon-button" onClick={onClose} aria-label={t('Close')}><X size={16} /></button>
      </header>
      <div className="whats-new-body">
        <h2 id="tasks-title">{view === 'queue' && !items.length ? t('Queue work for later') : !digest ? t('While you were away') : items.length ? (items.length === 1 ? t('1 session moved since {time}', { time: time(since) }) : t('{n} sessions moved since {time}', { n: items.length, time: time(since) })) : t('Quiet since {time}', { time: time(since) })}</h2>
        {digest && items.some(item => item.status === 'waiting') && <p className="task-alert"><TriangleAlert size={14} aria-hidden /> {items.filter(item => item.status === 'waiting').length === 1 ? t('1 session waiting for your approval') : t('{n} sessions waiting for your approval', { n: items.filter(item => item.status === 'waiting').length })}</p>}
        <div className="task-list">{items.map(digestCard)}</div>
        {digest && <p className="task-savings"><PiggyBank size={15} aria-hidden /> {rich(t('Spent: {amount}. The same work on {model} would have cost about {cost} so far.'), { amount: <strong>$0.00</strong>, model: REFERENCE.name, cost: dollars(saved) })}{digest.usage.paid.replies ? ` ${digest.usage.paid.replies === 1 ? t('1 reply used your paid keys.') : t('{n} replies used your paid keys.', { n: digest.usage.paid.replies })}` : ''}</p>}
        {items.length > 0 && <button type="button" className="button subtle small" onClick={() => { const now = Date.now(); markSeen(now); setSince(now) }}><CircleCheck size={14} /> {t('Mark as read')}</button>}

        <div className="whats-new-part">
          <h3><Moon size={14} aria-hidden /> {t('Queue a task')}</h3>
          <p className="task-meta">{t('Big jobs start after free plans reset their daily quota, one at a time, and switch models when one runs out. Keep Neru open.')}</p>
          <form className="task-form" onSubmit={event => { event.preventDefault(); void add() }}>
            <textarea rows={3} value={prompt} onChange={event => setPrompt(event.target.value)} placeholder={t('Write tests for the billing module and fix what they find')} aria-label={t('Task')} />
            <div className="task-form-row">
              <Select className="settings-select" value={project} onChange={event => setProject(event.target.value)} aria-label={t('Project')}>{projects.length === 0 && <option value="">{t('Open a project first')}</option>}{projects.map(path => <option key={path} value={path}>{baseName(path)}</option>)}</Select>
              <Select className="settings-select" value={mode} onChange={event => setMode(event.target.value as AgentMode)} aria-label={t('What Neru may do')}>{MODES.map(item => <option key={item.value} value={item.value}>{t(item.label)}</option>)}</Select>
              <Select className="settings-select" value={when} onChange={event => setWhen(event.target.value as typeof when)} aria-label={t('When')}>
                <option value="tonight">{t('Tonight')}{tonight ? `, ${new Date(tonight).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' })}` : ''}</option>
                <option value="hour">{t('In an hour')}</option>
                <option value="now">{t('Now')}</option>
              </Select>
              <button type="submit" className="button primary" disabled={adding || !prompt.trim() || !project}>{adding ? t('Adding…') : t('Queue')}</button>
            </div>
            <div className="task-form-row">
              <Select className="settings-select" value={repeat} onChange={event => setRepeat(event.target.value as typeof repeat)} aria-label={t('Repeat')}>
                <option value="">{t('Once')}</option>
                <option value="daily">{t('Every day')}</option>
                <option value="weekly">{t('Every week')}</option>
              </Select>
              <label className="task-meta"><input type="checkbox" checked={offline} onChange={event => setOffline(event.target.checked)} /> {t('Run even when Neru is closed')}</label>
            </div>
            {offline && <p className="task-meta">{t('Neru adds an entry to the system scheduler (Task Scheduler, launchd or cron) that runs neru -p with this task. Commands that need approval are skipped.')}</p>}
            {mode === 'manual' && when !== 'now' && <p className="task-meta">{t('Ask before every step stops at the first change. Approve it from your phone, or pick Edit files.')}</p>}
          </form>
        </div>
        <div className="whats-new-part">
          <h3>{t('Up next')}</h3>
          {upcoming.length ? <div className="task-list">{upcoming.map(taskCard)}</div> : <p className="task-meta">{t('Nothing queued.')}</p>}
          {past.length > 0 && <><h3 className="task-subhead">{t('This week')}</h3><div className="task-list">{past.map(taskCard)}</div></>}
        </div>
      </div>
    </section>
  </div>
}
