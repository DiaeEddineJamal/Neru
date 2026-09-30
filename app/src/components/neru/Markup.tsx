import { useCallback, useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type PointerEvent as ReactPointerEvent } from 'react'
import { motion, useReducedMotion } from 'motion/react'
import { Circle, MousePointer2, MoveUpRight, PenLine, Redo2, Send, Slash, Square, Trash2, Type, Undo2 } from 'lucide-react'
import { SPRING_PANEL } from '@/lib/ease'

type Tool = 'pen' | 'line' | 'arrow' | 'rect' | 'ellipse' | 'text'
type Point = [number, number]
type Shape =
  | { id: number; tool: 'pen'; color: string; width: number; points: Point[] }
  | { id: number; tool: 'line' | 'arrow' | 'rect' | 'ellipse'; color: string; width: number; from: Point; to: Point }
  | { id: number; tool: 'text'; color: string; size: number; at: Point; text: string }

const COLORS = [
  { value: '#ef4444', name: 'Red' },
  { value: '#3b82f6', name: 'Blue' },
  { value: '#22c55e', name: 'Green' },
  { value: '#111111', name: 'Black' },
  { value: '#ffffff', name: 'White' },
]
const TOOLS: { id: Tool; label: string; key: string; icon: typeof PenLine }[] = [
  { id: 'pen', label: 'Pen', key: 'p', icon: PenLine },
  { id: 'line', label: 'Line', key: 'l', icon: Slash },
  { id: 'arrow', label: 'Arrow', key: 'a', icon: MoveUpRight },
  { id: 'rect', label: 'Rectangle', key: 'r', icon: Square },
  { id: 'ellipse', label: 'Ellipse', key: 'o', icon: Circle },
  { id: 'text', label: 'Text', key: 't', icon: Type },
]

/** The arrowhead of a line from `from` to `to`, as three points. */
function arrowHead(from: Point, to: Point, width: number): Point[] {
  const angle = Math.atan2(to[1] - from[1], to[0] - from[0])
  const length = Math.max(12, width * 4.5)
  const spread = Math.PI / 7
  return [to, [to[0] - length * Math.cos(angle - spread), to[1] - length * Math.sin(angle - spread)], [to[0] - length * Math.cos(angle + spread), to[1] - length * Math.sin(angle + spread)]]
}

/** A smooth SVG path through freehand points (midpoint quadratic curves). */
function penPath(points: Point[]): string {
  if (points.length === 0) return ''
  if (points.length < 3) return `M${points[0][0]},${points[0][1]} ${points.map(([x, y]) => `L${x},${y}`).join(' ')}`
  let d = `M${points[0][0]},${points[0][1]}`
  for (let i = 1; i < points.length - 1; i += 1) {
    const [x, y] = points[i], [nx, ny] = points[i + 1]
    d += ` Q${x},${y} ${(x + nx) / 2},${(y + ny) / 2}`
  }
  const last = points[points.length - 1]
  return `${d} L${last[0]},${last[1]}`
}

function ShapeView({ shape }: { shape: Shape }) {
  const stroke = { stroke: shape.color, fill: 'none', strokeLinecap: 'round' as const, strokeLinejoin: 'round' as const }
  switch (shape.tool) {
    case 'pen': return <path d={penPath(shape.points)} strokeWidth={shape.width} {...stroke} />
    case 'line': return <line x1={shape.from[0]} y1={shape.from[1]} x2={shape.to[0]} y2={shape.to[1]} strokeWidth={shape.width} {...stroke} />
    case 'arrow': return <g><line x1={shape.from[0]} y1={shape.from[1]} x2={shape.to[0]} y2={shape.to[1]} strokeWidth={shape.width} {...stroke} /><polygon points={arrowHead(shape.from, shape.to, shape.width).map(p => p.join(',')).join(' ')} fill={shape.color} stroke={shape.color} strokeWidth={shape.width / 2} strokeLinejoin="round" /></g>
    case 'rect': return <rect x={Math.min(shape.from[0], shape.to[0])} y={Math.min(shape.from[1], shape.to[1])} width={Math.abs(shape.to[0] - shape.from[0])} height={Math.abs(shape.to[1] - shape.from[1])} rx={shape.width} strokeWidth={shape.width} {...stroke} />
    case 'ellipse': return <ellipse cx={(shape.from[0] + shape.to[0]) / 2} cy={(shape.from[1] + shape.to[1]) / 2} rx={Math.abs(shape.to[0] - shape.from[0]) / 2} ry={Math.abs(shape.to[1] - shape.from[1]) / 2} strokeWidth={shape.width} {...stroke} />
    case 'text': return <text x={shape.at[0]} y={shape.at[1]} fill={shape.color} fontSize={shape.size} fontFamily="Inter, 'Segoe UI', sans-serif" fontWeight={600} dominantBaseline="hanging" stroke={shape.color === '#ffffff' ? '#111' : '#fff'} strokeWidth={shape.size / 7} paintOrder="stroke" strokeLinejoin="round">{shape.text}</text>
  }
}

