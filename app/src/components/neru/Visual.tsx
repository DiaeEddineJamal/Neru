import { Component, memo, useCallback, useEffect, useRef, useState, type ErrorInfo, type PointerEvent as ReactPointerEvent, type ReactNode } from 'react'
import { Code, Table } from '@/components/icons'
import { formatTick, formatValue, niceTicks, parseChart, type ChartSpec } from '@/lib/visual/chart'
import { visualKind, type VisualKind } from '@/lib/visual/fence'
import { WIDGET_MAX_BYTES, WIDGET_MAX_HEIGHT, WIDGET_MIN_HEIGHT, WIDGET_SANDBOX_URL } from '@/lib/visual/widget'
import { t } from '../../i18n'
import { Frame, MermaidBlock } from './Diagrams'
import './Visual.css'


const color = (index: number) => `var(--viz-${(index % 8) + 1})`
const isLight = () => typeof document !== 'undefined' && document.documentElement.dataset.theme === 'light'

/** The code block a visual falls back to, with a short note on why. */
function VisualFallback({ code, note, detail }: { code: string; note: string; detail?: string }) {
  return <div className="diagram-block diagram-error visual-fallback" data-testid="visual-fallback">
    <div className="diagram-bar"><span>{note}</span></div>
    <pre><code>{code}</code></pre>
    {detail && <small>{detail}</small>}
  </div>
}

/** A render error inside a visual shows its code, never breaks the message. Keyed by code, so new code retries. */
class VisualBoundary extends Component<{ code: string; children: ReactNode }, { error: string | null }> {
  state = { error: null as string | null }
  static getDerivedStateFromError(error: unknown) { return { error: error instanceof Error ? error.message : String(error) } }
  componentDidCatch(error: unknown, info: ErrorInfo) { console.warn('Visual failed to render', error, info.componentStack) }
  render() {
    return this.state.error !== null ? <VisualFallback code={this.props.code} note={t('Could not draw this visual')} detail={this.state.error} /> : this.props.children
  }
}

const PENDING: Record<VisualKind, string> = { chart: 'Drawing chart…', widget: 'Building widget…', mermaid: 'Drawing diagram…' }

function VisualPending({ kind }: { kind: VisualKind }) {
  return <div className="diagram-block visual-pending" role="status" data-testid="visual-pending"><div className="diagram-loading">{t(PENDING[kind])}</div></div>
}

/**
 * One visual fence (```chart, ```html-widget / ```visual, ```mermaid). `open`: the fence is still
 * streaming (no closing fence yet), so a placeholder stands in. Memoized by content, so finished
 * visuals do not redraw while later tokens arrive.
 */
export const VisualFence = memo(function VisualFence({ tag, code, open = false }: { tag: string; code: string; open?: boolean }) {
  const kind = visualKind(tag)
  if (!kind) return <VisualFallback code={code} note={t('Unknown visual')} />
  if (open) return <VisualPending kind={kind} />
  return <VisualBoundary key={code} code={code}>
    {kind === 'chart' ? <ChartBlock code={code} /> : kind === 'widget' ? <WidgetBlock key={code} code={code} /> : <MermaidBlock code={code} />}
  </VisualBoundary>
})

// ---------------------------------------------------------------- charts

type Tip = { x: number; y: number; flip: boolean; title: string; rows: { name: string; value: string; color: string }[] }

const W = 640, H = 300
const PAD = { top: 14, right: 16, bottom: 34, left: 52 }

export const ChartBlock = memo(function ChartBlock({ code }: { code: string }) {
  const parsed = parseChart(code)
  const [table, setTable] = useState(false)
  if (!parsed.ok) return <VisualFallback code={code} note={t('Could not draw this chart')} detail={parsed.error} />
  const { spec } = parsed
  const actions = <button type="button" onClick={() => setTable(value => !value)} aria-pressed={table} aria-label={t('Show data')} title={t('Show data')}><Table size={13} /></button>
  return <Frame label={spec.title ?? t('Chart')} code={code} actions={actions} full={<ChartView spec={spec} large />}>
    {table ? <ChartTable spec={spec} /> : <ChartView spec={spec} />}
  </Frame>
})

