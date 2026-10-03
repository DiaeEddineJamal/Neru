import { useCallback, useEffect, useMemo, useState } from 'react'
import { isTauri } from '@tauri-apps/api/core'
import { Check, Copy, Download, Folder, FolderOpen, LoaderCircle, LogIn, RefreshCw, Settings2, ShieldCheck, TriangleAlert } from 'lucide-react'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { cn } from '@/lib/utils'
import { api } from '../../../api'
import type { ImportProgress, ImportScan, ProjectInfo, TeamAgent } from '../../../types'
import { AgentMark } from './TeamView'

const errorText = (value: unknown) => value instanceof Error ? value.message : String(value)
const count = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`
const baseName = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path
const SOURCE_NAMES: Record<string, string> = { claude: 'Claude Code', 'claude-desktop': 'Claude app', codex: 'Codex', cursor: 'Cursor', vscode: 'VS Code', copilot: 'VS Code Copilot', opencode: 'OpenCode', gemini: 'Gemini CLI', shared: 'Shared' }
/** Which logo a source wears; Copilot chats come from VS Code but carry Copilot's. */
const markFor = (source: string) => source

function when(ms: number) {
  const days = Math.floor((Date.now() - ms) / 86_400_000)
  if (days < 1) return 'today'
  if (days < 2) return 'yesterday'
  if (days < 30) return `${days} days ago`
  return new Date(ms).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' })
}

/** Settings → Agents: the coding-agent CLIs Neru can run with your own subscriptions. */
export function AgentsSettings({ onError, onNotice }: { onError: (message: string) => void; onNotice: (message: string) => void }) {
  const [agents, setAgents] = useState<TeamAgent[]>([])
  const [loading, setLoading] = useState(false)
  const [copied, setCopied] = useState('')
  const load = useCallback((refresh: boolean) => {
    if (!isTauri()) return
    setLoading(true)
    void api.listTeamAgents(refresh).then(setAgents).catch(cause => onError(errorText(cause))).finally(() => setLoading(false))
  }, [onError])
  useEffect(() => load(false), [load])
  const copy = (text: string) => { void navigator.clipboard.writeText(text); setCopied(text); window.setTimeout(() => setCopied(''), 1600) }
  // Gemini CLI signs in with a free AI Studio key now that Google retired free personal sign-in.
  const [geminiKey, setGeminiKey] = useState('')
  const [geminiSaved, setGeminiSaved] = useState(false)
  useEffect(() => { if (isTauri()) void api.savedKeyProviders().then(ids => setGeminiSaved(ids.includes('gemini'))).catch(() => undefined) }, [])
  const saveGeminiKey = (key: string) => void api.setGeminiKey(key)
    .then(() => { setGeminiKey(''); setGeminiSaved(Boolean(key.trim())); onNotice(key.trim() ? 'Gemini CLI will use this key. It is encrypted on this PC.' : 'Gemini key removed.'); load(true) })
    .catch(cause => onError(errorText(cause)))
  // Versions: the newest on npm, an update or a specific version, and a CLI path of your own.
  const [latest, setLatest] = useState<Record<string, string | null>>({})
  const [busy, setBusy] = useState('')
  const [open, setOpen] = useState<string | null>(null)
  const [pin, setPin] = useState('')
  const [more, setMore] = useState(false)
  const check = (agent: TeamAgent) => void api.agentLatestVersion(agent.kind).then(version => setLatest(current => ({ ...current, [agent.kind]: version }))).catch(cause => onError(errorText(cause)))
  const install = (agent: TeamAgent, version?: string) => {
    setBusy(agent.kind)
    void api.installAgent(agent.kind, version).then(output => { onNotice(output.split('\n').filter(Boolean).slice(-2).join(' ') || `${agent.name} installed.`); load(true) }).catch(cause => onError(errorText(cause))).finally(() => setBusy(''))
  }
  const choosePath = async (agent: TeamAgent) => {
    const path = await openDialog({ title: `Where is ${agent.name}?`, multiple: false, directory: false })
    if (typeof path !== 'string') return
    void api.setAgentPath(agent.kind, path).then(() => { onNotice(`${agent.name} now runs from ${path}.`); load(true) }).catch(cause => onError(errorText(cause)))
  }
  const resetPath = (agent: TeamAgent) => void api.setAgentPath(agent.kind, null).then(() => load(true)).catch(cause => onError(errorText(cause)))
  const version = (agent: TeamAgent) => agent.version?.match(/\d+\.\d+\.\d+[\w.-]*/)?.[0] ?? null
  const signIn = (agent: TeamAgent) => void api.teamAgentLogin(agent.kind)
    .then(() => onNotice(`${agent.name} sign-in opened in a terminal. Come back and press “Look again” when it is done.`))
    .catch(cause => onError(errorText(cause)))
  return <section className="settings-section provider-settings"><h2>Agents</h2>
    <p className="settings-lede">The coding agents Neru can run as Team members, each on its own subscription. Every CLI signs in for itself; Neru never reads or stores their credentials.</p>
    {agents.filter(agent => agent.path || more).map(agent => <div className="settings-row agent-row" key={agent.kind}>
      <AgentMark kind={agent.kind} size={32} />
      <div><strong>{agent.name}</strong><p>{!agent.path ? 'Not installed' : `${agent.signedIn ? 'Signed in' : 'Not signed in'}${agent.version ? ` · ${version(agent) ?? agent.version}` : ''}${agent.resumes ? '' : ' · plain-text replies, gets the whole thread each turn'}`}{latest[agent.kind] && version(agent) && latest[agent.kind] !== version(agent) ? <b className="agent-update"> · {latest[agent.kind]} available</b> : latest[agent.kind] && agent.path ? ' · up to date' : ''}</p>{agent.path && <p className="agent-path" title={agent.path}>{agent.customPath ? 'Custom path: ' : ''}{agent.path}</p>}</div>
      {!agent.path
        ? <div className="agent-buttons">
          <button className="button subtle small" onClick={() => copy(agent.install)} title={agent.install}>{copied === agent.install ? <Check size={13} /> : <Copy size={13} />}Copy command</button>
          <button className="button primary small" onClick={() => install(agent)} disabled={busy === agent.kind} title={agent.npm ? `npm install -g ${agent.npm}` : 'Opens the installer in a terminal'}>{busy === agent.kind ? <LoaderCircle size={13} className="animate-spin" /> : <Download size={13} />}Install</button>
          <button className="button subtle small" onClick={() => void choosePath(agent)} title="Use a CLI that is installed somewhere Neru does not look"><FolderOpen size={13} />Locate</button>
        </div>
        : <div className="agent-buttons">
          {agent.kind !== 'gemini' && <button className={cn('button small', agent.signedIn ? 'subtle' : 'primary')} onClick={() => signIn(agent)}><LogIn size={13} />{agent.signedIn ? 'Switch account' : 'Sign in'}</button>}
          <button className={cn('icon-button small', open === agent.kind && 'active')} onClick={() => { setOpen(open === agent.kind ? null : agent.kind); setPin(''); if (agent.npm && latest[agent.kind] === undefined) check(agent) }} aria-expanded={open === agent.kind} aria-label={`Version and path for ${agent.name}`} title="Version and path"><Settings2 size={14} /></button>
        </div>}
      {open === agent.kind && agent.path && <div className="agent-key agent-manage">
        <div><span>Version</span><code>{version(agent) ?? agent.version ?? 'unknown'}</code>
          {agent.npm ? <>
            {latest[agent.kind] && latest[agent.kind] !== version(agent) && <button className="button primary small" onClick={() => install(agent)} disabled={busy === agent.kind}>{busy === agent.kind ? <LoaderCircle size={13} className="animate-spin" /> : <Download size={13} />}Update to {latest[agent.kind]}</button>}
            <input value={pin} onChange={event => setPin(event.target.value)} placeholder="Or a version, e.g. 2.1.0" aria-label="Version to install" />
            <button className="button subtle small" onClick={() => install(agent, pin.trim())} disabled={!pin.trim() || busy === agent.kind}>Install</button>
          </> : <button className="button subtle small" onClick={() => install(agent)}>Reinstall or update</button>}
        </div>
        <div><span>Path</span><code className="truncate" title={agent.path}>{agent.path}</code>
          <button className="button subtle small" onClick={() => void choosePath(agent)}><FolderOpen size={13} />Choose…</button>
          {agent.customPath && <button className="button subtle small" onClick={() => resetPath(agent)}>Use PATH</button>}
        </div>
      </div>}
      {agent.kind === 'gemini' && agent.path && <form className="agent-key" onSubmit={event => { event.preventDefault(); if (geminiKey.trim()) saveGeminiKey(geminiKey) }}>
        <p>Google no longer lets Gemini CLI sign in with a free personal account. A free <a href="https://aistudio.google.com/apikey" target="_blank" rel="noreferrer">AI Studio API key</a> works instead, and also unlocks Google Gemini in the model picker.</p>
        <div><input type="password" autoComplete="off" value={geminiKey} onChange={event => setGeminiKey(event.target.value)} placeholder={geminiSaved ? 'Key saved on this PC. Paste a new one to replace it.' : 'Paste your Gemini API key'} aria-label="Gemini API key" />
          <button type="submit" className="button primary small" disabled={!geminiKey.trim()}>Save key</button>
          {geminiSaved && <button type="button" className="button subtle small" onClick={() => saveGeminiKey('')}>Remove</button>}</div>
      </form>}
    </div>)}
    {agents.length === 0 && <p className="settings-note">{loading ? 'Looking for agents…' : 'No agents found yet.'}</p>}
    {agents.some(agent => !agent.path) && <button className="button subtle small agent-more" onClick={() => setMore(value => !value)} aria-expanded={more}>{more ? 'Hide agents that are not installed' : `${agents.filter(agent => !agent.path).length} more agents you can install`}</button>}
    <div className="settings-actions"><button className="button subtle" onClick={() => load(true)} disabled={loading}><RefreshCw size={14} className={loading ? 'animate-spin' : undefined} />Look again</button></div>
    <p className="settings-note"><ShieldCheck size={14} /> Members run in your project with the access you pick per task: read-only, edit files, auto (the agent's own reviewer decides) or full access.</p>
  </section>
}

type ImportTab = 'chats' | 'skills' | 'servers' | 'rules'
const BY_PROJECT = 'neru.imports.byProject'
/** Matches how import groups chats into tasks: slashes and case do not split a folder. */
const folderKey = (path: string) => path.replace(/\\/g, '/').replace(/\/+$/, '').toLowerCase()
const RANGES = [{ days: 1, label: 'Last 24 hours' }, { days: 7, label: 'Last 7 days' }, { days: 30, label: 'Last 30 days' }, { days: 90, label: 'Last 90 days' }, { days: 0, label: 'All time' }]

/** Settings → Imports: chats, skills, MCP servers and instructions from the tools already on this machine. */
export function ImportsSettings({ project, onError, onNotice, onOpenTeam }: { project: ProjectInfo | null; onError: (message: string) => void; onNotice: (message: string) => void; onOpenTeam: () => void }) {
  const [scan, setScan] = useState<ImportScan | null>(null)
  const [scanning, setScanning] = useState(false)
  const [days, setDays] = useState(30)
  const [tab, setTab] = useState<ImportTab>('chats')
  const [source, setSource] = useState('all')
  const [showImported, setShowImported] = useState(false)
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const [working, setWorking] = useState(false)
  const [done, setDone] = useState<string | null>(null)
  const [progress, setProgress] = useState<ImportProgress | null>(null)
  const [byProject, setByProject] = useState(() => { try { return localStorage.getItem(BY_PROJECT) === '1' } catch { return false } })
  const chooseByProject = (on: boolean) => { setByProject(on); try { localStorage.setItem(BY_PROJECT, on ? '1' : '0') } catch { /* storage unavailable */ } }
  useEffect(() => {
    if (!isTauri()) return
    const off = api.onImportProgress(setProgress)
    return () => { void off.then(stop => stop()) }
  }, [])

  const rescan = useCallback((range: number) => {
    if (!isTauri()) return
    setScanning(true); setPicked(new Set())
    void api.scanImports(range).then(setScan).catch(cause => onError(errorText(cause))).finally(() => setScanning(false))
  }, [onError])
  useEffect(() => rescan(days), [rescan, days, project?.path])

  const rows = useMemo(() => {
    if (!scan) return []
    const keep = (from: string) => source === 'all' || markFor(from) === markFor(source) || from === source
    if (tab === 'chats') return scan.chats.filter(chat => keep(chat.source) && (showImported || !chat.imported)).map(chat => ({ key: chat.key, source: chat.source, title: chat.title, detail: chat.unreadable ? chat.reason ?? 'Could not be read' : `${chat.cwd ? baseName(chat.cwd) : 'No folder'} · ${chat.messages ? `${chat.messages} messages · ` : ''}${when(chat.updated)}`, flag: chat.imported ? 'Imported' : '', disabled: chat.unreadable, warn: chat.unreadable, group: chat.cwd }))
    if (tab === 'skills') return scan.skills.filter(skill => keep(skill.source)).map(skill => ({ key: skill.path, source: skill.source, title: skill.name, detail: skill.description || skill.path, flag: skill.conflict ? 'Replaces yours' : '', disabled: false, warn: false, group: '' }))
    if (tab === 'servers') return scan.servers.filter(server => keep(server.source)).map(server => ({ key: server.key, source: server.source, title: server.name, detail: server.target, flag: server.conflict ? 'Replaces yours' : '', disabled: false, warn: false, group: '' }))
    return scan.rules.filter(rules => keep(rules.source)).map(rules => ({ key: rules.path, source: rules.source, title: baseName(rules.path), detail: `${rules.scope === 'project' ? 'Into this project’s .neru/instructions.md' : 'Into Neru’s AGENTS.md, for every project'} · ${rules.path}`, flag: rules.imported ? 'Imported' : '', disabled: rules.imported, warn: false, group: '' }))
  }, [scan, tab, source, showImported])
  /** Chat rows under their project folder, in the order the folders first appear. */
  const groups = useMemo(() => {
    if (tab !== 'chats' || !byProject) return null
    const folders = new Map<string, { path: string; rows: typeof rows }>()
    for (const row of rows) {
      const folder = folders.get(folderKey(row.group))
      if (folder) folder.rows.push(row); else folders.set(folderKey(row.group), { path: row.group, rows: [row] })
    }
    return [...folders.values()]
  }, [rows, tab, byProject])

  const toggle = (key: string) => setPicked(current => { const next = new Set(current); if (next.has(key)) next.delete(key); else next.add(key); return next })
  const pickable = rows.filter(row => !row.disabled)
  const allPicked = pickable.length > 0 && pickable.every(row => picked.has(row.key))
  const pickAll = (keys: string[], on: boolean) => setPicked(current => { const next = new Set(current); for (const key of keys) { if (on) next.add(key); else next.delete(key) } return next })
  const chooseTab = (next: ImportTab) => { setTab(next); setPicked(new Set()); setDone(null) }

  const run = async (kind: ImportTab, keys: string[]) => {
    if (kind === 'chats') { const tasks = await api.importChats(keys); return `${keys.length} ${keys.length === 1 ? 'chat' : 'chats'} became ${tasks.length} Team ${tasks.length === 1 ? 'task' : 'tasks'}.` }
    if (kind === 'skills') { await api.importSkills(keys); return `${keys.length} ${keys.length === 1 ? 'skill' : 'skills'} added to your personal skills.` }
    if (kind === 'servers') { const count = await api.importServers(keys); return `${count} MCP ${count === 1 ? 'server' : 'servers'} added to Connectors.` }
    const count = await api.importRules(keys); return `${count} instruction ${count === 1 ? 'file' : 'files'} added.`
  }
  const importPicked = async () => {
    setWorking(true); setProgress(null)
    try { const message = await run(tab, [...picked]); setDone(message); onNotice(message); rescan(days) } catch (cause) { onError(errorText(cause)) } finally { setWorking(false); setProgress(null) }
  }
  /** Everything not brought over yet, except things that would replace what you already have. */
  const importEverything = async () => {
    if (!scan) return
    const chats = scan.chats.filter(chat => !chat.imported && !chat.unreadable).map(chat => chat.key)
    const skills = scan.skills.filter(skill => !skill.conflict).map(skill => skill.path)
    const servers = scan.servers.filter(server => !server.conflict).map(server => server.key)
    const rules = scan.rules.filter(item => !item.imported).map(item => item.path)
    if (!window.confirm(`Import ${chats.length} chats, ${skills.length} skills, ${servers.length} MCP servers and ${rules.length} instruction files? Anything with the same name as yours is skipped.`)) return
    setWorking(true); setProgress(null)
    const results: string[] = []
    try {
      if (chats.length) results.push(await run('chats', chats))
      if (skills.length) results.push(await run('skills', skills))
      if (servers.length) results.push(await run('servers', servers))
      if (rules.length) results.push(await run('rules', rules))
      const message = results.join(' ') || 'Nothing new to import.'
      setDone(message); onNotice(message); rescan(days)
    } catch (cause) { onError(errorText(cause)) } finally { setWorking(false); setProgress(null) }
  }

  const found = scan?.sources.filter(item => item.found) ?? []
  const tabs: { id: ImportTab; label: string; count: number }[] = [
    { id: 'chats', label: 'Chats', count: scan?.chats.filter(chat => !chat.imported && !chat.unreadable).length ?? 0 },
    { id: 'skills', label: 'Skills', count: scan?.skills.length ?? 0 },
    { id: 'servers', label: 'MCP servers', count: scan?.servers.length ?? 0 },
    { id: 'rules', label: 'Instructions', count: scan?.rules.length ?? 0 },
  ]
  const sourcesInTab = [...new Set((scan ? (tab === 'chats' ? scan.chats : tab === 'skills' ? scan.skills : tab === 'servers' ? scan.servers : scan.rules) : []).map(item => item.source))]

  const renderRow = (row: typeof rows[number]) => <label key={row.key} className={cn('import-row', row.disabled && 'disabled', row.warn && 'unreadable')} role="listitem">
    <input type="checkbox" checked={picked.has(row.key)} disabled={row.disabled} onChange={() => toggle(row.key)} />
    <AgentMark kind={markFor(row.source)} size={22} />
    <span className="import-text"><strong className="truncate">{row.title}</strong><small className="truncate" title={row.detail}>{row.warn && <TriangleAlert size={12} aria-label="Cannot be imported" />}{row.detail}</small></span>
    {row.flag && <span className={cn('import-flag', row.flag === 'Replaces yours' && 'warn')}>{row.flag}</span>}
  </label>

  return <section className="settings-section imports"><h2>Imports</h2>
    <p className="settings-lede">Bring your work over from the agents and editors on this machine. Chats become Team tasks, one per project, and each agent picks up its own conversation where it left off.</p>
    <div className="import-sources">
      {(scan?.sources ?? []).map(item => <div key={item.id} className={cn('import-source', !item.found && 'missing')} title={item.path}>
        <AgentMark kind={markFor(item.id)} size={26} />
        <div><strong>{item.name}</strong><span>{!item.found ? 'Not found' : [item.chats && count(item.chats, 'chat'), item.skills && count(item.skills, 'skill'), item.servers && `${item.servers} MCP`, item.rules && count(item.rules, 'instruction file')].filter(Boolean).join(' · ') || 'Nothing to import'}</span>{item.found && item.note && <small>{item.note}</small>}</div>
      </div>)}
      {!scan && <p className="settings-note">{scanning ? 'Looking through this machine…' : 'Nothing scanned yet.'}</p>}
    </div>
    <div className="import-bar">
      <button className="button primary" onClick={() => void importEverything()} disabled={!scan || working || scanning || found.length === 0}><Download size={15} />Import everything</button>
      <select className="settings-select" value={days} onChange={event => setDays(Number(event.target.value))} aria-label="How far back to look for chats">{RANGES.map(range => <option key={range.days} value={range.days}>{range.label}</option>)}</select>
      <button className="button subtle" onClick={() => rescan(days)} disabled={scanning}><RefreshCw size={14} className={scanning ? 'animate-spin' : undefined} />Scan again</button>
    </div>
    {working && progress && progress.total > 0 && <div className="import-progress" role="status">
      <progress max={progress.total} value={progress.done} aria-label="Chats imported" />
      <span className="truncate" title={progress.title}>{progress.done < progress.total ? `Reading chat ${progress.done + 1} of ${progress.total}: ${progress.title}` : `Read ${progress.total} of ${progress.total} chats. Creating Team tasks…`}</span>
    </div>}
    <nav className="segmented import-tabs" aria-label="What to import">{tabs.map(item => <button key={item.id} className={tab === item.id ? 'active' : ''} aria-current={tab === item.id ? 'page' : undefined} onClick={() => chooseTab(item.id)}>{item.label}<span className="import-count">{item.count}</span></button>)}</nav>
    <div className="import-filters">
      <div className="import-chips" role="group" aria-label="Source">
        <button className={cn('team-to-chip', source === 'all' && 'on')} onClick={() => setSource('all')}>All</button>
        {sourcesInTab.map(item => <button key={item} className={cn('team-to-chip', source === item && 'on')} onClick={() => setSource(item)}>{SOURCE_NAMES[item] ?? item}</button>)}
      </div>
      {tab === 'chats' && <div className="import-chips">
        <label className="import-check"><input type="checkbox" checked={byProject} onChange={event => chooseByProject(event.target.checked)} />By project</label>
        <label className="import-check"><input type="checkbox" checked={showImported} onChange={event => setShowImported(event.target.checked)} />Show imported</label>
      </div>}
    </div>
    <div className="import-list" role="list">
      {rows.length > 0 && <label className="import-row import-all"><input type="checkbox" checked={allPicked} disabled={pickable.length === 0} onChange={() => setPicked(allPicked ? new Set() : new Set(pickable.map(row => row.key)))} /><span>Select all {pickable.length}</span></label>}
      {groups ? groups.map(group => {
        const keys = group.rows.filter(row => !row.disabled).map(row => row.key)
        const chosen = keys.filter(key => picked.has(key)).length
        const all = keys.length > 0 && chosen === keys.length
        const name = group.path ? baseName(group.path) : 'No folder'
        return <div key={folderKey(group.path)} className="import-group" role="list" aria-label={name}>
          <label className="import-row import-group-head" title={group.path || 'Chats without a project folder'}>
            <input type="checkbox" checked={all} disabled={keys.length === 0} ref={input => { if (input) input.indeterminate = chosen > 0 && !all }} onChange={() => pickAll(keys, !all)} aria-label={`Pick all chats in ${name}`} />
            <Folder size={16} />
            <span className="import-text"><strong className="truncate">{name}</strong></span>
            <span className="import-count">{group.rows.length}</span>
          </label>
          {group.rows.map(renderRow)}
        </div>
      }) : rows.map(renderRow)}
      {scan && rows.length === 0 && <p className="empty-small">{tab === 'chats' ? 'No chats in this time range. Try a longer range.' : 'Nothing found here.'}</p>}
    </div>
    <div className="settings-actions import-actions">
      {done && tab === 'chats' && <button className="button subtle" onClick={onOpenTeam}>Open Team</button>}
      <button className="button primary" onClick={() => void importPicked()} disabled={picked.size === 0 || working}>{working ? 'Importing…' : `Import ${picked.size || ''} ${tabs.find(item => item.id === tab)?.label.toLowerCase()}`.replace('  ', ' ')}</button>
    </div>
    <p className="settings-note"><ShieldCheck size={14} /> Neru only reads these tools’ files; nothing in them is changed or deleted. Cursor and VS Code chats come over as history; Claude Code, Codex and OpenCode chats can be continued.</p>
  </section>
}
