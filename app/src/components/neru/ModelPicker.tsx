import { useEffect, useMemo, useRef, useState, type ReactNode, type Ref } from 'react'
import { Brain, Check, CircleAlert, CodeXml, Eye, ListChecks, LoaderCircle, Search, Square, Wrench } from 'lucide-react'
import { api } from '../../api'
import type { ApiFormat } from '../../providerCatalog'
import type { ModelInfo, ProbeResult } from '../../types'
import { cn } from '@/lib/utils'

export type ModelFilter = 'all' | 'vision' | 'reasoning' | 'tools' | 'code' | 'free'

const FILTERS: { id: ModelFilter; label: string; test: (info: ModelInfo) => boolean; hint: string }[] = [
  { id: 'all', label: 'All', test: () => true, hint: 'Every chat model this key can list' },
  { id: 'vision', label: 'Vision', test: info => info.vision, hint: 'Reads images as well as text' },
  { id: 'reasoning', label: 'Reasoning', test: info => info.reasoning, hint: 'Thinks before it answers' },
  { id: 'tools', label: 'Tools', test: info => info.tools, hint: 'Can call tools, which Neru needs to edit files and run commands' },
  { id: 'code', label: 'Code', test: info => info.kind === 'code', hint: 'Tuned for writing code' },
  { id: 'free', label: 'Free', test: info => info.free, hint: 'Free to use' },
]

/** Providers whose own list already says which models are live (or where a test request would spend scarce quota). */
export const LISTING_IS_AUTHORITATIVE = new Set(['openrouter', 'ollama', 'local'])

export function contextLabel(tokens?: number) {
  if (!tokens) return ''
  if (tokens >= 1_000_000) return `${String(Math.round(tokens / 100_000) / 10).replace(/\.0$/, '')}M`
  return `${tokens % 1024 === 0 ? tokens / 1024 : Math.round(tokens / 1000)}K`
}

export const shortModelName = (id: string) => id.split('/').pop() || id

/** Replaces the classification of one model with what a check found. */
export function applyProbe(models: ModelInfo[], result: ProbeResult): ModelInfo[] {
  if (result.status !== 'ok' && result.status !== 'unavailable') return models
  return models.map(info => info.id === result.model ? { ...info, verified: result.status as 'ok' | 'unavailable', verifiedReason: result.reason } : info)
}

export function filterModels(models: ModelInfo[], options: { query: string; filter: ModelFilter; showUnavailable: boolean; keep?: string }) {
  const terms = options.query.toLowerCase().split(/\s+/).filter(Boolean)
  const test = FILTERS.find(item => item.id === options.filter)?.test ?? (() => true)
  return models.filter(info => (options.showUnavailable || info.verified !== 'unavailable' || info.id === options.keep)
    && test(info)
    && terms.every(term => `${info.id} ${info.name ?? ''}`.toLowerCase().includes(term)))
}

/** Models that read images sit above text-only ones; the order inside each group is the list's order. */
export function groupModels(models: ModelInfo[]): { key: string; title: string; items: ModelInfo[] }[] {
  const vision = models.filter(info => info.vision)
  const text = models.filter(info => !info.vision)
  return [
    { key: 'vision', title: 'Text + vision', items: vision },
    { key: 'text', title: 'Text only', items: text },
  ].filter(group => group.items.length > 0)
}

function guessNote(info: ModelInfo) {
  return info.source === 'name' ? ' (guessed from the model name)' : ''
}

export function ModelBadges({ info }: { info: ModelInfo }) {
  const context = contextLabel(info.contextWindow)
  return <span className="model-badges">
    {info.vision && <span className="model-badge vision" title={`Reads images${guessNote(info)}`}><Eye size={12} strokeWidth={2} /></span>}
    {info.reasoning && <span className="model-badge reasoning" title={`Reasoning model${guessNote(info)}`}><Brain size={12} strokeWidth={2} /></span>}
    {info.tools && <span className="model-badge tools" title={`Can call tools, which Neru needs to edit files and run commands${guessNote(info)}`}><Wrench size={12} strokeWidth={2} /></span>}
    {info.kind === 'code' && <span className="model-badge code" title="Tuned for code"><CodeXml size={12} strokeWidth={2} /></span>}
    {info.free && <span className="model-badge free" title="Free to use">free</span>}
    {context && <span className="model-badge context" title={`${info.contextWindow?.toLocaleString()} tokens of context`}>{context}</span>}
    {info.verified === 'ok' && <span className="model-badge ok" title={info.verifiedReason || 'A test request worked'}><Check size={12} strokeWidth={2.4} /></span>}
    {info.verified === 'unavailable' && <span className="model-badge bad" title={info.verifiedReason || 'The provider does not serve this model to this key'}><CircleAlert size={12} strokeWidth={2} /></span>}
  </span>
}