function ChartTable({ spec }: { spec: ChartSpec }) {
  if (spec.type === 'scatter') return <div className="visual-table"><table><thead><tr><th>{t('Series')}</th><th>{spec.x ?? 'x'}</th><th>{spec.y ?? 'y'}</th></tr></thead>
    <tbody>{spec.series.flatMap(series => (series.points ?? []).map((point, index) => <tr key={`${series.name}-${index}`}><td>{series.name}</td><td>{formatValue(point[0])}</td><td>{formatValue(point[1])}</td></tr>))}</tbody></table></div>
  return <div className="visual-table"><table>
    <thead><tr><th>{spec.x ?? ''}</th>{spec.series.map(series => <th key={series.name}>{series.name}</th>)}</tr></thead>
    <tbody>{spec.labels.map((label, index) => <tr key={index}><td>{label}</td>{spec.series.map(series => <td key={series.name}>{series.data[index] === undefined ? '' : formatValue(series.data[index])}</td>)}</tr>)}</tbody>
  </table></div>
}

function ChartView({ spec, large = false }: { spec: ChartSpec; large?: boolean }) {
  const [hidden, setHidden] = useState<Set<number>>(() => new Set())
  const [tip, setTip] = useState<Tip | null>(null)
  const [hover, setHover] = useState<number | null>(null)
  const box = useRef<HTMLDivElement>(null)
  const pie = spec.type === 'pie'
  const legendItems = pie ? spec.labels : spec.series.map(series => series.name)
  const toggle = (index: number) => setHidden(current => {
    const next = new Set(current)
    if (next.has(index)) next.delete(index)
    else if (next.size < legendItems.length - 1) next.add(index) // keep at least one visible
    return next
  })
  const place = (event: ReactPointerEvent, content: Omit<Tip, 'x' | 'y' | 'flip'>) => {
    const rect = box.current?.getBoundingClientRect()
    const x = rect && Number.isFinite(event.clientX) ? event.clientX - rect.left : 0
    const y = rect && Number.isFinite(event.clientY) ? event.clientY - rect.top : 0
    setTip({ ...content, x, y, flip: !!rect && x > rect.width * 0.6 })
  }
  const leave = () => { setTip(null); setHover(null) }
  const label = [spec.title, spec.type].filter(Boolean).join(' — ')

  return <div className={`visual-chart${large ? ' large' : ''}`} ref={box} onPointerLeave={leave}>
    {(legendItems.length > 1) && <div className="visual-legend" role="group" aria-label={t('Legend')}>
      {legendItems.map((name, index) => <button key={index} type="button" className="visual-legend-item" aria-pressed={!hidden.has(index)} onClick={() => toggle(index)} title={hidden.has(index) ? t('Show {name}', { name }) : t('Hide {name}', { name })}>
        <i style={{ background: color(index) }} />{name}
      </button>)}
    </div>}
    <svg viewBox={`0 0 ${W} ${H}`} role="img" aria-label={label} preserveAspectRatio="xMidYMid meet">
      <title>{label}</title>
      {pie ? <PieMarks spec={spec} hidden={hidden} hover={hover} setHover={setHover} place={place} />
        : spec.type === 'scatter' ? <ScatterMarks spec={spec} hidden={hidden} hover={hover} setHover={setHover} place={place} />
        : <CartesianMarks spec={spec} hidden={hidden} hover={hover} setHover={setHover} place={place} />}
    </svg>
    {tip && <div className={`visual-tip${tip.flip ? " flip" : ""}`} role="tooltip" style={{ left: tip.x, top: tip.y }}>
      <b>{tip.title}</b>
      {tip.rows.map(row => <span key={row.name}><i style={{ background: row.color }} />{row.name}<em>{row.value}</em></span>)}
    </div>}
  </div>
}

type Marks = { spec: ChartSpec; hidden: Set<number>; hover: number | null; setHover: (index: number | null) => void; place: (event: ReactPointerEvent, tip: Omit<Tip, 'x' | 'y' | 'flip'>) => void }

