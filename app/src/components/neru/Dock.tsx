import { useRef, useState, type PointerEvent as ReactPointerEvent, type ReactNode } from 'react'
import { Maximize2, Minimize2, X } from 'lucide-react'
import { cn } from '@/lib/utils'

export type PaneId = 'terminal' | 'files' | 'changes' | 'browser'

export interface DockPaneSpec {
  id: PaneId
  title: string
  /** Replaces the plain title in the pane header (tabs, for example). */
  header?: ReactNode
  /** Extra header buttons, shown before expand and close. */
  actions?: ReactNode
  body: ReactNode
  /** Keep the body mounted while the pane is closed (shells and the browser keep running). */
  keepAlive?: boolean
}

const WIDTH_KEY = 'neru.dock.width'
const MIN = 320
const readWidth = () => { try { const value = Number(localStorage.getItem(WIDTH_KEY)); return value >= MIN ? value : 640 } catch { return 640 } }

/**
 * The panes beside the conversation. Terminal, files and changes stack in one column; the browser
 * gets its own column, so opening all of them lays out like Claude Code's workspace.
 */
export function Dock({ panes, open, expanded, onExpand, onClose }: {
  panes: DockPaneSpec[]
  open: Record<PaneId, boolean>
  expanded: PaneId | null
  onExpand: (id: PaneId | null) => void
  onClose: (id: PaneId) => void
}) {
  const [width, setWidth] = useState(readWidth)
  const [resizing, setResizing] = useState(false)
  const root = useRef<HTMLDivElement>(null)
  const visible = (id: PaneId) => open[id] && (!expanded || expanded === id)
  const stackVisible = (['terminal', 'files', 'changes'] as PaneId[]).some(visible)
  const browserVisible = visible('browser')
  const anyOpen = stackVisible || browserVisible

  const startResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    event.preventDefault()
    const startX = event.clientX, from = width
    const limit = () => Math.max(MIN, (root.current?.parentElement?.clientWidth ?? 1200) - 360)
    let next = from
    setResizing(true)
    const move = (moved: PointerEvent) => { next = Math.max(MIN, Math.min(limit(), from + startX - moved.clientX)); setWidth(next) }
    const up = () => { setResizing(false); try { localStorage.setItem(WIDTH_KEY, String(next)) } catch { /* storage unavailable */ } window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up) }
    window.addEventListener('pointermove', move); window.addEventListener('pointerup', up)
  }

  const render = (spec: DockPaneSpec) => {
    const shown = visible(spec.id)
    if (!shown && !spec.keepAlive) return null
    return <section key={spec.id} className={cn('dock-pane', `dock-${spec.id}`)} hidden={!shown} aria-label={spec.title}>
      <header className="dock-head">
        <div className="dock-head-main">{spec.header ?? <strong>{spec.title}</strong>}</div>
        {spec.actions}
        <button type="button" className="dock-button" aria-label={expanded === spec.id ? 'Restore' : 'Expand'} title={expanded === spec.id ? 'Restore' : 'Expand'} onClick={() => onExpand(expanded === spec.id ? null : spec.id)}>{expanded === spec.id ? <Minimize2 size={15} /> : <Maximize2 size={15} />}</button>
        <button type="button" className="dock-button" aria-label={`Close ${spec.title}`} title="Close" onClick={() => onClose(spec.id)}><X size={16} /></button>
      </header>
      <div className="dock-body">{spec.body}</div>
    </section>
  }

  const stack = panes.filter(spec => spec.id !== 'browser')
  const browser = panes.find(spec => spec.id === 'browser')
  return <div ref={root} className={cn('dock', resizing && 'resizing', expanded && 'expanded')} style={{ width: expanded ? undefined : width }} hidden={!anyOpen}>
    {anyOpen && !expanded && <div className="dock-resizer" onPointerDown={startResize} onDoubleClick={() => { setWidth(640); try { localStorage.setItem(WIDTH_KEY, '640') } catch { /* storage unavailable */ } }} role="separator" aria-orientation="vertical" aria-label="Resize panels" />}
    <div className="dock-col dock-stack" hidden={!stackVisible}>{stack.map(render)}</div>
    <div className="dock-col dock-web" hidden={!browserVisible}>{browser && render(browser)}</div>
  </div>
}