/** Paints the screenshot and every shape into one image for the agent. */
async function flatten(src: string, width: number, height: number, shapes: Shape[]): Promise<string> {
  const image = new Image()
  image.src = src
  await image.decode()
  const canvas = document.createElement('canvas')
  canvas.width = width
  canvas.height = height
  const ctx = canvas.getContext('2d')
  if (!ctx) throw new Error('Drawing is not available')
  ctx.drawImage(image, 0, 0, width, height)
  ctx.lineCap = 'round'
  ctx.lineJoin = 'round'
  for (const shape of shapes) {
    ctx.strokeStyle = shape.color
    ctx.fillStyle = shape.color
    if (shape.tool === 'text') {
      ctx.font = `600 ${shape.size}px Inter, 'Segoe UI', sans-serif`
      ctx.textBaseline = 'top'
      ctx.lineWidth = shape.size / 7
      ctx.strokeStyle = shape.color === '#ffffff' ? '#111' : '#fff'
      ctx.strokeText(shape.text, shape.at[0], shape.at[1])
      ctx.fillText(shape.text, shape.at[0], shape.at[1])
      continue
    }
    ctx.lineWidth = shape.width
    ctx.beginPath()
    if (shape.tool === 'pen') ctx.stroke(new Path2D(penPath(shape.points)))
    else if (shape.tool === 'line' || shape.tool === 'arrow') {
      ctx.moveTo(...shape.from); ctx.lineTo(...shape.to); ctx.stroke()
      if (shape.tool === 'arrow') {
        const [a, b, c] = arrowHead(shape.from, shape.to, shape.width)
        ctx.beginPath(); ctx.moveTo(...a); ctx.lineTo(...b); ctx.lineTo(...c); ctx.closePath(); ctx.fill()
      }
    } else if (shape.tool === 'rect') {
      ctx.roundRect(Math.min(shape.from[0], shape.to[0]), Math.min(shape.from[1], shape.to[1]), Math.abs(shape.to[0] - shape.from[0]), Math.abs(shape.to[1] - shape.from[1]), shape.width)
      ctx.stroke()
    } else {
      ctx.ellipse((shape.from[0] + shape.to[0]) / 2, (shape.from[1] + shape.to[1]) / 2, Math.abs(shape.to[0] - shape.from[0]) / 2, Math.abs(shape.to[1] - shape.from[1]) / 2, 0, 0, Math.PI * 2)
      ctx.stroke()
    }
  }
  return canvas.toDataURL('image/jpeg', 0.9)
}

/**
 * Draw on a snapshot of the page, like the Claude app's browser markup: pen, line, arrow,
 * rectangle, ellipse and text in five colors, with undo, redo and clear. "Add to chat" sends
 * the marked-up picture to the message box.
 */
