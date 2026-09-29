import { useEffect, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import '@xterm/xterm/css/xterm.css'
import { api } from '../../api'

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

export function TerminalPane({ projectKey }: { projectKey: string }) {
  const host = useRef<HTMLDivElement>(null)
  const [error, setError] = useState('')

  useEffect(() => {
    if (!host.current) return
    const terminal = new Terminal({ theme: { background: '#111310', foreground: '#e9e9e1', cursor: '#8ea291', selectionBackground: '#344a39' }, fontFamily: 'JetBrains Mono, monospace', fontSize: 12, cursorBlink: true, allowProposedApi: true })
    const fit = new FitAddon()
    terminal.loadAddon(fit)
    terminal.open(host.current)
    fit.fit()
    if (agentFeed) terminal.write(agentFeed)
    const onAgent = (text: string) => terminal.write(text)
    listeners.add(onAgent)
    let id = ''
    let disposed = false
    const stops: Array<() => void> = []
    const resize = new ResizeObserver(() => { fit.fit(); if (id) void api.terminalResize(id, terminal.cols, terminal.rows) })
    resize.observe(host.current)
    void (async () => {
      try {
        const stopOutput = await listen<{ id: string; data: string }>('terminal-output', event => { if (event.payload.id === id) terminal.write(event.payload.data) })
        stops.push(stopOutput)
        const stopExit = await listen<string>('terminal-exit', event => { if (event.payload === id) terminal.writeln('\r\n[session ended]') })
        stops.push(stopExit)
        if (disposed) return
        id = await api.terminalStart()
        await api.terminalResize(id, terminal.cols, terminal.rows)
        terminal.onData(data => { if (id) void api.terminalWrite(id, data) })
      } catch (cause) { setError(String(cause)) }
    })()
    return () => { disposed = true; listeners.delete(onAgent); resize.disconnect(); stops.forEach(stop => stop()); if (id) void api.terminalStop(id); terminal.dispose() }
  }, [projectKey])

  return <div className="terminal-shell"><div className="terminal-header"><span className="status-dot" /> Integrated terminal <span className="muted">· project shell and Neru commands</span></div>{error && <div className="inline-error">{error}</div>}<div ref={host} className="terminal-host" /></div>
}
