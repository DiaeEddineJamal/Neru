import { useEffect, useState, type ReactNode } from 'react'
import { BookOpen, Brain, Cloud, Download, FolderOpen, Globe, LoaderCircle, LogIn, LogOut, Monitor, Plus, RotateCw, ShieldCheck, Trash2 } from 'lucide-react'
import { api } from '../../api'
import type { McpServer } from '../../types'
import { cn } from '@/lib/utils'

type Auth = 'oauth' | 'token' | 'none'
interface CatalogEntry {
  id: string; name: string; description: string
  /** Hosted connectors: their Streamable HTTP endpoint and how they authenticate. */
  url?: string; auth?: Auth
  /** Local connectors: the command that starts them and the keys they need. */
  command?: string; args?: string; env?: string[]
  logo?: string; logoUrl?: string; glyph?: ReactNode
}

/**
 * Well-known MCP servers. Hosted ones connect over HTTPS and sign in in the browser; local ones run on
 * this PC. Logos come from Simple Icons' official CDN, or the brand's own site / Wikimedia Commons for
 * brands Simple Icons does not carry.
 */
const HOSTED: CatalogEntry[] = [
  { id: 'linear', name: 'Linear', description: 'Issues, projects and cycles', url: 'https://mcp.linear.app/mcp', auth: 'oauth', logo: 'linear' },
  { id: 'notion', name: 'Notion', description: 'Search, read and update pages and databases', url: 'https://mcp.notion.com/mcp', auth: 'oauth', logo: 'notion' },
  { id: 'figma', name: 'Figma', description: 'Designs, frames and variables for building UI', url: 'https://mcp.figma.com/mcp', auth: 'oauth', logo: 'figma' },
  { id: 'sentry', name: 'Sentry', description: 'Errors, issues and stack traces', url: 'https://mcp.sentry.dev/mcp', auth: 'oauth', logo: 'sentry' },
  { id: 'github', name: 'GitHub', description: 'Repos, issues and pull requests (personal access token)', url: 'https://api.githubcopilot.com/mcp/', auth: 'token', logo: 'github' },
  { id: 'stripe', name: 'Stripe', description: 'Customers, payments and products', url: 'https://mcp.stripe.com', auth: 'oauth', logo: 'stripe' },
  { id: 'supabase', name: 'Supabase', description: 'Tables, SQL, migrations and edge functions', url: 'https://mcp.supabase.com/mcp', auth: 'oauth', logo: 'supabase' },
  { id: 'vercel', name: 'Vercel', description: 'Projects, deployments and logs', url: 'https://mcp.vercel.com', auth: 'oauth', logo: 'vercel' },
  { id: 'cloudflare-docs', name: 'Cloudflare Docs', description: 'Search the Cloudflare developer docs', url: 'https://docs.mcp.cloudflare.com/mcp', auth: 'none', logo: 'cloudflare' },
  { id: 'huggingface', name: 'Hugging Face', description: 'Models, datasets, papers and Spaces', url: 'https://huggingface.co/mcp', auth: 'none', logo: 'huggingface' },
  { id: 'context7', name: 'Context7', description: 'Up-to-date library docs and code examples', url: 'https://mcp.context7.com/mcp', auth: 'none', glyph: <BookOpen size={18} /> },
  { id: 'deepwiki', name: 'DeepWiki', description: 'Ask questions about any public GitHub repository', url: 'https://mcp.deepwiki.com/mcp', auth: 'none', glyph: <Globe size={18} /> },
]

const LOCAL: CatalogEntry[] = [
  { id: 'filesystem', name: 'Filesystem', description: 'Read and search files in folders you choose', command: 'npx', args: '-y @modelcontextprotocol/server-filesystem D:\\Neru\\projects', glyph: <FolderOpen size={18} /> },
  { id: 'playwright', name: 'Playwright', description: 'Drive a real browser: click, type, screenshot', command: 'npx', args: '-y @playwright/mcp@latest', logoUrl: 'https://playwright.dev/img/playwright-logo.svg', glyph: <Globe size={18} /> },
  { id: 'postgres', name: 'PostgreSQL', description: 'Inspect schemas and run read-only queries', command: 'npx', args: '-y @modelcontextprotocol/server-postgres postgresql://localhost/mydb', logo: 'postgresql' },
  { id: 'slack', name: 'Slack', description: 'Read channels and threads, post messages', command: 'npx', args: '-y @modelcontextprotocol/server-slack', env: ['SLACK_BOT_TOKEN', 'SLACK_TEAM_ID'], logoUrl: 'https://upload.wikimedia.org/wikipedia/commons/d/d5/Slack_icon_2019.svg' },
  { id: 'brave', name: 'Brave Search', description: 'Web and local search with the Brave API', command: 'npx', args: '-y @modelcontextprotocol/server-brave-search', env: ['BRAVE_API_KEY'], logo: 'brave' },
  { id: 'figma-local', name: 'Figma (API key)', description: 'Framelink: layout data from a Figma API key', command: 'npx', args: '-y figma-developer-mcp --stdio', env: ['FIGMA_API_KEY'], logo: 'figma' },
  { id: 'memory', name: 'Memory', description: 'A small knowledge graph Neru can remember with', command: 'npx', args: '-y @modelcontextprotocol/server-memory', glyph: <Brain size={18} /> },
  { id: 'git', name: 'Git', description: 'Log, diff, blame and branches for any repository (needs uv)', command: 'uvx', args: 'mcp-server-git', logo: 'git' },
  { id: 'fetch', name: 'Fetch', description: 'Fetch web pages as Markdown (needs uv)', command: 'uvx', args: 'mcp-server-fetch', glyph: <Download size={18} /> },
]

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

