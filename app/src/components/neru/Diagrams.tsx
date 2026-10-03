import { useEffect, useId, useState } from 'react'
import { Check, Copy, Maximize2, Minus, Plus, X } from 'lucide-react'

/** Shared shell: a titled block with copy and fullscreen, as Traycer shows diagrams and wireframes. */
function Frame({ label, code, children, full }: { label: string; code: string; children: React.ReactNode; full: React.ReactNode }) {
  const [copied, setCopied] = useState(false)
  const [open, setOpen] = useState(false)
  useEffect(() => {
    if (!open) return
    const close = (event: KeyboardEvent) => { if (event.key === 'Escape') setOpen(false) }
    window.addEventListener('keydown', close)
    return () => window.removeEventListener('keydown', close)
  }, [open])
  return <div className="diagram-block">
    <div className="diagram-bar">
      <span>{label}</span>
      <button type="button" onClick={() => { void navigator.clipboard.writeText(code); setCopied(true); window.setTimeout(() => setCopied(false), 1400) }} aria-label="Copy source" title="Copy source">{copied ? <Check size={13} /> : <Copy size={13} />}</button>
      <button type="button" onClick={() => setOpen(true)} aria-label="Fullscreen" title="Fullscreen"><Maximize2 size={13} /></button>
    </div>
    {children}
    {open && <div className="modal-backdrop diagram-full" onMouseDown={() => setOpen(false)}>
      <div className="diagram-full-body" onMouseDown={event => event.stopPropagation()}>
        <button type="button" className="icon-button diagram-close" onClick={() => setOpen(false)} aria-label="Close"><X size={16} /></button>
        {full}
      </div>
    </div>}
  </div>
}

/** A ```mermaid fence, drawn with Mermaid (loaded on first use) in the app's light or dark theme. */
export function MermaidBlock({ code }: { code: string }) {
  const id = useId().replace(/:/g, '')
  const [svg, setSvg] = useState('')
  const [error, setError] = useState('')
  const [zoom, setZoom] = useState(1)
  useEffect(() => {
    let cancelled = false
    void import('mermaid').then(async ({ default: mermaid }) => {
      const dark = document.documentElement.dataset.theme !== 'light'
      mermaid.initialize({ startOnLoad: false, theme: dark ? 'dark' : 'neutral', securityLevel: 'strict', fontFamily: 'Inter, sans-serif' })
      try {
        const { svg } = await mermaid.render(`mermaid-${id}`, code)
        if (!cancelled) { setSvg(svg); setError('') }
      } catch (cause) {
        if (!cancelled) setError(cause instanceof Error ? cause.message.split('\n')[0] : 'Mermaid parse error')
      }
    })
    return () => { cancelled = true }
  }, [code, id])
  if (error) return <div className="diagram-block diagram-error"><div className="diagram-bar"><span>Mermaid parse error</span></div><pre>{code}</pre><small>{error}</small></div>
  const drawing = <div className="diagram-svg" dangerouslySetInnerHTML={{ __html: svg }} />
  return <Frame label="Diagram" code={code} full={<>
    <div className="diagram-zoom"><button type="button" className="icon-button small" onClick={() => setZoom(value => Math.max(0.4, value - 0.2))} aria-label="Zoom out"><Minus size={13} /></button><span>{Math.round(zoom * 100)}%</span><button type="button" className="icon-button small" onClick={() => setZoom(value => Math.min(4, value + 0.2))} aria-label="Zoom in"><Plus size={13} /></button><button type="button" className="button subtle small" onClick={() => setZoom(1)}>Fit</button></div>
    <div className="diagram-canvas"><div style={{ transform: `scale(${zoom})`, transformOrigin: 'top center' }}>{drawing}</div></div>
  </>}>{svg ? drawing : <div className="diagram-loading">Drawing…</div>}</Frame>
}

/** A ```wireframe fence holding an HTML page, shown live in a sandbox: scripts run, but it cannot
 * navigate, submit forms, open windows or reach the app. */
export function WireframeBlock({ html }: { html: string }) {
  const frame = (className: string) => <iframe className={className} title="Wireframe" sandbox="allow-scripts" srcDoc={html} />
  return <Frame label="Wireframe" code={html} full={frame('wireframe-full')}><div className="wireframe-resize">{frame('wireframe-frame')}</div></Frame>
}
