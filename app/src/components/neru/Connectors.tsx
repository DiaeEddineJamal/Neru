import { useEffect, useState } from 'react'
import { Cloud, ExternalLink, KeyRound, LoaderCircle, LogIn, LogOut, Monitor, Plus, RotateCw, Search, ShieldCheck, Trash2 } from '@/components/icons'
import { api } from '../../api'
import type { McpServer } from '../../types'
import { t } from '../../i18n'
import { cn } from '@/lib/utils'
import { Select } from '../ui/Select'
import { CATALOG, CATEGORIES, RUNTIMES, explainConnectorError, filterCatalog, type CatalogEntry, type ConnectorCategory } from './connectorCatalog'
import './Connectors.css'

function CatalogLogo({ entry }: { entry: CatalogEntry }) {
  const [failed, setFailed] = useState(false)
  const source = entry.logoUrl ?? (entry.logo ? `https://cdn.simpleicons.org/${entry.logo}` : null)
  return <span className="catalog-logo" aria-hidden>{source && !failed
    ? <img src={source} alt="" width={20} height={20} loading="lazy" referrerPolicy="no-referrer" onError={() => setFailed(true)} />
    : entry.glyph ?? <span className="catalog-letter">{entry.name[0]}</span>}</span>
}

/** Splits an argument line like a shell would for simple cases: spaces separate, quotes group. */
function splitArgs(line: string) {
  return [...line.matchAll(/"([^"]*)"|'([^']*)'|(\S+)/g)].map(match => match[1] ?? match[2] ?? match[3])
}
const joinArgs = (args: string[]) => args.map(arg => /\s/.test(arg) ? `"${arg}"` : arg).join(' ')
const parsePairs = (text: string, separator: string) => Object.fromEntries(text.split('\n').map(line => line.trim()).filter(line => line.includes(separator)).map(line => [line.slice(0, line.indexOf(separator)).trim(), line.slice(line.indexOf(separator) + separator.length).trim()]))
const formatPairs = (pairs: Record<string, string>, separator: string) => Object.entries(pairs).map(([key, value]) => `${key}${separator}${value}`).join('\n')

interface Draft { previous?: string; kind: 'local' | 'hosted'; name: string; command: string; args: string; env: string; url: string; headers: string; enabled: boolean; transport: 'http' | 'sse'; entry?: CatalogEntry }
const blank: Draft = { kind: 'hosted', name: '', command: 'npx', args: '', env: '', url: 'https://', headers: '', enabled: true, transport: 'http' }
/** Endpoints named …/sse are the older HTTP+SSE transport. */
const transportFor = (url: string): Draft['transport'] => /\/sse\/?$/.test(url) ? 'sse' : 'http'
const bearer = (headers: string) => (parsePairs(headers, ':').Authorization ?? '').replace(/^Bearer\s*/i, '')
/** Required settings of a catalog connector the user has not filled in yet. */
const missingFields = (draft: Draft) => {
  if (!draft.entry) return []
  if (draft.kind === 'hosted') return draft.entry.auth === 'token' && !bearer(draft.headers) ? [t('Access token')] : []
  const env = parsePairs(draft.env, '=')
  return (draft.entry.env ?? []).filter(field => !field.optional && !env[field.key]).map(field => t(field.label))
}

const statusText = (server: McpServer) => server.status === 'connected' ? (server.tools.length === 1 ? t('1 tool') : t('{n} tools', { n: server.tools.length }))
  : server.status === 'starting' ? t('Connecting…') : server.status === 'off' ? t('Off') : server.status === 'needs_auth' ? t('Sign-in needed') : server.status === 'needs_trust' ? t('Waiting for trust') : t('Not running')

const openLink = (url: string) => { void api.openUrl(url).catch(() => window.open(url, '_blank', 'noopener')) }

/** What a catalog card says about setup: sign-in, a key, or a program this PC needs. */
function entryMeta(entry: CatalogEntry) {
  if (entry.url) return entry.auth === 'oauth' ? t('Hosted · sign in') : entry.auth === 'token' ? t('Hosted · access token') : t('Hosted · no sign-in')
  const runtime = entry.command ? RUNTIMES[entry.command].name : ''
  const needsKey = (entry.env ?? []).some(field => !field.optional && !field.value)
  return needsKey ? t('This PC · {runtime} · key needed', { runtime }) : t('This PC · {runtime}', { runtime })
}