interface Draft { previous?: string; kind: 'local' | 'hosted'; name: string; command: string; args: string; env: string; url: string; headers: string; enabled: boolean }
const blank: Draft = { kind: 'hosted', name: '', command: 'npx', args: '', env: '', url: 'https://', headers: '', enabled: true }

const statusText = (server: McpServer) => server.status === 'connected' ? `${server.tools.length} tool${server.tools.length === 1 ? '' : 's'}`
  : server.status === 'starting' ? 'Connecting…' : server.status === 'off' ? 'Off' : server.status === 'needs_auth' ? 'Sign-in needed' : 'Not running'

/** Settings → Connectors: MCP servers, hosted or local, that Neru offers to the model as tools. */
export function Connectors({ onError }: { onError: (message: string) => void }) {
  const [servers, setServers] = useState<McpServer[] | null>(null)
  const [draft, setDraft] = useState<Draft | null>(null)
  const [saving, setSaving] = useState(false)
  const [working, setWorking] = useState<string | null>(null)
  const [tab, setTab] = useState<'hosted' | 'local'>('hosted')

  useEffect(() => {
    let alive = true
    api.mcpServers().then(list => { if (alive) setServers(list) }).catch(cause => onError(String(cause)))
    return () => { alive = false }
  }, [onError])
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
      const list = await api.mcpSaveServer({
        name: draft.name,
        command: hosted ? '' : draft.command,
        args: hosted ? [] : splitArgs(draft.args),
        env: hosted ? {} : parsePairs(draft.env, '='),
        url: hosted ? draft.url.trim() : undefined,
        headers: hosted ? parsePairs(draft.headers, ':') : {},
        enabled: draft.enabled,
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
    ? { ...blank, kind: 'hosted', name: entry.id, url: entry.url, headers: entry.auth === 'token' ? 'Authorization: Bearer ' : '' }
    : { ...blank, kind: 'local', name: entry.id, command: entry.command ?? 'npx', args: entry.args ?? '', env: (entry.env ?? []).map(key => `${key}=`).join('\n') })
  const catalog = (tab === 'hosted' ? HOSTED : LOCAL).filter(entry => !servers?.some(server => server.name === entry.id))

  return <section className="settings-section provider-settings"><h2>Connectors</h2>
    <p className="settings-lede">Give Neru more tools with Model Context Protocol servers: hosted services you sign in to, or programs that run on this PC. Neru asks before every connector call unless you always allow it or choose a permission mode that does.</p>
    {servers === null && <p className="settings-note"><LoaderCircle size={14} className="animate-spin" /> Loading connectors…</p>}
    {servers && servers.length > 0 && <div className="connector-list">{servers.map(server => <div key={server.name} className="connector">
      <div className="connector-head">
        <span className={cn('connector-dot', server.status)} aria-hidden />
        <div className="connector-title"><strong>{server.name}</strong><code>{server.url ?? `${server.command} ${joinArgs(server.args)}`}</code></div>
        <span className="connector-status">{server.url ? <Cloud size={12} /> : <Monitor size={12} />} {statusText(server)}</span>
        <div className="connector-actions">
          {server.url && server.status === 'needs_auth' && <button className="button primary small" disabled={working === server.name} onClick={() => void act(server.name, () => api.mcpSignIn(server.name))}>{working === server.name ? <LoaderCircle size={13} className="animate-spin" /> : <LogIn size={13} />} Sign in</button>}
          {server.url && server.signedIn && <button className="icon-button small" disabled={working === server.name} onClick={() => void act(server.name, () => api.mcpSignOut(server.name))} title="Sign out" aria-label={`Sign out of ${server.name}`}><LogOut size={14} /></button>}
          <button className="icon-button small" disabled={working === server.name} onClick={() => void toggle(server)} title={server.enabled ? 'Turn off' : 'Turn on'} aria-label={server.enabled ? `Turn off ${server.name}` : `Turn on ${server.name}`}><span className={cn('switch', server.enabled && 'on')} /></button>
          <button className="icon-button small" disabled={!server.enabled || working === server.name} onClick={() => void act(server.name, () => api.mcpRestart(server.name))} title="Reconnect" aria-label={`Reconnect ${server.name}`}>{working === server.name && server.status !== 'needs_auth' ? <LoaderCircle size={14} className="animate-spin" /> : <RotateCw size={14} />}</button>
          <button className="button subtle small" onClick={() => setDraft({ previous: server.name, kind: server.url ? 'hosted' : 'local', name: server.name, command: server.command || 'npx', args: joinArgs(server.args), env: formatPairs(server.env, '='), url: server.url ?? 'https://', headers: formatPairs(server.headers ?? {}, ': '), enabled: server.enabled })}>Edit</button>
          <button className="icon-button small" onClick={() => { if (window.confirm(`Remove the ${server.name} connector${server.signedIn ? ' and its sign-in' : ''}?`)) void act(server.name, () => api.mcpRemoveServer(server.name)) }} title="Remove" aria-label={`Remove ${server.name}`}><Trash2 size={14} /></button>
        </div>
      </div>
      {server.status === 'error' && server.error && <pre className="connector-error">{server.error}</pre>}
      {server.status === 'connected' && server.tools.length > 0 && <div className="connector-tools">{server.tools.map(tool => <span key={tool.name} title={tool.description}>{tool.name}{tool.readOnly && <small>read-only</small>}</span>)}</div>}
    </div>)}</div>}

    {draft ? <div className="connector-form">
      <div className="segmented" role="tablist" aria-label="Connector type">
        <button role="tab" aria-selected={draft.kind === 'hosted'} className={draft.kind === 'hosted' ? 'active' : ''} onClick={() => setDraft({ ...draft, kind: 'hosted' })}><Cloud size={14} /> Hosted (URL)</button>
        <button role="tab" aria-selected={draft.kind === 'local'} className={draft.kind === 'local' ? 'active' : ''} onClick={() => setDraft({ ...draft, kind: 'local' })}><Monitor size={14} /> Runs on this PC</button>
      </div>
      <label>Name<input autoFocus value={draft.name} onChange={event => setDraft({ ...draft, name: event.target.value })} placeholder="linear" /></label>
      {draft.kind === 'hosted' ? <>
        <label>Server URL<input value={draft.url} onChange={event => setDraft({ ...draft, url: event.target.value })} placeholder="https://mcp.example.com/mcp" /></label>
        <label>Headers (optional, one Name: value per line)<textarea rows={2} value={draft.headers} onChange={event => setDraft({ ...draft, headers: event.target.value })} placeholder="Authorization: Bearer …   (leave empty to sign in with OAuth)" spellCheck={false} /></label>
      </> : <>
        <div className="settings-grid"><label>Command<input value={draft.command} onChange={event => setDraft({ ...draft, command: event.target.value })} placeholder="npx, uvx, node, python…" /></label><label>Arguments<input value={draft.args} onChange={event => setDraft({ ...draft, args: event.target.value })} placeholder="-y @modelcontextprotocol/server-filesystem D:\Neru\projects" /></label></div>
        <label>Environment (one KEY=value per line)<textarea rows={3} value={draft.env} onChange={event => setDraft({ ...draft, env: event.target.value })} placeholder="GITHUB_PERSONAL_ACCESS_TOKEN=…" spellCheck={false} /></label>
      </>}
      <div className="settings-actions"><button className="button primary" disabled={saving || !draft.name.trim() || (draft.kind === 'hosted' ? !/^https?:\/\/.+/.test(draft.url) : !draft.command.trim())} onClick={() => void save()}>{saving ? <><LoaderCircle size={14} className="animate-spin" /> Connecting…</> : draft.previous ? 'Save connector' : 'Add connector'}</button><button className="button subtle" onClick={() => setDraft(null)}>Cancel</button></div>
    </div> : servers && <div className="catalog">
      <div className="catalog-top">
        <div className="settings-subhead"><strong>Add a connector</strong><span>Pick one to fill it in, or add your own.</span></div>
        <button className="button subtle small" onClick={() => setDraft({ ...blank, kind: tab })}><Plus size={13} /> Custom</button>
      </div>
      <div className="segmented" role="tablist" aria-label="Catalog">
        <button role="tab" aria-selected={tab === 'hosted'} className={tab === 'hosted' ? 'active' : ''} onClick={() => setTab('hosted')}><Cloud size={14} /> Hosted, sign in</button>
        <button role="tab" aria-selected={tab === 'local'} className={tab === 'local' ? 'active' : ''} onClick={() => setTab('local')}><Monitor size={14} /> Runs on this PC</button>
      </div>
      <div className="catalog-grid">{catalog.map(entry => <button key={entry.id} className="catalog-item" onClick={() => pick(entry)} title={entry.url ?? `${entry.command} ${entry.args}`}>
        <CatalogLogo entry={entry} />
        <span className="catalog-text"><strong>{entry.name}</strong><small>{entry.description}</small></span>
        {entry.auth === 'oauth' ? <LogIn size={14} className="catalog-add" /> : <Plus size={14} className="catalog-add" />}
      </button>)}</div>
    </div>}
    <p className="settings-note"><ShieldCheck size={14} /> Hosted sign-ins happen in your browser; Neru stores only the resulting token, encrypted with your system keychain, and refreshes it. Local connectors run with your permissions and keep their package caches in Neru’s storage. Add only servers you trust.</p>
  </section>
}
