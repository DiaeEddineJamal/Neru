import { useEffect, useRef, useState } from 'react'
import { isTauri } from '@tauri-apps/api/core'
import { Check, Copy, LoaderCircle, SquareTerminal } from 'lucide-react'
import { api } from '../../api'
import type { CliStatus } from '../../types'

type Os = 'windows' | 'macos' | 'linux'
const OS_TABS: { id: Os; label: string }[] = [{ id: 'windows', label: 'Windows' }, { id: 'macos', label: 'macOS' }, { id: 'linux', label: 'Linux' }]
const REPO = 'https://raw.githubusercontent.com/DiaeEddineJamal/Neru/main'

function currentOs(): Os {
  const text = `${navigator.platform} ${navigator.userAgent}`
  return /Win/i.test(text) ? 'windows' : /Mac|iPhone|iPad/i.test(text) ? 'macos' : 'linux'
}

// Only routes that work end to end with the published releases.
const INSTALL: Record<Os, { label: string; command: string }[]> = {
  windows: [
    { label: 'PowerShell', command: `irm ${REPO}/install.ps1 | iex` },
    { label: 'With Node.js 18 or later', command: 'npm install -g neru-cli' },
  ],
  macos: [
    { label: 'Terminal (Apple silicon and Intel)', command: `curl -fsSL ${REPO}/install.sh | bash` },
    { label: 'With Node.js 18 or later', command: 'npm install -g neru-cli' },
  ],
  linux: [
    { label: 'Terminal (x64 and arm64)', command: `curl -fsSL ${REPO}/install.sh | bash` },
    { label: 'With Node.js 18 or later', command: 'npm install -g neru-cli' },
  ],
}

const UPDATE: Record<Os, [string, string][]> = {
  windows: [
    ['neru update', 'Updates a PowerShell install'],
    ['npm install -g neru-cli@latest', 'Updates an npm install'],
    [`$env:NERU_UNINSTALL='1'; irm ${REPO}/install.ps1 | iex`, 'Removes a PowerShell install'],
    ['npm uninstall -g neru-cli', 'Removes an npm install'],
  ],
  macos: [
    ['neru update', 'Updates a script install'],
    ['npm install -g neru-cli@latest', 'Updates an npm install'],
    [`curl -fsSL ${REPO}/install.sh | bash -s -- --uninstall`, 'Removes a script install'],
    ['npm uninstall -g neru-cli', 'Removes an npm install'],
  ],
  linux: [
    ['neru update', 'Updates a script install'],
    ['npm install -g neru-cli@latest', 'Updates an npm install'],
    [`curl -fsSL ${REPO}/install.sh | bash -s -- --uninstall`, 'Removes a script install'],
    ['npm uninstall -g neru-cli', 'Removes an npm install'],
  ],
}

const USAGE: [string, string][] = [
  ['neru', 'Start a session in this folder'],
  ['neru "fix the failing test"', 'Start with a first message'],
  ['neru -p "explain this project"', 'Print the answer and exit'],
  ['cat log.txt | neru -p "why does this fail?"', 'Ask about piped input'],
  ['neru -c', 'Continue the latest session'],
  ['neru -r [title or id]', 'Resume a session'],
  ['neru login', 'Connect a model provider (several are free)'],
  ['neru models [search]', 'List the models your key can use'],
  ['neru mcp list | get | add | add-json | remove', 'Manage connectors, with claude mcp syntax'],
  ['neru sessions', 'List this folder\'s sessions'],
  ['neru doctor', 'Check the model, tools and setup'],
  ['neru config', 'Show where settings live and what is set'],
  ['neru update', 'Update the CLI'],
]

