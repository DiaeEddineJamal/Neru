import { useCallback, useEffect, useState, type ReactNode } from 'react'
import { BookOpen, Check, ChevronRight, Circle, ExternalLink, Plug, RotateCw, Sparkles, Wrench } from '@/components/icons'
import { api } from '../../api'
import type { ContextUsage, McpServer, OutputStyle, SkillView, TaskReport, Todo, TrustStatus } from '../../types'
import { cn } from '@/lib/utils'
import { t } from '../../i18n'
import './SessionPanel.css'
import { Select } from '../ui/Select'

// The session panel beside a Code session, like Claude Code's: what fills the context window, the task
// list, connected MCP servers and their tools, memory files, skills, the output style and project hooks.

const PARTS: Record<string, { label: string; hint: string }> = {
  system: { label: 'System prompt', hint: 'Instructions, memory files, your profile and the project map' },
  tools: { label: 'Tool definitions', hint: 'Descriptions of the tools Neru can call, sent with every request' },
  messages: { label: 'Messages', hint: 'What you and Neru wrote' },
  results: { label: 'Tool results', hint: 'Files read, command output and search results' },
}

export const tokenLabel = (tokens: number) => tokens >= 1_000_000 ? `${(tokens / 1_000_000).toFixed(1).replace(/\.0$/, '')}M` : tokens >= 1_000 ? `${(tokens / 1_000).toFixed(1).replace(/\.0$/, '')}k` : String(tokens)

function Section({ id, title, count, children, defaultOpen = true, action }: { id: string; title: string; count?: number | string; children: ReactNode; defaultOpen?: boolean; action?: ReactNode }) {
  const key = `neru.sessionPanel.${id}`
  const [open, setOpen] = useState(() => { try { const saved = localStorage.getItem(key); return saved === null ? defaultOpen : saved === '1' } catch { return defaultOpen } })
  const toggle = () => setOpen(value => { try { localStorage.setItem(key, value ? '0' : '1') } catch { /* storage unavailable */ } return !value })
  return <section className={cn('sp-section', open && 'is-open')}>
    <div className="sp-section-head">
      <button type="button" className="sp-section-toggle" aria-expanded={open} onClick={toggle}><ChevronRight size={13} className="sp-chevron" aria-hidden />{t(title)}{count !== undefined && <span className="sp-count">{count}</span>}</button>
      {action}
    </div>
    {open && <div className="sp-section-body">{children}</div>}
  </section>
}

function ContextSection({ context, onCompact, canCompact }: { context: ContextUsage | null; onCompact: () => void; canCompact: boolean }) {
  if (!context) return <p className="sp-empty">{t('Send a message to see how the context fills up.')}</p>
  const limit = context.limit || context.window
  const percent = limit ? Math.min(100, Math.round(context.used / limit * 100)) : 0
  const parts = (context.breakdown ?? []).filter(part => part.tokens > 0)
  return <div className="sp-context">
    <div className="sp-context-total"><strong>{tokenLabel(context.used)}</strong><span>{t('of {limit} tokens · {percent}%', { limit: tokenLabel(limit), percent })}</span></div>
    <div className="sp-bar" role="img" aria-label={t('{percent}% of the context window used', { percent })}>
      {parts.map(part => <i key={part.kind} className={`sp-part-${part.kind}`} style={{ width: `${limit ? part.tokens / limit * 100 : 0}%` }} />)}
    </div>
    <ul className="sp-legend">
      {parts.map(part => <li key={part.kind} title={t(PARTS[part.kind]?.hint ?? '')}><i className={`sp-part-${part.kind}`} aria-hidden />{t(PARTS[part.kind]?.label ?? part.kind)}<span>{tokenLabel(part.tokens)}</span></li>)}
      <li><i className="sp-part-free" aria-hidden />{t('Free space')}<span>{tokenLabel(Math.max(0, limit - context.used))}</span></li>
    </ul>
    {limit < context.window && <p className="sp-note">{t('This key allows {limit} per request, less than the model’s {window} window.', { limit: tokenLabel(limit), window: tokenLabel(context.window) })}</p>}
    {!context.measured && <p className="sp-note">{t('Estimated until the provider reports the real count.')}</p>}
    {context.quotas?.filter(quota => quota.remaining !== null).map(quota => <p key={quota.label} className="sp-note">{quota.label}: {t('{n} left', { n: quota.remaining ?? 0 })}{quota.resetsIn ? ` · ${t('resets in {time}', { time: quota.resetsIn })}` : ''}</p>)}
    <button type="button" className="button subtle small" onClick={onCompact} disabled={!canCompact}>{t('Compact now')}</button>
  </div>
}

const seconds = (ms: number) => ms >= 60_000 ? `${Math.floor(ms / 60_000)}m ${Math.round(ms % 60_000 / 1000)}s` : `${Math.round(ms / 1000)}s`