/** A connector's error in plain words, with the raw text one click away. */
function ConnectorError({ error }: { error: string }) {
  const plain = explainConnectorError(error)
  return <div className="connector-error-box" role="alert">
    <p>{t(plain.message)}{plain.link && <> <button type="button" className="button subtle small" onClick={() => openLink(plain.link!.url)}><ExternalLink size={12} /> {t(plain.link.label)}</button></>}</p>
    <details><summary>{t('Details')}</summary><pre className="connector-error">{error}</pre></details>
  </div>
}

/** Settings → Connectors: MCP servers, hosted or local, that Neru offers to the model as tools. */
export function Connectors({ onError }: { onError: (message: string) => void }) {
  const [servers, setServers] = useState<McpServer[] | null>(null)
  const [draft, setDraft] = useState<Draft | null>(null)
  const [saving, setSaving] = useState(false)
  const [working, setWorking] = useState<string | null>(null)
  const [query, setQuery] = useState('')
  const [category, setCategory] = useState<ConnectorCategory | 'all'>('all')
  const [trusted, setTrusted] = useState(false)

  useEffect(() => {
    let alive = true
    api.mcpServers().then(list => { if (alive) setServers(list) }).catch(cause => onError(String(cause)))
    api.projectTrustStatus().then(status => { if (alive) setTrusted(status.trusted) }).catch(() => undefined)
    return () => { alive = false }
  }, [onError])
  const untrust = async () => {
    setWorking('trust')
    try { setTrusted((await api.untrustProject()).trusted); setServers(await api.mcpServers()) } catch (cause) { onError(String(cause)) } finally { setWorking(null) }
  }
  const starting = Boolean(servers?.some(server => server.status === 'starting'))
  useEffect(() => {
    // Connectors finish starting in the background; refresh until none is still starting.
    if (!starting) return
    const timer = window.setInterval(() => { void api.mcpServers().then(setServers).catch(() => undefined) }, 1500)
    return () => window.clearInterval(timer)
  }, [starting])

  const act = async (name: string, action: () => Promise<McpServer[]>) => {
    setWorking(name)
    try { setServers(await action()) } catch (cause) { onError(String(cause)) } finally { setWorking(null) }
  }
  const save = async () => {
    if (!draft) return
    setSaving(true)
    try {
      const hosted = draft.kind === 'hosted'
      // Empty values are optional keys left blank: leave them unset rather than set to "".
      const env = Object.fromEntries(Object.entries(parsePairs(draft.env, '=')).filter(([, value]) => value !== ''))
      const list = await api.mcpSaveServer({
        name: draft.name,
        command: hosted ? '' : draft.command,
        args: hosted ? [] : splitArgs(draft.args),
        env: hosted ? {} : env,
        url: hosted ? draft.url.trim() : undefined,
        headers: hosted ? parsePairs(draft.headers, ':') : {},
        enabled: draft.enabled,
        transport: hosted && draft.transport === 'sse' ? 'sse' : undefined,
      }, draft.previous)
      setServers(list)
      const saved = list.find(server => server.name === draft.name)
      setDraft(null)
      // Hosted connectors that want OAuth go straight to the browser sign-in.
      if (saved?.status === 'needs_auth') await act(saved.name, () => api.mcpSignIn(saved.name))
    } catch (cause) { onError(String(cause)) } finally { setSaving(false) }
  }
  const toggle = (server: McpServer) => act(server.name, () => api.mcpSaveServer({ ...server, enabled: !server.enabled }, server.name))
  const pick = (entry: CatalogEntry) => setDraft(entry.url
    ? { ...blank, entry, kind: 'hosted', name: entry.id, url: entry.url, headers: entry.auth === 'token' ? 'Authorization: Bearer ' : '', transport: entry.transport ?? transportFor(entry.url) }
    : { ...blank, entry, kind: 'local', name: entry.id, command: entry.command ?? 'npx', args: entry.args ?? '', env: (entry.env ?? []).map(field => `${field.key}=${field.value ?? ''}`).join('\n') })
  const available = CATALOG.filter(entry => !servers?.some(server => server.name === entry.id))
  const catalog = filterCatalog(available, query, category)
  const missing = draft ? missingFields(draft) : []
  const setEnv = (key: string, value: string) => draft && setDraft({ ...draft, env: formatPairs({ ...parsePairs(draft.env, '='), [key]: value }, '=') })
  const setToken = (value: string) => draft && setDraft({ ...draft, headers: formatPairs({ ...parsePairs(draft.headers, ':'), Authorization: `Bearer ${value.trim()}` }, ': ') })

  return <section className="settings-section provider-settings"><h2>{t('Connectors')}</h2>
    <p className="settings-lede">{t('Give Neru more tools with Model Context Protocol servers: hosted services you sign in to, or programs that run on this PC. Neru asks before every connector call unless you always allow it or choose a permission mode that does.')}</p>
    {servers === null && <p className="settings-note"><LoaderCircle size={14} className="animate-spin" /> {t('Loading connectors…')}</p>}
    {servers && servers.length > 0 && <div className="connector-list">{servers.map(server => <div key={server.name} className="connector">
      <div className="connector-head">
        <span className={cn('connector-dot', server.status)} aria-hidden />
        <div className="connector-title"><strong>{server.name}</strong><code>{server.url ?? `${server.command} ${joinArgs(server.args)}`}</code>{server.source === 'project' && <small className="connector-source">{t('From this project (.mcp.json)')}</small>}</div>
        <span className="connector-status">{server.url ? <Cloud size={12} /> : <Monitor size={12} />} {statusText(server)}</span>
        <div className="connector-actions">
          {server.status === 'needs_trust' && <button className="button primary small" disabled={working !== null} onClick={() => void act(server.name, async () => { await api.trustProject(); return api.mcpServers() })} title={t('Trust this project folder and start the connectors in its .mcp.json')}>{working === server.name ? <LoaderCircle size={13} className="animate-spin" /> : <ShieldCheck size={13} />} {t('Trust project')}</button>}
          {server.url && server.status === 'needs_auth' && <button className="button primary small" disabled={working === server.name} onClick={() => void act(server.name, () => api.mcpSignIn(server.name))}>{working === server.name ? <LoaderCircle size={13} className="animate-spin" /> : <LogIn size={13} />} {t('Sign in')}</button>}
          {server.url && server.signedIn && <button className="icon-button small" disabled={working === server.name} onClick={() => void act(server.name, () => api.mcpSignOut(server.name))} title={t('Sign out')} aria-label={t('Sign out of {name}', { name: server.name })}><LogOut size={14} /></button>}
          {server.source !== 'project' && <button className="icon-button small" disabled={working === server.name} onClick={() => void toggle(server)} title={server.enabled ? t('Turn off') : t('Turn on')} aria-label={server.enabled ? t('Turn off {name}', { name: server.name }) : t('Turn on {name}', { name: server.name })}><span className={cn('switch', server.enabled && 'on')} /></button>}
          {server.status !== 'needs_trust' && <button className="icon-button small" disabled={!server.enabled || working === server.name} onClick={() => void act(server.name, () => api.mcpRestart(server.name))} title={t('Reconnect')} aria-label={t('Reconnect {name}', { name: server.name })}>{working === server.name && server.status !== 'needs_auth' ? <LoaderCircle size={14} className="animate-spin" /> : <RotateCw size={14} />}</button>}
          {server.source !== 'project' && <><button className="button subtle small" onClick={() => setDraft({ previous: server.name, kind: server.url ? 'hosted' : 'local', name: server.name, command: server.command || 'npx', args: joinArgs(server.args), env: formatPairs(server.env, '='), url: server.url ?? 'https://', headers: formatPairs(server.headers ?? {}, ': '), enabled: server.enabled, transport: server.transport === 'sse' ? 'sse' : 'http' })}>{t('Edit')}</button>
          <button className="icon-button small" onClick={() => { if (window.confirm(server.signedIn ? t('Remove the {name} connector and its sign-in?', { name: server.name }) : t('Remove the {name} connector?', { name: server.name }))) void act(server.name, () => api.mcpRemoveServer(server.name)) }} title={t('Remove')} aria-label={t('Remove {name}', { name: server.name })}><Trash2 size={14} /></button></>}
        </div>
      </div>
      {server.status === 'error' && server.error && <ConnectorError error={server.error} />}
      {server.status === 'needs_trust' && <p className="connector-note">{t("Not started yet: this project's .mcp.json asks for it. Trust the folder if you know where the project came from.")}</p>}
      {server.status === 'connected' && server.tools.length > 0 && <div className="connector-tools">{server.tools.map(tool => <span key={tool.name} title={tool.description}>{tool.name}{tool.readOnly && <small>{t('read-only')}</small>}</span>)}</div>}
      {server.status === 'connected' && (server.resources?.length ?? 0) > 0 && <div className="connector-tools" aria-label={t('Resources')}><small className="connector-kind">{t('Resources, attach with @')}</small>{server.resources!.map(resource => <span key={resource.uri} title={resource.description || resource.uri}>{resource.name}</span>)}</div>}
      {server.status === 'connected' && (server.prompts?.length ?? 0) > 0 && <div className="connector-tools" aria-label={t('Prompts')}><small className="connector-kind">{t('Prompts, run as commands')}</small>{server.prompts!.map(prompt => <span key={prompt.name} title={prompt.description}>/mcp__{server.name}__{prompt.name}</span>)}</div>}
    </div>)}</div>}

    {draft ? <div className="connector-form">
      {draft.entry ? <div className="connector-setup">
        <div className="connector-setup-head"><CatalogLogo entry={draft.entry} /><div><strong>{draft.entry.name}</strong><small>{t(draft.entry.description)}</small></div></div>
        {draft.entry.command && <p className="connector-requirement"><Monitor size={13} /> {t('Runs on this PC and needs {runtime}. If it is not installed yet, install it, then restart Neru.', { runtime: RUNTIMES[draft.entry.command].name })} <button type="button" className="button subtle small" onClick={() => openLink(RUNTIMES[draft.entry!.command!].url)}><ExternalLink size={12} /> {t('Get {runtime}', { runtime: RUNTIMES[draft.entry.command].name })}</button></p>}
        {draft.entry.auth === 'oauth' && <p className="connector-requirement"><LogIn size={13} /> {t('Your browser opens to sign in after you add it.')}</p>}
        {draft.entry.setup && <p className="connector-requirement">{t(draft.entry.setup)}</p>}
        {draft.entry.setupUrl && <button type="button" className="button subtle small" onClick={() => openLink(draft.entry!.setupUrl!)}><ExternalLink size={12} /> {draft.entry.auth === 'token' || draft.entry.env?.some(field => field.secret) ? t('Where to get the key') : t('Setup guide')}</button>}
        {draft.kind === 'local' && (draft.entry.env ?? []).filter(field => !field.value).map(field => <label key={field.key}>{t(field.label)}{field.optional && <small className="connector-optional"> {t('(optional)')}</small>}
          <input type={field.secret ? 'password' : 'text'} autoComplete="off" spellCheck={false} value={parsePairs(draft.env, '=')[field.key] ?? ''} onChange={event => setEnv(field.key, event.target.value)} placeholder={field.key} /></label>)}
        {draft.kind === 'hosted' && draft.entry.auth === 'token' && <label><span className="connector-label"><KeyRound size={13} /> {t('Access token')}</span><input type="password" autoComplete="off" spellCheck={false} value={bearer(draft.headers)} onChange={event => setToken(event.target.value)} placeholder="github_pat_…" /></label>}
      </div> : <div className="segmented" role="tablist" aria-label={t('Connector type')}>
        <button role="tab" aria-selected={draft.kind === 'hosted'} className={draft.kind === 'hosted' ? 'active' : ''} onClick={() => setDraft({ ...draft, kind: 'hosted' })}><Cloud size={14} /> {t('Hosted (URL)')}</button>
        <button role="tab" aria-selected={draft.kind === 'local'} className={draft.kind === 'local' ? 'active' : ''} onClick={() => setDraft({ ...draft, kind: 'local' })}><Monitor size={14} /> {t('Runs on this PC')}</button>
      </div>}
      <details className="connector-advanced" open={!draft.entry}>
        <summary>{draft.entry ? t('Advanced settings') : t('Settings')}</summary>
        <label>{t('Name')}<input autoFocus={!draft.entry} value={draft.name} onChange={event => setDraft({ ...draft, name: event.target.value })} placeholder="linear" /></label>
        {draft.kind === 'hosted' ? <>
          <label>{t('Server URL')}<input value={draft.url} onChange={event => setDraft({ ...draft, url: event.target.value, transport: draft.previous ? draft.transport : transportFor(event.target.value) })} placeholder="https://mcp.example.com/mcp" /></label>
          <label>{t('Transport')}<Select className="settings-select" value={draft.transport} onChange={event => setDraft({ ...draft, transport: event.target.value as Draft['transport'] })}>
            <option value="http">{t('Streamable HTTP (current)')}</option>
            <option value="sse">{t('HTTP + SSE (older servers)')}</option>
          </Select></label>
          <label>{t('Headers (optional, one Name: value per line)')}<textarea rows={2} value={draft.headers} onChange={event => setDraft({ ...draft, headers: event.target.value })} placeholder={t('Authorization: Bearer …   (leave empty to sign in with OAuth)')} spellCheck={false} /></label>
        </> : <>
          <div className="settings-grid"><label>{t('Command')}<input value={draft.command} onChange={event => setDraft({ ...draft, command: event.target.value })} placeholder={t('npx, uvx, node, python…')} /></label><label>{t('Arguments')}<input value={draft.args} onChange={event => setDraft({ ...draft, args: event.target.value })} placeholder="-y @modelcontextprotocol/server-filesystem D:\Neru\projects" /></label></div>
          <label>{t('Environment (one KEY=value per line)')}<textarea rows={3} value={draft.env} onChange={event => setDraft({ ...draft, env: event.target.value })} placeholder="GITHUB_PERSONAL_ACCESS_TOKEN=…" spellCheck={false} /></label>
        </>}
      </details>
      {missing.length > 0 && <p className="connector-missing">{t('Fill in: {fields}', { fields: missing.join(', ') })}</p>}
      <div className="settings-actions"><button className="button primary" disabled={saving || missing.length > 0 || !draft.name.trim() || (draft.kind === 'hosted' ? !/^https?:\/\/.+/.test(draft.url) : !draft.command.trim())} onClick={() => void save()}>{saving ? <><LoaderCircle size={14} className="animate-spin" /> {t('Connecting…')}</> : draft.previous ? t('Save connector') : t('Add connector')}</button><button className="button subtle" onClick={() => setDraft(null)}>{t('Cancel')}</button></div>
    </div> : servers && <div className="catalog">
      <div className="catalog-top">
        <div className="settings-subhead"><strong>{t('Add a connector')}</strong><span>{t('Pick one to fill it in, or add your own.')}</span></div>
        <button className="button subtle small" onClick={() => setDraft({ ...blank })}><Plus size={13} /> {t('Custom')}</button>
      </div>
      <label className="settings-search catalog-search"><Search size={15} /><input type="search" value={query} onChange={event => setQuery(event.target.value)} placeholder={t('Search connectors: Gmail, arXiv, Notion…')} aria-label={t('Search connectors')} /></label>
      <div className="catalog-chips" role="group" aria-label={t('Categories')}>
        <button type="button" className={cn('catalog-chip', category === 'all' && 'active')} aria-pressed={category === 'all'} onClick={() => setCategory('all')}>{t('All')}</button>
        {CATEGORIES.map(item => <button key={item.id} type="button" className={cn('catalog-chip', category === item.id && 'active')} aria-pressed={category === item.id} onClick={() => setCategory(item.id)}>{t(item.label)}</button>)}
      </div>
      {catalog.length === 0 ? <p className="settings-note">{t('No connector matches. Try another word, or add your own with Custom.')}</p>
        : <div className="catalog-grid">{catalog.map(entry => <button key={entry.id} className="catalog-item" onClick={() => pick(entry)} title={entry.url ?? `${entry.command} ${entry.args}`}>
          <CatalogLogo entry={entry} />
          <span className="catalog-text"><strong>{entry.name}</strong><small>{t(entry.description)}</small><span className="catalog-meta">{entry.url ? <Cloud size={11} /> : <Monitor size={11} />} {entryMeta(entry)}</span></span>
          {entry.auth === 'oauth' ? <LogIn size={14} className="catalog-add" /> : <Plus size={14} className="catalog-add" />}
        </button>)}</div>}
    </div>}
    {trusted && <p className="settings-note trust-note"><ShieldCheck size={14} /> {t('This project folder is trusted: its .mcp.json connectors, settings hooks and allow rules apply.')} <button className="button subtle small" disabled={working !== null} onClick={() => void untrust()}>{working === 'trust' ? <LoaderCircle size={13} className="animate-spin" /> : null} {t('Stop trusting')}</button></p>}
    <p className="settings-note"><ShieldCheck size={14} /> {t('Hosted sign-ins happen in your browser; Neru stores only the resulting token, encrypted with your system keychain, and refreshes it. Local connectors run with your permissions and keep their package caches in Neru’s storage. Add only servers you trust.')}</p>
  </section>
}