const FLAGS: [string, string][] = [
  ['--permission-mode <default|plan|acceptEdits|auto|bypassPermissions>', 'How much Neru may do without asking'],
  ['--dangerously-skip-permissions', 'Run every tool without asking'],
  ['--model <name>', 'Use this model for this run'],
  ['--fallback-model <names>', 'Models to switch to when the model is limited or down'],
  ['--output-format <text|json|stream-json>', 'Output with -p, shaped like Claude Code\'s'],
  ['--allowedTools, --disallowedTools <rules>', 'Allow or block tools, e.g. "Bash(npm test:*) Edit"'],
  ['--max-turns <n>', 'Most tool rounds per request'],
  ['--system-prompt, --append-system-prompt <text>', 'Replace or add to the system prompt'],
  ['--add-dir <dir>', 'Also let Neru work in this folder'],
  ['--session-id <uuid>', 'Resume or start this session'],
  ['--effort <low|medium|high>', 'Reasoning effort'],
  ['--cwd <dir>', 'Work in another folder'],
  ['--no-web', 'Turn off web search and fetch'],
  ['--verbose', 'Show timings and extra detail'],
  ['-v, -h', 'Version, help'],
]

const ENV: [string, string][] = [
  ['NERU_API_KEY', 'Connect a model without neru login, for CI and scripts'],
  ['NERU_PROVIDER, NERU_MODEL, NERU_BASE_URL', 'Which provider and model the key is for'],
  ['NO_COLOR', 'Plain output without colors'],
]

const SLASH: [string, string[]][] = [
  ['Session', ['/clear', '/resume', '/fork', '/rename', '/rewind', '/compact', '/export', '/copy', '/exit']],
  ['Model & mode', ['/model', '/mode', '/plan', '/effort', '/login', '/logout', '/web', '/output-style']],
  ['Project', ['/init', '/review', '/security-review', '/pr-comments', '/diff', '/memory', '/todos', '/permissions', '/hooks', '/agents', '/trust']],
  ['Tools', ['/mcp', '/skills', '/bashes', '/doctor', '/cost', '/context', '/status', '/statusline', '/config', '/release-notes', '/update', '/bug', '/terminal-setup', '/help']],
]

const KEYS: [string, string][] = [
  ['/', 'Commands'],
  ['@', 'Mention a file'],
  ['!', 'Run a shell command'],
  ['#', 'Remember something'],
  ['Shift+Tab', 'Cycle the permission mode'],
  ['Esc', 'Interrupt'],
  ['Esc Esc', 'Rewind'],
  ['Shift+Enter, Ctrl+J, \\', 'New line'],
  ['↑ ↓', 'History'],
  ['Tab', 'Complete'],
  ['Ctrl+L', 'Clear the screen'],
  ['Ctrl+C twice', 'Quit'],
]

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false)
  const timer = useRef<number | undefined>(undefined)
  useEffect(() => () => window.clearTimeout(timer.current), [])
  const copy = () => { void navigator.clipboard.writeText(text).then(() => { setCopied(true); window.clearTimeout(timer.current); timer.current = window.setTimeout(() => setCopied(false), 1500) }).catch(() => undefined) }
  return <button type="button" className="icon-button small" onClick={copy} aria-label={copied ? 'Copied' : 'Copy command'} title={copied ? 'Copied' : 'Copy'}>{copied ? <Check size={14} /> : <Copy size={14} />}</button>
}

export function CommandBlock({ label, command }: { label?: string; command: string }) {
  return <div className="cli-command">{label && <span className="cli-command-label">{label}</span>}<div className="cli-command-line"><code>{command}</code><CopyButton text={command} /></div></div>
}

function Table({ rows }: { rows: [string, string][] }) {
  return <dl className="cli-table">{rows.map(([term, text]) => <div key={term}><dt><code>{term}</code></dt><dd>{text}</dd></div>)}</dl>
}