/** What the running (or last) task cost. Free plans run out of requests first, so requests lead. */
export function TaskSection({ report }: { report: TaskReport | null }) {
  const task = report?.current ?? report?.last
  if (!task) return <p className="sp-empty">{t('Send a message to see how many requests it takes.')}</p>
  const running = Boolean(report?.current)
  const typical = report?.typical && report.typical.tasks >= 3 ? report.typical : null
  const models = Object.entries(task.models).sort((a, b) => b[1] - a[1])
  const rows: [string, string, string?][] = [
    ['Tool rounds', String(task.rounds), t('{n} tool calls', { n: task.toolCalls })],
    ...(task.failedEdits ? [['Failed edits', String(task.failedEdits), 'Edits whose text did not match the file; each costs another request'] as [string, string, string]] : []),
    ...(task.parseFailures ? [['Unreadable tool calls', String(task.parseFailures)] as [string, string]] : []),
    ...(task.retries ? [['Retries', String(task.retries), 'Sent again after a dropped connection or a server error'] as [string, string, string]] : []),
    ...(task.switches ? [['Model switches', String(task.switches)] as [string, string]] : []),
    ...(task.waitedMs >= 1000 ? [['Waited on rate limits', seconds(task.waitedMs)] as [string, string]] : []),
    ['Tokens in / out', `${tokenLabel(task.inputTokens)} / ${tokenLabel(task.outputTokens)}`, task.cachedTokens ? t('{n} read from the provider’s cache', { n: tokenLabel(task.cachedTokens) }) : undefined],
  ]
  return <div className="sp-context">
    <div className="sp-context-total">
      <strong>{t(task.requests === 1 ? '{n} request' : '{n} requests', { n: task.requests })}</strong>
      <span>{running ? t('this task so far') : task.outcome === 'error' ? t('last task, ended with an error') : task.outcome === 'stopped' ? t('last task, stopped') : t('last task')}{typical ? ` · ${t('typical {n}', { n: typical.requests })}` : ''}</span>
    </div>
    <ul className="sp-legend">
      {rows.map(([label, value, hint]) => <li key={label} title={hint ? t(hint) : undefined}>{t(label)}<span>{value}</span></li>)}
    </ul>
    {models.length > 1 && <><p className="sp-subhead">{t('Requests by model')}</p><ul className="sp-legend">{models.map(([model, count]) => <li key={model} title={model}><span className="sp-model">{model.split('/').slice(1).join('/') || model}</span><span>{count}</span></li>)}</ul></>}
  </div>
}

const STATUS: Record<McpServer['status'], string> = { connected: 'Connected', starting: 'Starting', error: 'Error', off: 'Off', needs_auth: 'Needs sign-in', needs_trust: 'Needs trust' }

function ServerRow({ server }: { server: McpServer }) {
  const [open, setOpen] = useState(false)
  return <li className="sp-server">
    <button type="button" className="sp-row" aria-expanded={open} onClick={() => setOpen(value => !value)} disabled={!server.tools.length}>
      <span className={cn('sp-dot', `is-${server.status}`)} aria-hidden />
      <span className="sp-row-name">{server.name}</span>
      <span className="sp-row-meta">{server.status === 'connected' ? t('{n} tools', { n: server.tools.length }) : t(STATUS[server.status])}</span>
    </button>
    {server.error && <p className="sp-note is-error">{server.error}</p>}
    {open && <ul className="sp-tools">{server.tools.map(tool => <li key={tool.name} title={tool.description}><Wrench size={11} aria-hidden />{tool.name}{tool.readOnly && <span className="sp-tag">{t('read-only')}</span>}</li>)}</ul>}
  </li>
}