export function ModelFilters({ models, filter, onFilter, hidden, showUnavailable, onShowUnavailable }: {
  models: ModelInfo[]; filter: ModelFilter; onFilter: (filter: ModelFilter) => void; hidden: number; showUnavailable: boolean; onShowUnavailable: (show: boolean) => void
}) {
  return <div className="model-filters" role="group" aria-label="Filter models">
    {FILTERS.map(item => {
      const count = models.filter(item.test).length
      if (item.id !== 'all' && count === 0) return null
      return <button key={item.id} type="button" className={cn('model-chip', filter === item.id && 'on')} aria-pressed={filter === item.id} title={item.hint} onClick={() => onFilter(item.id)}>{item.label}<span>{count}</span></button>
    })}
    {(hidden > 0 || showUnavailable) && <button type="button" className={cn('model-chip warn', showUnavailable && 'on')} aria-pressed={showUnavailable} title="Models this provider refused when checked" onClick={() => onShowUnavailable(!showUnavailable)}>{showUnavailable ? 'Hide unavailable' : `Show unavailable`}<span>{hidden}</span></button>}
  </div>
}

export function ModelSearch({ value, onChange, count, inputRef, onEnter }: { value: string; onChange: (value: string) => void; count: number; inputRef?: Ref<HTMLInputElement>; onEnter?: () => void }) {
  return <div className="model-search">
    <Search size={13} aria-hidden />
    <input ref={inputRef} value={value} onChange={event => onChange(event.target.value)} placeholder={`Search ${count} models`} aria-label="Search models" spellCheck={false}
      onKeyDown={event => { if (event.key === 'Enter' && onEnter) { event.preventDefault(); onEnter() } }} />
  </div>
}

export function ModelRowText({ info, unique }: { info: ModelInfo; unique: boolean }) {
  return <span className={cn('model-row-text', info.verified === 'unavailable' && 'is-unavailable')} title={info.verified === 'unavailable' ? info.verifiedReason || info.id : info.id}>
    <span className="model-row-name">{unique ? shortModelName(info.id) : info.id}</span>
    {info.name && info.name !== shortModelName(info.id) && <span className="model-row-sub">{info.name}</span>}
  </span>
}

/** Shared by the composer's menu: the display name is the short id unless two models would share it. */
export function uniqueShortNames(models: ModelInfo[]) {
  const counts = new Map<string, number>()
  for (const info of models) counts.set(shortModelName(info.id), (counts.get(shortModelName(info.id)) ?? 0) + 1)
  return (info: ModelInfo) => counts.get(shortModelName(info.id)) === 1
}

type Notice = { tone: 'error' | 'note'; text: string }

/** The Model setting: a searchable, filterable list with capability badges, a check when a model is picked,
 *  and a bulk "Check availability" that marks the models the provider refuses. */