/** Settings → CLI: the neru command, how to install it, and what it can do. */
export function CliSettings({ onError, onNotice }: { onError: (message: string) => void; onNotice: (message: string) => void }) {
  const desktop = isTauri()
  const [os, setOs] = useState<Os>(currentOs)
  const [status, setStatus] = useState<CliStatus | null>(null)
  const [checking, setChecking] = useState(desktop)
  const [installing, setInstalling] = useState(false)

  useEffect(() => {
    if (!desktop) return
    let active = true
    api.cliStatus().then(value => { if (active) setStatus(value) }).catch(cause => onError(String(cause))).finally(() => { if (active) setChecking(false) })
    return () => { active = false }
  }, [desktop, onError])

  const install = async () => {
    setInstalling(true)
    try { const next = await api.cliInstallPath(); setStatus(next); onNotice(next.note ?? 'neru is on your PATH.') } catch (cause) { onError(String(cause)) } finally { setInstalling(false) }
  }

  const summary = !desktop ? 'Checking needs the desktop app.' : checking ? 'Checking…' : status?.onPath ? `${status.version ? `Version ${status.version}` : 'Installed'} · ${status.path}` : 'Not found on your PATH.'
  return <section className="settings-section cli-settings"><h2>Neru in your terminal</h2>
    <p className="settings-lede">The <code>neru</code> command runs the same agent in any terminal, with the commands and flags of Claude Code. It shares settings, keys, sessions, memory, skills and connectors with this app.</p>
    <div className="settings-row">
      <div><strong><SquareTerminal size={15} className="cli-status-icon" /> neru command {status?.onPath && <span className="cli-badge">On PATH</span>}</strong><p>{summary}</p>{status?.note && <p>{status.note}</p>}</div>
      {desktop && <button type="button" className="button subtle" onClick={() => void install()} disabled={installing || checking}>{installing && <LoaderCircle size={14} className="animate-spin" />}{status?.onPath ? 'Repair' : 'Add neru to PATH'}</button>}
    </div>

    <h3 className="cli-heading">Install</h3>
    <div className="segmented" role="tablist" aria-label="Operating system">{OS_TABS.map(tab => <button key={tab.id} type="button" role="tab" aria-selected={os === tab.id} className={os === tab.id ? 'active' : ''} onClick={() => setOs(tab.id)}>{tab.label}</button>)}</div>
    <div className="cli-commands" role="tabpanel">{INSTALL[os].map(item => <CommandBlock key={item.command} label={item.label} command={item.command} />)}</div>
    <p className="cli-note">Then run <code>neru login</code> once, or set <code>NERU_API_KEY</code>. Installed with this app? {os === 'windows' ? <>The installer already added <code>neru</code>.</> : <>Use Add neru to PATH above.</>}</p>
    {os === 'linux' && <p className="cli-note">Needs glibc 2.35 or later (Ubuntu 22.04, Debian 12, Fedora 36 and newer) and the WebKitGTK 4.1 runtime (<code>sudo apt install libwebkit2gtk-4.1-0</code>). Over SSH, in CI or in a container, install <code>xvfb</code> too and neru uses it by itself.</p>}

    <h3 className="cli-heading">Update and uninstall</h3>
    <Table rows={UPDATE[os]} />
    <p className="cli-note">Uninstalling keeps your settings and sessions.</p>

    <h3 className="cli-heading">Usage</h3>
    <Table rows={USAGE} />

    <h3 className="cli-heading">Flags</h3>
    <Table rows={FLAGS} />

    <h3 className="cli-heading">Environment</h3>
    <Table rows={ENV} />

    <h3 className="cli-heading">In a session</h3>
    <dl className="cli-table cli-slash">{SLASH.map(([group, names]) => <div key={group}><dt>{group}</dt><dd>{names.map(name => <code key={name}>{name}</code>)}</dd></div>)}</dl>
    <h3 className="cli-heading">Input shortcuts</h3>
    <dl className="cli-table cli-keys">{KEYS.map(([key, text]) => <div key={key}><dt>{key.split(', ').map(part => <kbd key={part}>{part}</kbd>)}</dt><dd>{text}</dd></div>)}</dl>
  </section>
}