export function MarkupBoard({ shot, onClose, onSelect, onSend }: {
  shot: { dataUrl: string; width: number; height: number }
  onClose: () => void
  /** The pointer tool: leave drawing and pick an element on the live page instead. */
  onSelect: () => void
  onSend?: (dataUrl: string) => void
}) {
  const reduce = useReducedMotion() ?? false
  const [tool, setTool] = useState<Tool>('pen')
  const [color, setColor] = useState(COLORS[0].value)
  // Snapshots of the drawing: undo and redo step through them, so clearing is one undoable step too.
  const [history, setHistory] = useState<{ past: Shape[][]; present: Shape[]; future: Shape[][] }>({ past: [], present: [], future: [] })
  const shapes = history.present
  const [draft, setDraft] = useState<Shape | null>(null)
  const [typing, setTyping] = useState<{ at: Point; screen: { x: number; y: number }; value: string } | null>(null)
  const [sending, setSending] = useState(false)
  const svg = useRef<SVGSVGElement>(null)
  const nextId = useRef(1)
  // Image pixels per screen pixel, so strokes look the same thickness at any zoom.
  const [scale, setScale] = useState(1)
  useLayoutEffect(() => {
    const node = svg.current
    if (!node) return
    const measure = () => { const box = node.getBoundingClientRect(); if (box.width) setScale(shot.width / box.width) }
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(node)
    return () => observer.disconnect()
  }, [shot.width])
  const width = 3.5 * scale

  const add = useCallback((shape: Shape) => setHistory(h => ({ past: [...h.past, h.present], present: [...h.present, shape], future: [] })), [])
  const undo = useCallback(() => setHistory(h => h.past.length ? { past: h.past.slice(0, -1), present: h.past[h.past.length - 1], future: [h.present, ...h.future] } : h), [])
  const redo = useCallback(() => setHistory(h => h.future.length ? { past: [...h.past, h.present], present: h.future[0], future: h.future.slice(1) } : h), [])
  const clear = useCallback(() => setHistory(h => h.present.length ? { past: [...h.past, h.present], present: [], future: [] } : h), [])

  const typingRef = useRef(typing)
  typingRef.current = typing
  const commitText = useCallback(() => {
    const current = typingRef.current
    if (current && current.value.trim()) add({ id: nextId.current++, tool: 'text', color, size: 20 * scale, at: current.at, text: current.value.trim() })
    typingRef.current = null
    setTyping(null)
  }, [add, color, scale])

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (typing) return
      const mod = event.ctrlKey || event.metaKey
      if (mod && event.key.toLowerCase() === 'z') { event.preventDefault(); if (event.shiftKey) redo(); else undo(); return }
      if (mod && event.key.toLowerCase() === 'y') { event.preventDefault(); redo(); return }
      if (event.key === 'Escape') { event.preventDefault(); onClose(); return }
      if (mod || event.altKey) return
      const picked = TOOLS.find(item => item.key === event.key.toLowerCase())
      if (picked) setTool(picked.id)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [typing, undo, redo, onClose])

  const toImage = (event: ReactPointerEvent): Point => {
    const box = svg.current!.getBoundingClientRect()
    return [(event.clientX - box.left) * (shot.width / box.width), (event.clientY - box.top) * (shot.height / box.height)]
  }

  const down = (event: ReactPointerEvent<SVGSVGElement>) => {
    if (event.button !== 0) return
    if (typing) { commitText(); return }
    const at = toImage(event)
    if (tool === 'text') {
      const box = svg.current!.getBoundingClientRect()
      setTyping({ at, screen: { x: event.clientX - box.left, y: event.clientY - box.top }, value: '' })
      return
    }
    // Keeps the stroke going when the pointer leaves the picture; not every pointer can be captured.
    try { event.currentTarget.setPointerCapture(event.pointerId) } catch { /* draw without capture */ }
    const id = nextId.current++
    setDraft(tool === 'pen' ? { id, tool, color, width, points: [at] } : { id, tool, color, width, from: at, to: at })
  }
  const move = (event: ReactPointerEvent<SVGSVGElement>) => {
    if (!draft) return
    const at = toImage(event)
    setDraft(current => !current ? current
      : current.tool === 'pen' ? { ...current, points: [...current.points, at] }
      : current.tool === 'text' ? current
      : { ...current, to: event.shiftKey && (current.tool === 'rect' || current.tool === 'ellipse') ? [current.from[0] + Math.sign(at[0] - current.from[0]) * Math.max(Math.abs(at[0] - current.from[0]), Math.abs(at[1] - current.from[1])), current.from[1] + Math.sign(at[1] - current.from[1]) * Math.max(Math.abs(at[0] - current.from[0]), Math.abs(at[1] - current.from[1]))] : at })
  }
  const up = () => {
    if (!draft) return
    const tiny = draft.tool === 'pen' ? draft.points.length < 2 : draft.tool !== 'text' && Math.hypot(draft.to[0] - draft.from[0], draft.to[1] - draft.from[1]) < 3 * scale
    if (!tiny) add(draft)
    setDraft(null)
  }

  const send = async () => {
    if (!onSend) return
    setSending(true)
    try { onSend(shapes.length ? await flatten(shot.dataUrl, shot.width, shot.height, shapes) : shot.dataUrl); onClose() } finally { setSending(false) }
  }

  return <div className="markup" role="dialog" aria-label="Mark up the page">
    <div className="markup-canvas">
      <div className="markup-sheet" style={{ '--ratio': shot.width / shot.height } as CSSProperties}>
        <img src={shot.dataUrl} alt="Snapshot of the page" draggable={false} />
        <svg ref={svg} viewBox={`0 0 ${shot.width} ${shot.height}`} data-tool={tool} onPointerDown={down} onPointerMove={move} onPointerUp={up} onPointerCancel={up}>
          {shapes.map(shape => <ShapeView key={shape.id} shape={shape} />)}
          {draft && <ShapeView shape={draft} />}
        </svg>
        {typing && <input className="markup-text" autoFocus value={typing.value} placeholder="Type, then Enter" style={{ left: typing.screen.x, top: typing.screen.y, color }}
          onChange={event => setTyping(current => current && { ...current, value: event.target.value })}
          onKeyDown={event => { if (event.key === 'Enter') { event.preventDefault(); commitText() } else if (event.key === 'Escape') { event.preventDefault(); setTyping(null) } }}
          onBlur={commitText} />}
      </div>
    </div>
    <motion.div className="markup-bar" role="toolbar" aria-label="Markup tools" initial={reduce ? false : { opacity: 0, y: 12, scale: 0.97 }} animate={{ opacity: 1, y: 0, scale: 1 }} transition={reduce ? { duration: 0 } : SPRING_PANEL}>
      <button type="button" className="markup-tool" title="Select an element" aria-label="Select an element" onClick={onSelect}><MousePointer2 size={15} /></button>
      {TOOLS.map(item => <button key={item.id} type="button" className={`markup-tool${tool === item.id ? ' on' : ''}`} title={`${item.label} (${item.key.toUpperCase()})`} aria-label={item.label} aria-pressed={tool === item.id} onClick={() => setTool(item.id)}><item.icon size={15} /></button>)}
      <i className="markup-sep" />
      {COLORS.map(item => <button key={item.value} type="button" className={`markup-color${color === item.value ? ' on' : ''}`} style={{ '--swatch': item.value } as CSSProperties} title={item.name} aria-label={item.name} aria-pressed={color === item.value} onClick={() => setColor(item.value)} />)}
      <i className="markup-sep" />
      <button type="button" className="markup-tool" title="Undo (Ctrl+Z)" aria-label="Undo" disabled={!history.past.length} onClick={undo}><Undo2 size={15} /></button>
      <button type="button" className="markup-tool" title="Redo (Ctrl+Shift+Z)" aria-label="Redo" disabled={!history.future.length} onClick={redo}><Redo2 size={15} /></button>
      <button type="button" className="markup-tool" title="Clear all" aria-label="Clear all" disabled={!shapes.length} onClick={clear}><Trash2 size={15} /></button>
      <i className="markup-sep" />
      <button type="button" className="markup-text-button" onClick={onClose}>Close</button>
      <button type="button" className="markup-send" disabled={!onSend || sending} onClick={() => void send()}><Send size={13} /> Add to chat</button>
    </motion.div>
  </div>
}
