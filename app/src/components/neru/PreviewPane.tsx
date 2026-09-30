import { useEffect, useRef, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { ExternalLink, LoaderCircle, Play, RotateCw, Square } from 'lucide-react'
import { api } from '../../api'

/**
 * The page itself. Neru's window is cross-origin isolated (for on-device speech), which blocks
 * ordinary iframes of other origins ("refused to connect"). A credentialless iframe may load
 * them; the attribute has to be set before the frame navigates, and React drops it when it is
 * rendered as an empty string, so it is set by hand before `src`.
 */
function PreviewFrame({ src }: { src: string }) {
  const frame = useRef<HTMLIFrameElement>(null)
  useEffect(() => {
    const node = frame.current
    if (!node) return
    node.setAttribute('credentialless', '')
    node.src = src
  }, [src])
  return <iframe ref={frame} className="preview-frame" title="App preview" />
}

/** The in-app browser. `run` changes when something asks for a fresh preview (e.g. "Open preview"). */
export function PreviewPane({ projectKey, run = 0, onOpenExternal }: { projectKey: string; run?: number; onOpenExternal: (url: string) => void }) {
  const [url, setUrl] = useState('')
  const [shown, setShown] = useState('')
  const [frameKey, setFrameKey] = useState(0)
  const [hint, setHint] = useState('')
  const [busy, setBusy] = useState(false)
  const [progress, setProgress] = useState('')
  const [error, setError] = useState('')
  const started = useRef(0)

  const show = (next: string) => { setUrl(next); setShown(next); setFrameKey(value => value + 1); setError('') }

  useEffect(() => {
    setShown(''); setError('')
    // A preview already running (started from chat or earlier) opens straight away.
    void api.previewCurrent().then(current => { if (current?.url) { show(current.url); setHint(current.command) } }).catch(() => undefined)
    void api.previewHint().then(value => { setHint(current => current || value.command); setUrl(current => current || value.url) }).catch(() => setHint(''))
    const stops = [
      listen<string>('preview://open', event => show(event.payload)),
      listen<string>('preview://status', event => setProgress(event.payload)),
    ]
    return () => { stops.forEach(stop => void stop.then(fn => fn())) }
  }, [projectKey])

  const start = async () => {
    setBusy(true); setError(''); setProgress('Getting the preview ready…')
    try {
      const value = await api.previewStart()
      show(value.url); setHint(value.command)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause))
    } finally { setBusy(false); setProgress('') }
  }

  useEffect(() => {
    if (run > 0 && run !== started.current) { started.current = run; void start() }
    // start only depends on stable setters
  }, [run])

  const stop = async () => { await api.previewStop(); setShown(''); setHint(current => current) }

  return <div className="preview-pane">
    <div className="preview-bar">
      <button className="icon-button" aria-label="Reload" title="Reload" disabled={!shown} onClick={() => setFrameKey(value => value + 1)}><RotateCw size={15} /></button>
      <input value={url} onChange={event => setUrl(event.target.value)} onKeyDown={event => { if (event.key === 'Enter') show(url) }} placeholder="http://127.0.0.1:5173" aria-label="Preview URL" spellCheck={false} />
      <button className="button primary" disabled={busy} onClick={() => void start()} title={hint ? `Runs ${hint}` : 'Start the preview'}>{busy ? <LoaderCircle size={14} className="animate-spin" /> : <Play size={14} />} {shown ? 'Restart' : 'Run preview'}</button>
      <button className="icon-button" onClick={() => void stop()} aria-label="Stop the preview server" title="Stop the preview server"><Square size={14} /></button>
      <button className="icon-button" aria-label="Open in browser" title="Open in browser" disabled={!(shown || url)} onClick={() => onOpenExternal(shown || url)}><ExternalLink size={15} /></button>
    </div>
    {(progress || hint) && <p className="preview-hint">{progress || `Runs ${hint}. The page loads here once it is up.`}</p>}
    {error && <pre className="clone-error preview-error">{error}</pre>}
    {shown
      ? <PreviewFrame key={frameKey} src={shown} />
      : <div className="empty-pane">{busy ? <><LoaderCircle size={22} className="animate-spin" /><p>{progress || 'Starting…'}</p></> : <p>Run the preview, or type a localhost or https URL.</p>}</div>}
  </div>
}