export function SessionPanel({ sessionId, context, todos, canCompact, onCompact, onOpenSettings, onError }: {
  sessionId?: string | null; context: ContextUsage | null; todos: Todo[]; canCompact: boolean; onCompact: () => void
  onOpenSettings: (tab: 'connectors' | 'skills' | 'styles' | 'privacy') => void; onError: (message: string) => void
}) {
  const [servers, setServers] = useState<McpServer[]>([])
  const [memory, setMemory] = useState<{ project: string | null; user: string } | null>(null)
  const [facts, setFacts] = useState<string[]>([])
  const [skills, setSkills] = useState<SkillView[]>([])
  const [styles, setStyles] = useState<OutputStyle[]>([])
  const [trust, setTrust] = useState<TrustStatus | null>(null)
  const [cost, setCost] = useState<{ sessionId: string; report: TaskReport } | null>(null)

  const load = useCallback(() => {
    const quiet = <T,>(promise: Promise<T>, set: (value: T) => void) => void promise.then(set).catch(() => undefined)
    quiet(api.mcpServers(), setServers)
    quiet(api.memoryFiles(), setMemory)
    quiet(api.projectMemoryFacts(), setFacts)
    quiet(api.listSkills(), setSkills)
    quiet(api.outputStyles(), setStyles)
    quiet(api.projectTrustStatus(), setTrust)
  }, [])
  useEffect(load, [load])
  // The context changes after every model reply, which is when the task's counts move too.
  useEffect(() => {
    if (!sessionId) return
    let live = true
    void api.taskStats(sessionId).then(report => { if (live) setCost({ sessionId, report }) }).catch(() => undefined)
    return () => { live = false }
  }, [sessionId, context])

  const openMemory = (scope: 'project' | 'user') => void api.openMemoryFile(scope).then(() => load()).catch(cause => onError(cause instanceof Error ? cause.message : String(cause)))
  const chooseStyle = (name: string) => void api.setOutputStyle(name).then(setStyles).catch(cause => onError(cause instanceof Error ? cause.message : String(cause)))
  const enabledSkills = skills.filter(skill => skill.enabled)
  const connected = servers.filter(server => server.status === 'connected')
  const done = todos.filter(todo => todo.status === 'completed').length
  const current = styles.find(style => style.current)

  return <div className="session-panel">
    <div className="sp-toolbar"><button type="button" className="dock-button" onClick={load} aria-label={t('Refresh')} title={t('Refresh')}><RotateCw size={14} /></button></div>

    <Section id="context" title="Context">
      <ContextSection context={context} onCompact={onCompact} canCompact={canCompact} />
    </Section>

    <Section id="cost" title="Task cost">
      <TaskSection report={cost && cost.sessionId === sessionId ? cost.report : null} />
    </Section>

    {todos.length > 0 && <Section id="tasks" title="Tasks" count={`${done}/${todos.length}`}>
      <ul className="sp-todos">{todos.map((todo, index) => <li key={index} className={`is-${todo.status}`}>{todo.status === 'completed' ? <Check size={13} /> : <Circle size={13} weight={todo.status === 'in_progress' ? 'fill' : 'regular'} />}<span>{todo.content}</span></li>)}</ul>
    </Section>}

    <Section id="mcp" title="MCP servers" count={servers.length ? `${connected.length}/${servers.length}` : undefined}
      action={<button type="button" className="sp-link" onClick={() => onOpenSettings('connectors')}><Plug size={12} />{t('Manage')}</button>}>
      {servers.length ? <ul className="sp-list">{servers.map(server => <ServerRow key={server.name} server={server} />)}</ul>
        : <p className="sp-empty">{t('No MCP servers yet. Add one under Connectors to give Neru more tools.')}</p>}
    </Section>

    <Section id="memory" title="Memory">
      <ul className="sp-list">
        <li><button type="button" className="sp-row" onClick={() => openMemory('project')}><BookOpen size={13} aria-hidden /><span className="sp-row-name">{t('Project memory')}</span><span className="sp-row-meta sp-path" title={memory?.project ?? ''}>{memory?.project ? memory.project.split(/[\\/]/).slice(-2).join('/') : t('Not created yet')}</span><ExternalLink size={11} aria-hidden /></button></li>
        <li><button type="button" className="sp-row" onClick={() => openMemory('user')}><BookOpen size={13} aria-hidden /><span className="sp-row-name">{t('Your memory')}</span><span className="sp-row-meta sp-path" title={memory?.user ?? ''}>{memory?.user ? memory.user.split(/[\\/]/).slice(-2).join('/') : ''}</span><ExternalLink size={11} aria-hidden /></button></li>
      </ul>
      {facts.length > 0 && <><p className="sp-subhead">{t('Remembered about this project')}</p><ul className="sp-facts">{facts.slice(0, 8).map(fact => <li key={fact}>{fact}</li>)}</ul></>}
    </Section>

    <Section id="skills" title="Skills" count={enabledSkills.length || undefined} defaultOpen={false}
      action={<button type="button" className="sp-link" onClick={() => onOpenSettings('skills')}>{t('Manage')}</button>}>
      {enabledSkills.length ? <ul className="sp-list">{enabledSkills.map(skill => <li key={`${skill.source}:${skill.name}`} className="sp-row is-static" title={skill.description}><Sparkles size={13} aria-hidden /><span className="sp-row-name">{skill.name}</span><span className="sp-row-meta">{skill.source}</span></li>)}</ul>
        : <p className="sp-empty">{t('No skills turned on.')}</p>}
    </Section>

    <Section id="style" title="Output style" count={current?.name} defaultOpen={false}>
      {styles.length ? <Select className="sp-select" value={current?.name ?? ''} onChange={event => chooseStyle(event.target.value)} aria-label={t('Output style')}>{styles.map(style => <option key={style.name} value={style.name} title={style.description}>{style.name}</option>)}</Select>
        : <p className="sp-empty">{t('Default style.')}</p>}
      {current?.description && <p className="sp-note">{current.description}</p>}
    </Section>

    <Section id="hooks" title="Hooks" count={trust?.hookEvents.length || undefined} defaultOpen={false}>
      {trust?.hookEvents.length ? <ul className="sp-list">{trust.hookEvents.map(event => <li key={event} className="sp-row is-static"><span className="sp-row-name">{event}</span><span className="sp-row-meta">{trust.trusted ? t('Active') : t('Needs trust')}</span></li>)}</ul>
        : <p className="sp-empty">{t('This project has no hooks.')}</p>}
    </Section>
  </div>
}