export function SettingsModelPicker({ providerId, baseUrl, apiKey, models, onModels, value, onSelect, loading, formatFor, emptyText, footer }: {
  providerId: string; baseUrl: string; apiKey: string; models: ModelInfo[]; onModels: (update: (models: ModelInfo[]) => ModelInfo[]) => void
  value: string; onSelect: (model: string) => void; loading: boolean; formatFor: (model: string) => ApiFormat; emptyText: string; footer?: ReactNode
}) {
  const [query, setQuery] = useState('')
  const [filter, setFilter] = useState<ModelFilter>('all')
  const [showUnavailable, setShowUnavailable] = useState(false)
  const [picking, setPicking] = useState('')
  const [notice, setNotice] = useState<Notice | null>(null)
  const [run, setRun] = useState<{ id: string; done: number; total: number } | null>(null)
  const [summary, setSummary] = useState('')
  const runId = useRef('')
  const mounted = useRef(true)
  const authoritative = LISTING_IS_AUTHORITATIVE.has(providerId)

  useEffect(() => { mounted.current = true; return () => { mounted.current = false } }, [])
  // The list belongs to one provider and key; switching them drops the old messages.
  useEffect(() => { setNotice(null); setSummary(''); setQuery(''); setFilter('all') }, [providerId, baseUrl])
  useEffect(() => {
    let stop: (() => void) | undefined
    let live = true
    void api.onModelCheck(progress => {
      if (progress.runId !== runId.current || !mounted.current) return
      if (progress.result) onModels(current => applyProbe(current, progress.result as ProbeResult))
      setRun(current => current ? { ...current, done: progress.done, total: progress.total } : current)
    }).then(unlisten => { if (live) stop = unlisten; else unlisten() }).catch(() => undefined)
    return () => { live = false; stop?.() }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  const hidden = models.filter(info => info.verified === 'unavailable').length
  const shown = useMemo(() => filterModels(models, { query, filter, showUnavailable, keep: value }), [models, query, filter, showUnavailable, value])
  const groups = groupModels(shown)
  const isUnique = useMemo(() => uniqueShortNames(models), [models])
  const checked = models.filter(info => info.verified === 'ok').length

  const pick = async (info: ModelInfo) => {
    if (info.id === value || picking) return
    setNotice(null)
    if (authoritative) { onSelect(info.id); return }
    setPicking(info.id)
    try {
      const result = await api.probeModel(providerId, formatFor(info.id), baseUrl, apiKey, info.id)
      if (!mounted.current) return
      onModels(current => applyProbe(current, result))
      if (result.status === 'unavailable') { setNotice({ tone: 'error', text: `${info.id} is not available on this provider: ${result.reason}. Kept ${value || 'your previous model'}.` }); return }
      if (result.status === 'badKey') { setNotice({ tone: 'error', text: `${result.reason}. Check the API key, then pick a model again.` }); return }
      if (result.status === 'unknown') setNotice({ tone: 'note', text: `Could not confirm ${shortModelName(info.id)} right now (${result.reason}). It may still work.` })
      onSelect(info.id)
    } catch (cause) {
      if (mounted.current) { setNotice({ tone: 'note', text: `Could not check ${shortModelName(info.id)} (${cause instanceof Error ? cause.message : String(cause)}). Using it anyway.` }); onSelect(info.id) }
    } finally { if (mounted.current) setPicking('') }
  }

  const check = async () => {
    const id = `check-${Date.now().toString(36)}`
    runId.current = id
    setNotice(null); setSummary('')
    const targets = models.filter(info => info.verified !== 'unavailable').map(info => ({ model: info.id, apiFormat: formatFor(info.id) }))
    setRun({ id, done: 0, total: targets.length })
    try {
      const outcome = await api.checkModels(id, providerId, baseUrl, apiKey, targets)
      if (!mounted.current) return
      const parts = [`${outcome.ok} work`, `${outcome.unavailable} unavailable`]
      if (outcome.unknown) parts.push(`${outcome.unknown} could not be confirmed`)
      setSummary(`${outcome.cancelled ? 'Stopped. ' : ''}${parts.join(', ')}.${outcome.note ? ` ${outcome.note}` : ''}`)
      if (outcome.unavailable > 0) setShowUnavailable(false)
    } catch (cause) {
      if (mounted.current) setNotice({ tone: 'error', text: cause instanceof Error ? cause.message : String(cause) })
    } finally { if (mounted.current) setRun(null); runId.current = '' }
  }
  const cancel = () => { if (runId.current) void api.cancelCheckModels(runId.current).catch(() => undefined) }

  if (models.length === 0) return <div className="model-picker empty"><p>{loading ? 'Looking up models…' : emptyText}</p></div>

  return <div className="model-picker">
    <ModelSearch value={query} onChange={setQuery} count={models.length} />
    <ModelFilters models={models} filter={filter} onFilter={setFilter} hidden={hidden} showUnavailable={showUnavailable} onShowUnavailable={setShowUnavailable} />
    <div className="model-picker-list" role="listbox" aria-label="Models">
      {groups.map(group => <div key={group.key} className="model-group">
        <p className="model-group-title">{group.title}<span>{group.items.length}</span></p>
        {group.items.map(info => <button key={info.id} type="button" role="option" aria-selected={info.id === value} disabled={Boolean(picking)} className={cn('model-row', info.id === value && 'selected', info.verified === 'unavailable' && 'unavailable')} onClick={() => void pick(info)}>
          <ModelRowText info={info} unique={isUnique(info)} />
          {picking === info.id ? <span className="model-row-status"><LoaderCircle size={13} className="animate-spin" />Checking…</span> : <ModelBadges info={info} />}
        </button>)}
      </div>)}
      {shown.length === 0 && <p className="model-picker-none">No models match{query ? ` “${query}”` : ' this filter'}.</p>}
    </div>
    {notice && <p className={cn('model-notice-line', notice.tone)} role={notice.tone === 'error' ? 'alert' : 'status'}>{notice.text}</p>}
    <div className="model-picker-foot">
      {authoritative
        ? <span className="model-foot-text">{providerId === 'openrouter' ? 'Only the models your OpenRouter account can use, with tool calling. Free-tier keys see free models only.' : 'These models are installed or served right now.'}</span>
        : run
          ? <>
            <span className="model-foot-text" aria-live="polite">{run.done}/{run.total} checked</span>
            <span className="model-progress" aria-hidden><i style={{ width: `${run.total ? Math.round(run.done / run.total * 100) : 0}%` }} /></span>
            <button type="button" className="button subtle small" onClick={cancel}><Square size={11} /> Stop</button>
          </>
          : <>
            <button type="button" className="button subtle small" onClick={() => void check()} disabled={loading || Boolean(picking)} title="Sends one tiny request per model to see which ones this provider actually serves"><ListChecks size={13} /> Check availability</button>
            <span className="model-foot-text">{summary || (checked || hidden ? `${checked} confirmed${hidden ? `, ${hidden} unavailable` : ''}. ` : 'Some listed models may not be served to your key. ')}</span>
          </>}
      {footer}
    </div>
  </div>
}
