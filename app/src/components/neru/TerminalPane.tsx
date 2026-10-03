import { useEffect, useRef, useState } from 'react'
import { Plus, SquareTerminal, X } from 'lucide-react'
import { listen } from '@tauri-apps/api/event'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'
import { api } from '../../api'
import { playBanner } from '@/lib/terminalBanner'

let agentFeed = ''
const listeners = new Set<(text: string) => void>()

/** Remembers agent command output so the terminal can show it even if it opens later. */
export function pushAgentOutput(command: string, output: string, phase: string) {
  const text = phase === 'running'
    ? `\r\n\x1b[32mneru$\x1b[0m ${command}\r\n`
    : `${output.replace(/\n/g, '\r\n')}\r\n`
  agentFeed += text
  if (agentFeed.length > 200_000) agentFeed = agentFeed.slice(-160_000)
  listeners.forEach(listener => listener(text))
}

export interface TerminalLaunchSpec { program: string; args: string[]; env: [string, string][] }
/** A tab to open: a shell in `cwd`, or a program such as a Team member's own CLI. */
export interface TerminalTabSpec { title: string; cwd?: string; launch?: TerminalLaunchSpec; scope?: string }

const openers = new Set<(spec: TerminalTabSpec) => void>()
/** Tabs asked for before the pane mounted; it opens them when it does. */
const pending: TerminalTabSpec[] = []
/** Opens a tab in the terminal pane (the caller also opens the pane). */
export function openTerminalTab(spec: TerminalTabSpec) {
  if (openers.size === 0) pending.push(spec)
  else openers.forEach(open => open(spec))
}