function Axes({ ticks, scaleY, x, y, labels, every }: { ticks: number[]; scaleY: (value: number) => number; x?: string; y?: string; labels: { at: number; text: string }[]; every: number }) {
  return <g className="visual-axes">
    {ticks.map(tick => <g key={tick}>
      <line x1={PAD.left} x2={W - PAD.right} y1={scaleY(tick)} y2={scaleY(tick)} className={tick === 0 ? 'zero' : 'grid'} />
      <text x={PAD.left - 8} y={scaleY(tick)} textAnchor="end" dominantBaseline="middle">{formatTick(tick)}</text>
    </g>)}
    {labels.map((item, index) => index % every === 0 && <text key={index} x={item.at} y={H - PAD.bottom + 16} textAnchor="middle">{item.text.length > 14 ? `${item.text.slice(0, 13)}…` : item.text}</text>)}
    {x && <text x={(PAD.left + W - PAD.right) / 2} y={H - 4} textAnchor="middle" className="axis-title">{x}</text>}
    {y && <text transform={`translate(12 ${(PAD.top + H - PAD.bottom) / 2}) rotate(-90)`} textAnchor="middle" className="axis-title">{y}</text>}
  </g>
}

function CartesianMarks({ spec, hidden, hover, setHover, place }: Marks) {
  const visible = spec.series.map((series, index) => ({ series, index })).filter(item => !hidden.has(item.index))
  const count = spec.labels.length
  const stacked = spec.stacked
  const geometry = (() => {
    const totals = spec.labels.map((_, i) => visible.reduce((sum, item) => sum + (item.series.data[i] ?? 0), 0))
    const values = stacked ? totals : visible.flatMap(item => item.series.data)
    let min = Math.min(...values), max = Math.max(...values)
    if (spec.type !== 'line' || min > 0 === max > 0) { min = Math.min(0, min); max = Math.max(0, max) }
    const ticks = niceTicks(min, max)
    return { ticks, lo: ticks[0], hi: ticks[ticks.length - 1] }
  })()
  const plotW = W - PAD.left - PAD.right, plotH = H - PAD.top - PAD.bottom
  const scaleY = (value: number) => PAD.top + plotH - ((value - geometry.lo) / (geometry.hi - geometry.lo || 1)) * plotH
  const band = plotW / Math.max(1, count)
  const center = (i: number) => PAD.left + band * i + band / 2
  const every = Math.ceil(count / (spec.type === 'bar' ? 12 : 10))
  const zero = scaleY(Math.max(geometry.lo, Math.min(0, geometry.hi)))

  const tipFor = (i: number) => ({ title: spec.labels[i], rows: visible.map(item => ({ name: item.series.name, value: item.series.data[i] === undefined ? '—' : formatValue(item.series.data[i]), color: color(item.index) })) })

  let marks: ReactNode
  if (spec.type === 'bar') {
    const group = band * 0.72
    const width = stacked ? group : group / Math.max(1, visible.length)
    marks = spec.labels.map((_, i) => {
      let base = 0
      return <g key={i} opacity={hover === null || hover === i ? 1 : 0.55}>{visible.map((item, slot) => {
        const value = item.series.data[i] ?? 0
        const from = stacked ? base : 0
        const to = from + value
        if (stacked) base = to
        const x = center(i) - group / 2 + (stacked ? 0 : slot * width)
        const top = scaleY(Math.max(from, to)), bottom = scaleY(Math.min(from, to))
        return <rect key={item.index} x={x + 1} y={top} width={Math.max(1, width - 2)} height={Math.max(0, bottom - top - (stacked ? 1 : 0))} rx={Math.min(3, width / 4)} fill={color(item.index)} className="visual-mark" />
      })}</g>
    })
  } else {
    const running = spec.labels.map(() => 0)
    const layers = visible.map(item => {
      const points = item.series.data.slice(0, count).map((value, i) => {
        const bottom = stacked ? running[i] : 0
        const top = stacked ? bottom + value : value
        if (stacked) running[i] = top
        return { x: center(i), y: scaleY(top), base: scaleY(bottom) }
      })
      return { item, points }
    })
    marks = layers.map(({ item, points }) => {
      const line = points.map((point, i) => `${i ? 'L' : 'M'}${point.x.toFixed(1)},${point.y.toFixed(1)}`).join(' ')
      const area = spec.type === 'area' && points.length
        ? `${line} ${[...points].reverse().map(point => `L${point.x.toFixed(1)},${(stacked ? point.base : zero).toFixed(1)}`).join(' ')} Z`
        : ''
      return <g key={item.index}>
        {area && <path d={area} fill={color(item.index)} className="visual-area" />}
        <path d={line} fill="none" stroke={color(item.index)} strokeWidth={2} strokeLinejoin="round" strokeLinecap="round" />
        {(points.length <= 40 || hover !== null) && points.map((point, i) => (points.length <= 40 || hover === i) && <circle key={i} cx={point.x} cy={point.y} r={hover === i ? 4.5 : 3} fill={color(item.index)} className="visual-dot" />)}
      </g>
    })
  }

  return <g>
    <Axes ticks={geometry.ticks} scaleY={scaleY} x={spec.x} y={spec.y} labels={spec.labels.map((text, i) => ({ at: center(i), text }))} every={every} />
    {spec.type !== 'bar' && hover !== null && <line className="visual-crosshair" x1={center(hover)} x2={center(hover)} y1={PAD.top} y2={H - PAD.bottom} />}
    {marks}
    {spec.labels.map((_, i) => <rect key={i} className="visual-hit" x={PAD.left + band * i} y={PAD.top} width={band} height={plotH} fill="transparent"
      onPointerEnter={event => { setHover(i); place(event, tipFor(i)) }} onPointerMove={event => place(event, tipFor(i))} />)}
  </g>
}

