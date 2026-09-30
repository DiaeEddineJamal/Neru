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

const INSTALL: Record<Os, { label: string; command: string }[]> = {
  windows: [
    { label: 'PowerShell', command: `irm ${REPO}/install.ps1 | iex` },
    { label: 'winget, CLI only', command: 'winget install Luziv.Neru.CLI' },
    { label: 'winget, the desktop app (also adds neru)', command: 'winget install Luziv.Neru' },
    { label: 'Any OS with Node.js 18+', command: 'npm install -g neru-cli' },
  ],
  macos: [
    { label: 'Terminal', command: `curl -fsSL ${REPO}/install.sh | bash` },
    { label: 'Any OS with Node.js 18+', command: 'npm install -g neru-cli' },
  ],
  linux: [
    { label: 'Terminal', command: `curl -fsSL ${REPO}/install.sh | bash` },
    { label: 'Any OS with Node.js 18+', command: 'npm install -g neru-cli' },
  ],
}

const USAGE: [string, string][] = [
  ['neru', 'Start a session in this folder'],
  ['neru "fix the failing test"', 'Start with a first message'],
  ['neru -p "explain this project"', 'Print the answer and exit; reads piped stdin'],
  ['cat log.txt | neru -p "why does this fail?"', 'Ask about piped output'],
  ['neru -c', 'Continue the latest session'],
  ['neru -r [search]', 'Resume a session, optionally filtered'],
  ['neru login', 'Connect a model provider'],
  ['neru models', 'List the models you can use'],
  ['neru mcp list|add|remove', 'Manage connectors'],
  ['neru sessions', 'List sessions'],
  ['neru doctor', 'Check the setup'],
  ['neru update', 'Update the CLI'],
]

const FLAGS: [string, string][] = [
  ['--mode <review|plan|edits|auto|bypass>', 'Permission mode (alias --permission-mode)'],
  ['--dangerously-skip-permissions', 'Run tools without asking'],
  ['--model <name>', 'Use this model'],
  ['--effort <low|medium|high>', 'Reasoning effort'],
  ['--output-format <text|json|stream-json>', 'Output with -p'],
  ['--append-system-prompt <text>', 'Add to the system prompt'],
  ['--cwd <dir>', 'Work in another folder'],
  ['--no-web', 'Turn off web search and fetch'],
  ['--verbose', 'Show more detail'],
  ['-v, -h', 'Version, help'],
]

const SLASH: [string, string[]][] = [
  ['Session', ['/clear', '/resume', '/fork', '/rename', '/rewind', '/compact', '/export', '/copy', '/exit']],
  ['Model & mode', ['/model', '/mode', '/plan', '/effort', '/login', '/logout', '/web']],
  ['Project', ['/init', '/review', '/security-review', '/diff', '/memory', '/todos', '/permissions', '/hooks']],
  ['Tools', ['/mcp', '/skills', '/doctor', '/cost', '/context', '/status', '/config', '/release-notes', '/update', '/bug', '/terminal-setup', '/help']],
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

function CommandBlock({ label, command }: { label?: string; command: string }) {
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
    <p className="settings-lede">The <code>neru</code> command runs the same agent in any terminal. It shares settings, keys, sessions, memory, skills and connectors with this app.</p>
    <div className="settings-row">
      <div><strong><SquareTerminal size={15} className="cli-status-icon" /> neru command {status?.onPath && <span className="cli-badge">On PATH</span>}</strong><p>{summary}</p>{status?.note && <p>{status.note}</p>}</div>
      {desktop && <button type="button" className="button subtle" onClick={() => void install()} disabled={installing || checking}>{installing && <LoaderCircle size={14} className="animate-spin" />}{status?.onPath ? 'Repair' : 'Add neru to PATH'}</button>}
    </div>

    <h3 className="cli-heading">Install</h3>
    <div className="segmented" role="tablist" aria-label="Operating system">{OS_TABS.map(tab => <button key={tab.id} type="button" role="tab" aria-selected={os === tab.id} className={os === tab.id ? 'active' : ''} onClick={() => setOs(tab.id)}>{tab.label}</button>)}</div>
    <div className="cli-commands" role="tabpanel">{INSTALL[os].map(item => <CommandBlock key={item.command} label={item.label} command={item.command} />)}</div>
    <p className="cli-note">Installed with this app? On Windows the installer already added <code>neru</code>. On macOS and Linux, use Add neru to PATH above.</p>
    {os === 'linux' && <p className="cli-note">The CLI needs the WebKitGTK 4.1 runtime (<code>sudo apt install libwebkit2gtk-4.1-0</code> on Debian and Ubuntu) and a desktop session.</p>}

    <h3 className="cli-heading">Update and uninstall</h3>
    <Table rows={[
      ['neru update', 'Updates a script install'],
      ['npm install -g neru-cli@latest', 'Updates an npm install'],
      ['winget upgrade Luziv.Neru.CLI', 'Updates a winget install'],
      ['npm uninstall -g neru-cli', 'Removes an npm install'],
      ['winget uninstall Luziv.Neru.CLI', 'Removes a winget install'],
    ]} />
    <p className="cli-note">To remove a script install, delete {os === 'windows' ? <code>%LOCALAPPDATA%\Neru\cli</code> : <><code>~/.neru</code> and <code>~/.local/bin/neru</code></>}.</p>

    <h3 className="cli-heading">Usage</h3>
    <Table rows={USAGE} />

    <h3 className="cli-heading">Flags</h3>
    <Table rows={FLAGS} />

    <h3 className="cli-heading">In a session</h3>
    <dl className="cli-table cli-slash">{SLASH.map(([group, names]) => <div key={group}><dt>{group}</dt><dd>{names.map(name => <code key={name}>{name}</code>)}</dd></div>)}</dl>
    <h3 className="cli-heading">Input shortcuts</h3>
    <dl className="cli-table cli-keys">{KEYS.map(([key, text]) => <div key={key}><dt>{key.split(', ').map(part => <kbd key={part}>{part}</kbd>)}</dt><dd>{text}</dd></div>)}</dl>
  </section>
}
