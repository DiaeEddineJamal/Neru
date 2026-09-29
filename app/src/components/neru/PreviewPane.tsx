import { useEffect, useState } from 'react'
import { listen } from '@tauri-apps/api/event'
import { ExternalLink, LoaderCircle, Square } from 'lucide-react'
import { api } from '../../api'

export function PreviewPane({ projectKey, onOpenExternal }: { projectKey: string; onOpenExternal: (url: string) => void }) {
  const [url, setUrl] = useState('http://127.0.0.1:5173')
  const [shown, setShown] = useState('')
  const [hint, setHint] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  useEffect(() => {
    setShown('')
    setError('')
    void api.previewHint().then(value => { setHint(value.command); setUrl(value.url) }).catch(() => setHint(''))
    const unlisten = listen<string>('preview://open', event => { setUrl(event.payload); setShown(event.payload); setError('') })
    return () => { void unlisten.then(stop => stop()) }
  }, [projectKey])

  const start = async () => {
    setBusy(true)
    setError('')
    try {
      const value = await api.previewStart()
      setUrl(value.url)
      setShown(value.url)
      setHint(value.command)
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause))
    } finally {
      setBusy(false)
    }
  }

  return <div className="preview-pane">
    <div className="preview-bar">
      <input value={url} onChange={event => setUrl(event.target.value)} onKeyDown={event => { if (event.key === 'Enter') setShown(url) }} aria-label="Preview URL" />
      <button className="button subtle" onClick={() => setShown(url)}>Open</button>
      <button className="button primary" disabled={busy} onClick={() => void start()}>{busy ? <LoaderCircle size={14} className="animate-spin" /> : null} Start dev server</button>
      <button className="button subtle" onClick={() => void api.previewStop()} title="Stop the dev server Neru started"><Square size={14} /> Stop</button>
      <button className="icon-button" aria-label="Open in browser" title="Open in browser" onClick={() => onOpenExternal(shown || url)}><ExternalLink size={15} /></button>
    </div>
    {hint && <p className="preview-hint">Dev command: {hint}. The page loads here when the server is up.</p>}
    {error && <p className="clone-error">{error}</p>}
    {shown ? <iframe className="preview-frame" title="App preview" src={shown} /> : <div className="empty-pane"><p>Start the dev server, or paste a localhost or https URL.</p></div>}
  </div>
}