function PieMarks({ spec, hidden, hover, setHover, place }: Marks) {
  const data = spec.series[0].data
  const slices = spec.labels.map((name, index) => ({ name, index, value: data[index] ?? 0 })).filter(slice => !hidden.has(slice.index) && slice.value > 0)
  const total = slices.reduce((sum, slice) => sum + slice.value, 0) || 1
  const cx = W / 2, cy = H / 2, r = H / 2 - 16, inner = r * 0.56
  const angles = slices.reduce<number[]>((list, slice) => [...list, list[list.length - 1] + (slice.value / total) * Math.PI * 2], [-Math.PI / 2])
  const point = (radius: number, a: number) => `${(cx + radius * Math.cos(a)).toFixed(2)},${(cy + radius * Math.sin(a)).toFixed(2)}`
  return <g>
    {slices.map((slice, i) => {
      const start = angles[i], end = angles[i + 1], sweep = end - start
      const large = sweep > Math.PI ? 1 : 0
      // A lone full slice is drawn as two halves so the arc is not degenerate.
      const d = sweep >= Math.PI * 2 - 1e-6
        ? `M${point(r, start)} A${r},${r} 0 1 1 ${point(r, start + Math.PI)} A${r},${r} 0 1 1 ${point(r, start)} M${point(inner, start)} A${inner},${inner} 0 1 0 ${point(inner, start + Math.PI)} A${inner},${inner} 0 1 0 ${point(inner, start)} Z`
        : `M${point(r, start)} A${r},${r} 0 ${large} 1 ${point(r, end)} L${point(inner, end)} A${inner},${inner} 0 ${large} 0 ${point(inner, start)} Z`
      const tip = { title: slice.name, rows: [{ name: formatValue(slice.value), value: `${((slice.value / total) * 100).toFixed(1)}%`, color: color(slice.index) }] }
      return <path key={slice.index} d={d} fill={color(slice.index)} fillRule="evenodd" className="visual-slice" opacity={hover === null || hover === slice.index ? 1 : 0.6}
        onPointerEnter={event => { setHover(slice.index); place(event, tip) }} onPointerMove={event => place(event, tip)} />
    })}
    <text x={cx} y={cy - 6} textAnchor="middle" className="visual-pie-total">{formatTick(slices.reduce((sum, slice) => sum + slice.value, 0))}</text>
    <text x={cx} y={cy + 14} textAnchor="middle" className="visual-pie-caption">{spec.y ?? t('Total')}</text>
  </g>
}

function ScatterMarks({ spec, hidden, hover, setHover, place }: Marks) {
  const visible = spec.series.map((series, index) => ({ series, index })).filter(item => !hidden.has(item.index))
  const all = visible.flatMap(item => item.series.points ?? [])
  const xs = niceTicks(Math.min(...all.map(p => p[0])), Math.max(...all.map(p => p[0])))
  const ys = niceTicks(Math.min(...all.map(p => p[1])), Math.max(...all.map(p => p[1])))
  const plotW = W - PAD.left - PAD.right, plotH = H - PAD.top - PAD.bottom
  const sx = (value: number) => PAD.left + ((value - xs[0]) / (xs[xs.length - 1] - xs[0] || 1)) * plotW
  const sy = (value: number) => PAD.top + plotH - ((value - ys[0]) / (ys[ys.length - 1] - ys[0] || 1)) * plotH
  let id = 0
  return <g>
    <Axes ticks={ys} scaleY={sy} x={spec.x} y={spec.y} labels={xs.map(value => ({ at: sx(value), text: formatTick(value) }))} every={1} />
    {visible.map(item => (item.series.points ?? []).map(([x, y]) => {
      const key = id++
      const tip = { title: item.series.name, rows: [{ name: spec.x ?? 'x', value: formatValue(x), color: color(item.index) }, { name: spec.y ?? 'y', value: formatValue(y), color: color(item.index) }] }
      return <g key={key}>
        <circle cx={sx(x)} cy={sy(y)} r={hover === key ? 6 : 4.5} fill={color(item.index)} className="visual-point" />
        <circle cx={sx(x)} cy={sy(y)} r={10} fill="transparent" className="visual-hit" onPointerEnter={event => { setHover(key); place(event, tip) }} onPointerMove={event => place(event, tip)} />
      </g>
    }))}
  </g>
}