/** One shell. The first tab plays the welcome and mirrors the agent's commands; later tabs are plain shells. */
function TerminalSession({ projectKey, first, active, onTitle, cwd, launch }: { projectKey: string; first: boolean; active: boolean; onTitle: (title: string) => void; cwd?: string; launch?: TerminalLaunchSpec }) {
  const host = useRef<HTMLDivElement>(null)
  const [error, setError] = useState('')
  const focus = useRef<() => void>(() => undefined)
  useEffect(() => { if (active) window.setTimeout(() => focus.current(), 30) }, [active])

  useEffect(() => {
    if (!host.current) return
    const terminal = new Terminal({
      theme: {
        background: '#151515', foreground: '#e9e8e1', cursor: '#8ea291', cursorAccent: '#151515', selectionBackground: '#344a3999',
        black: '#1b1d19', red: '#e0786b', green: '#8ea291', yellow: '#d9b870', blue: '#7fa3c7', magenta: '#b99bc9', cyan: '#7fb5ad', white: '#d8d6ce',
        brightBlack: '#6c7068', brightRed: '#f0968a', brightGreen: '#a9c2ac', brightYellow: '#ecd08f', brightBlue: '#9dbde0', brightMagenta: '#d0b5de', brightCyan: '#9dcfc7', brightWhite: '#f5f4ef',
      },
      fontFamily: "'JetBrains Mono', Consolas, monospace", fontSize: 13, lineHeight: 1, letterSpacing: 0,
      cursorBlink: true, cursorStyle: 'bar', cursorWidth: 2, scrollback: 5000, allowProposedApi: true,
    })
    const fit = new FitAddon()
    terminal.loadAddon(fit)
    terminal.open(host.current)
    fit.fit()
    // Shell output waits behind the welcome animation so the two never interleave.
    let held: string[] | null = []
    const out = (text: string) => { if (held) held.push(text); else terminal.write(text) }
    const reduce = window.matchMedia('(prefers-reduced-motion: reduce)').matches
    focus.current = () => { fit.fit(); terminal.focus() }
    terminal.onTitleChange(title => { if (title.trim()) onTitle(title.trim().split(/[\\/]/).pop() ?? title) })
    void (first ? playBanner(terminal, projectKey, !reduce) : Promise.resolve()).finally(() => {
      if (first && agentFeed) terminal.write(agentFeed)
      const queued = held ?? []
      held = null
      queued.forEach(text => terminal.write(text))
      terminal.focus()
    })
    const onAgent = (text: string) => out(text)
    if (first) listeners.add(onAgent)
    let id = ''
    let disposed = false
    const stops: Array<() => void> = []
    const resize = new ResizeObserver(() => { fit.fit(); if (id) void api.terminalResize(id, terminal.cols, terminal.rows) })
    resize.observe(host.current)
    void (async () => {
      try {
        const stopOutput = await listen<{ id: string; data: string }>('terminal-output', event => { if (event.payload.id === id) out(event.payload.data) })
        stops.push(stopOutput)
        const stopExit = await listen<string>('terminal-exit', event => { if (event.payload === id) out('\r\n\x1b[2m[session ended — close this tab or open a new one]\x1b[0m\r\n') })
        stops.push(stopExit)
        if (disposed) return
        id = await api.terminalStart(cwd, launch)
        await api.terminalResize(id, terminal.cols, terminal.rows)
        terminal.onData(data => { if (id) void api.terminalWrite(id, data) })
      } catch (cause) { setError(`The shell could not start: ${String(cause)}. Check that PowerShell is installed, then open a new tab.`) }
    })()
    return () => { disposed = true; listeners.delete(onAgent); resize.disconnect(); stops.forEach(stop => stop()); if (id) void api.terminalStop(id); terminal.dispose() }
    // Mounted once; props only change which tab is shown.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [projectKey])

  return <div className="terminal-session" hidden={!active}>{error && <div className="inline-error">{error}</div>}<div ref={host} className="terminal-host" /></div>
}

interface Tab { id: number; title: string; scope: string; cwd?: string; launch?: TerminalLaunchSpec }

/** Which tabs show: the project's, or one Team task's (opened in its folder). */
export interface TerminalScope { key: string; cwd?: string; label?: string }

const SAVED = 'neru.terminals.v1'
type SavedTab = { title: string; cwd?: string; launch?: TerminalLaunchSpec }
const readSaved = (): Record<string, SavedTab[]> => { try { return JSON.parse(localStorage.getItem(SAVED) ?? '{}') as Record<string, SavedTab[]> } catch { return {} } }

/** The integrated terminal with tabs. Shells keep running while you switch tabs or leave the view.
 * A Team task has its own tabs, opened in its folder and reopened with it after a restart. */
export function TerminalPane({ projectKey, scope = { key: projectKey } }: { projectKey: string; scope?: TerminalScope }) {
  const next = useRef(2)
  const shell = 'PowerShell'
  const [tabs, setTabs] = useState<Tab[]>([])
  const [activeFor, setActiveFor] = useState<Record<string, number>>({})
  const mirrorId = tabs.find(tab => tab.scope === projectKey)?.id
  const shown = tabs.filter(tab => tab.scope === scope.key)
  const active = activeFor[scope.key] ?? shown[0]?.id ?? 0
  const setActive = (id: number, key = scope.key) => setActiveFor(current => ({ ...current, [key]: id }))
  const add = (spec?: TerminalTabSpec) => {
    const id = next.current++
    const key = spec?.scope ?? scope.key
    setTabs(current => [...current, { id, title: spec?.title ?? shell, scope: key, cwd: spec?.cwd ?? (key === scope.key ? scope.cwd : undefined), launch: spec?.launch }])
    setActive(id, key)
  }
  const close = (id: number) => setTabs(current => {
    const left = current.filter(tab => tab.id !== id)
    if (!left.some(tab => tab.scope === scope.key)) { const fresh = next.current++; setActive(fresh); return [...left, { id: fresh, title: shell, scope: scope.key, cwd: scope.cwd }] }
    if (id === active) { const mine = current.filter(tab => tab.scope === scope.key); setActive(mine[Math.max(0, mine.findIndex(tab => tab.id === id) - 1)].id) }
    return left
  })
  // A scope seen for the first time gets its saved tabs back, or one shell in its folder.
  const restoredScopes = useRef(new Set<string>())
  useEffect(() => {
    // Once per scope, even when React runs effects twice in development.
    if (restoredScopes.current.has(scope.key)) return
    restoredScopes.current.add(scope.key)
    const saved = scope.key === projectKey ? [] : readSaved()[scope.key] ?? []
    const restored = (saved.length ? saved : [{ title: shell, cwd: scope.cwd }]).map(item => ({ ...item, id: next.current++, scope: scope.key }))
    setTabs(current => current.some(tab => tab.scope === scope.key) ? current : [...current, ...restored])
    setActiveFor(current => current[scope.key] ? current : { ...current, [scope.key]: restored[0].id })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scope.key])
  // Task tabs are remembered so the task reopens with them.
  useEffect(() => {
    const saved = readSaved()
    for (const key of new Set(tabs.map(tab => tab.scope))) if (key !== projectKey) saved[key] = tabs.filter(tab => tab.scope === key).map(({ title, cwd, launch }) => ({ title, cwd, launch }))
    try { localStorage.setItem(SAVED, JSON.stringify(saved)) } catch { /* storage unavailable */ }
  }, [tabs, projectKey])
  useEffect(() => {
    const open = (spec: TerminalTabSpec) => add(spec)
    openers.add(open)
    pending.splice(0).forEach(open)
    return () => { openers.delete(open) }
  })
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.ctrlKey && event.shiftKey && event.key.toLowerCase() === 't') { event.preventDefault(); add() }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })
  return <div className="terminal-shell">
    <div className="terminal-tabs" role="tablist" aria-label={scope.label ? `Terminals for ${scope.label}` : 'Terminals'}>
      {scope.label && <span className="terminal-scope" title={scope.cwd}>{scope.label}</span>}
      {shown.map(tab => { const mirror = tab.id === mirrorId; return <div key={tab.id} className={`terminal-tab${tab.id === active ? ' active' : ''}`} role="tab" aria-selected={tab.id === active}>
        <button type="button" className="terminal-tab-label" onClick={() => setActive(tab.id)} title={mirror ? 'Project shell; Neru\'s commands appear here' : tab.cwd ?? tab.title}><SquareTerminal size={13} aria-hidden /><span className="truncate">{mirror ? `${tab.title} · Neru` : tab.title}</span></button>
        <button type="button" className="terminal-tab-close" aria-label={`Close ${tab.title}`} onClick={() => close(tab.id)}><X size={12} /></button>
      </div> })}
      <button type="button" className="terminal-tab-add" aria-label="New terminal" title="New terminal (Ctrl+Shift+T)" onClick={() => add()}><Plus size={14} /></button>
    </div>
    {tabs.map(tab => <TerminalSession key={tab.id} projectKey={projectKey} cwd={tab.cwd} launch={tab.launch} first={tab.id === mirrorId} active={tab.id === active && tab.scope === scope.key} onTitle={title => setTabs(current => current.map(item => item.id === tab.id && !item.launch ? { ...item, title } : item))} />)}
  </div>
}
