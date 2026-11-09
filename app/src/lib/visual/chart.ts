/** The ```chart fence: a small JSON spec the conversation draws natively as SVG. */

export type ChartType = 'bar' | 'line' | 'area' | 'pie' | 'scatter'
export const CHART_TYPES: readonly ChartType[] = ['bar', 'line', 'area', 'pie', 'scatter']

export type ChartSeries = { name: string; data: number[]; points?: [number, number][] }
export type ChartSpec = {
  type: ChartType
  title?: string
  labels: string[]
  series: ChartSeries[]
  x?: string
  y?: string
  stacked: boolean
}

export const CHART_LIMITS = { series: 8, points: 500, bytes: 64 * 1024 }

export type ChartParse = { ok: true; spec: ChartSpec } | { ok: false; error: string }

const text = (value: unknown, max = 120) => typeof value === 'string' && value.trim() ? value.trim().slice(0, max) : undefined
const num = (value: unknown) => typeof value === 'number' && Number.isFinite(value) ? value : typeof value === 'string' && value.trim() !== '' && Number.isFinite(Number(value)) ? Number(value) : null

const cache = new Map<string, ChartParse>()

/** Parses and checks a chart spec. Results are cached by source, so a re-render costs nothing. */
export function parseChart(source: string): ChartParse {
  const hit = cache.get(source)
  if (hit) return hit
  const result = parseUncached(source)
  if (cache.size > 200) cache.clear()
  cache.set(source, result)
  return result
}

function parseUncached(source: string): ChartParse {
  if (source.length > CHART_LIMITS.bytes) return { ok: false, error: 'The chart spec is too large.' }
  let raw: unknown
  try { raw = JSON.parse(source) } catch (cause) { return { ok: false, error: `Not valid JSON: ${cause instanceof Error ? cause.message : String(cause)}` } }
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return { ok: false, error: 'The chart spec must be a JSON object.' }
  const spec = raw as Record<string, unknown>
  const type = String(spec.type ?? 'bar').toLowerCase() as ChartType
  if (!CHART_TYPES.includes(type)) return { ok: false, error: `Unknown chart type "${String(spec.type)}". Use bar, line, area, pie or scatter.` }

  // `series` may be a bare list of numbers (one unnamed series), or `data` may stand in for it.
  let rawSeries: unknown = spec.series ?? spec.data
  if (Array.isArray(rawSeries) && rawSeries.every(item => typeof item === 'number')) rawSeries = [{ name: text(spec.title) ?? '', data: rawSeries }]
  if (!Array.isArray(rawSeries) || rawSeries.length === 0) return { ok: false, error: 'The chart needs a non-empty "series" list.' }
  if (rawSeries.length > CHART_LIMITS.series) return { ok: false, error: `At most ${CHART_LIMITS.series} series are drawn.` }

  const labels = Array.isArray(spec.labels) ? spec.labels.slice(0, CHART_LIMITS.points).map(label => String(label ?? '').slice(0, 60)) : []
  const series: ChartSeries[] = []
  for (const [index, entry] of rawSeries.entries()) {
    if (!entry || typeof entry !== 'object') return { ok: false, error: `Series ${index + 1} must be an object with "name" and "data".` }
    const item = entry as Record<string, unknown>
    if (!Array.isArray(item.data)) return { ok: false, error: `Series ${index + 1} has no "data" list.` }
    if (item.data.length > CHART_LIMITS.points) return { ok: false, error: `At most ${CHART_LIMITS.points} points per series.` }
    const name = text(item.name, 60) ?? `Series ${index + 1}`
    if (type === 'scatter') {
      const points: [number, number][] = []
      for (const point of item.data) {
        const pair = Array.isArray(point) ? [num(point[0]), num(point[1])] : point && typeof point === 'object' ? [num((point as Record<string, unknown>).x), num((point as Record<string, unknown>).y)] : [null, null]
        if (pair[0] === null || pair[1] === null) return { ok: false, error: `Scatter points must be [x, y] pairs (series ${index + 1}).` }
        points.push([pair[0], pair[1]])
      }
      series.push({ name, data: points.map(point => point[1]), points })
    } else {
      const data = item.data.map(num)
      if (data.some(value => value === null)) return { ok: false, error: `Series ${index + 1} has a value that is not a number.` }
      series.push({ name, data: data as number[] })
    }
  }
  const longest = Math.max(...series.map(item => item.data.length))
  if (longest === 0) return { ok: false, error: 'The chart has no data.' }
  if (type === 'pie' && series[0].data.some(value => value < 0)) return { ok: false, error: 'Pie slices cannot be negative.' }
  // Missing labels become 1, 2, 3...
  const filled = type === 'scatter' ? labels : Array.from({ length: longest }, (_, index) => labels[index] ?? String(index + 1))
  return {
    ok: true,
    spec: {
      type,
      title: text(spec.title),
      labels: filled,
      series: type === 'pie' ? [series[0]] : series,
      x: text(spec.x ?? spec.xLabel ?? spec.xAxis, 60),
      y: text(spec.y ?? spec.yLabel ?? spec.yAxis, 60),
      stacked: spec.stacked === true && (type === 'bar' || type === 'area'),
    },
  }
}

/** Round axis ticks covering [min, max], about `count` of them. */
export function niceTicks(min: number, max: number, count = 5): number[] {
  if (min === max) { const pad = Math.abs(min) || 1; min -= pad; max += pad }
  const raw = (max - min) / count
  const power = 10 ** Math.floor(Math.log10(raw))
  const step = [1, 2, 2.5, 5, 10].map(m => m * power).find(candidate => candidate >= raw) ?? 10 * power
  const start = Math.floor(min / step) * step
  const ticks: number[] = []
  for (let value = start; value <= max + step * 1e-9; value += step) ticks.push(Number(value.toFixed(10)))
  if (ticks[ticks.length - 1] < max) ticks.push(Number((ticks[ticks.length - 1] + step).toFixed(10)))
  return ticks
}

const compact = new Intl.NumberFormat(undefined, { notation: 'compact', maximumFractionDigits: 1 })
const full = new Intl.NumberFormat(undefined, { maximumFractionDigits: 2 })
export const formatTick = (value: number) => Math.abs(value) >= 10_000 ? compact.format(value) : full.format(value)
export const formatValue = (value: number) => full.format(value)