// ---------------------------------------------------------------- widgets

const heights = new Map<string, number>()
const byteLength = (text: string) => new TextEncoder().encode(text).length

/**
 * A ```html-widget fence, run in app/public/viz-sandbox.html inside <iframe sandbox="allow-scripts">
 * (no allow-same-origin: an opaque origin with no access to the app, its storage or its IPC). The
 * page's own CSP allows no network. The widget arrives by postMessage and reports its height back;
 * messages are accepted only from this iframe's window with the opaque origin "null".
 */
export const WidgetBlock = memo(function WidgetBlock({ code }: { code: string }) {
  const frame = useRef<HTMLIFrameElement>(null)
  const loads = useRef(0)
  const sent = useRef(false)
  const [height, setHeight] = useState(() => heights.get(code) ?? 160)
  const [error, setError] = useState('')
  const [escaped, setEscaped] = useState(false)
  const [source, setSource] = useState(false)
  const tooBig = byteLength(code) > WIDGET_MAX_BYTES

  // Sent once, on the sandbox's "ready" or, failing that, its first load (the sandbox ignores repeats).
  const deliver = useCallback(() => {
    const target = frame.current?.contentWindow
    if (!target || sent.current) return
    sent.current = true
    target.postMessage({ source: 'neru-viz-host', type: 'render', html: code, theme: isLight() ? 'light' : 'dark' }, '*')
  }, [code])

  useEffect(() => {
    if (tooBig) return
    const onMessage = (event: MessageEvent) => {
      if (!frame.current || event.source !== frame.current.contentWindow || event.origin !== 'null') return
      const data = event.data as { source?: string; type?: string; height?: unknown; message?: unknown } | null
      if (!data || data.source !== 'neru-viz') return
      if (data.type === 'ready') deliver()
      else if (data.type === 'height' && typeof data.height === 'number' && Number.isFinite(data.height)) {
        const next = Math.round(Math.min(WIDGET_MAX_HEIGHT, Math.max(WIDGET_MIN_HEIGHT, data.height)))
        heights.set(code, next)
        setHeight(next)
      } else if (data.type === 'error' && typeof data.message === 'string') setError(data.message.slice(0, 300))
    }
    window.addEventListener('message', onMessage)
    return () => window.removeEventListener('message', onMessage)
  }, [code, tooBig, deliver])

  if (tooBig) return <VisualFallback code={code.slice(0, 4000) + '\n…'} note={t('This widget is too large to run (over 200 KB)')} />
  if (escaped) return <VisualFallback code={code} note={t('This widget tried to leave its sandbox and was stopped')} />

  // A second load means the widget navigated its frame away: stop it.
  const onLoad = () => {
    loads.current += 1
    if (loads.current > 1) setEscaped(true)
    else window.setTimeout(deliver, 300)
  }
  const actions = <button type="button" onClick={() => setSource(value => !value)} aria-pressed={source} aria-label={t('Show source')} title={t('Show source')}><Code size={13} /></button>
  return <Frame label={t('Interactive widget')} code={code} actions={actions}>
    <iframe ref={frame} className="visual-widget" title={t('Interactive widget')} src={WIDGET_SANDBOX_URL} sandbox="allow-scripts" referrerPolicy="no-referrer"
      style={{ height }} onLoad={onLoad} />
    {error && <small className="visual-widget-error">{t('The widget reported an error: {message}', { message: error })}</small>}
    {source && <pre className="visual-source"><code>{code}</code></pre>}
  </Frame>
})

