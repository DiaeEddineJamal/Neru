import { useEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent as ReactKeyboardEvent } from 'react'
import { createPortal } from 'react-dom'
import { Brain, Check, ChevronDown, CircleAlert, Eye, Info, Search, Wrench } from '@/components/icons'
import { cn } from '@/lib/utils'
import type { Effort, ModelInfo } from '../../types'
import { t } from '../../i18n'
import { describe, familyOf, hostOf, metricsFor, modelLogo, type Logo, type MetricNotes, type Metrics } from '@/lib/modelFamily'
import { ModelFilters, contextLabel, filterModels, groupModels, shortModelName, uniqueShortNames, type ModelFilter } from './ModelPicker'

const EFFORTS: { value: Effort; label: string; hint: string }[] = [
  { value: 'auto', label: 'Auto', hint: 'The model decides how long to think' },
  { value: 'low', label: 'Low', hint: 'Fastest answers, least reasoning' },
  { value: 'medium', label: 'Medium', hint: 'Balanced reasoning' },
  { value: 'high', label: 'High', hint: 'Thinks longest; best for hard problems' },
]

/** Reasoning raises the intelligence bar and lowers the speed bar a little, so the preview shows the trade. */
const EFFORT_DELTA: Record<Effort, { intelligence: number; speed: number }> = {
  auto: { intelligence: 0, speed: 0 }, low: { intelligence: -1, speed: 1 }, medium: { intelligence: 0, speed: 0 }, high: { intelligence: 1, speed: -2 },
}

const BY_MODEL_KEY = 'neru.effort.byModel'
const readEfforts = (): Record<string, Effort> => { try { return JSON.parse(localStorage.getItem(BY_MODEL_KEY) || '{}') } catch { return {} } }
const writeEfforts = (value: Record<string, Effort>) => { try { localStorage.setItem(BY_MODEL_KEY, JSON.stringify(value)) } catch { /* private mode */ } }

const clamp = (value: number) => Math.max(0, Math.min(10, value))

export function ModelLogo({ logo, size = 14, className }: { logo: Logo | null; size?: number; className?: string }) {
  if (!logo) return <span className={cn('model-logo blank', className)} style={{ width: size, height: size }} aria-hidden />
  if (logo.mono) return <span className={cn('model-logo mono', className)} style={{ width: size, height: size, '--logo': `url("${logo.url}")` } as CSSProperties} aria-hidden />
  return <img className={cn('model-logo', className)} src={logo.url} width={size} height={size} alt="" aria-hidden draggable={false} />
}

const METRICS: { key: keyof Metrics; label: string; tone: string }[] = [
  { key: 'intelligence', label: 'Intelligence', tone: 'good' },
  { key: 'speed', label: 'Speed', tone: 'warm' },
  { key: 'context', label: 'Context', tone: 'good' },
  { key: 'cost', label: 'Cost', tone: 'good' },
]

function MetricBar({ label, value, note, tone }: { label: string; value: number; note: string; tone: string }) {
  return <div className="ms-metric">
    <div className="ms-metric-head"><span>{t(label)}</span><span className="ms-metric-info" title={t(note)} aria-label={t(note)}><Info size={10} /></span></div>
    <div className={cn('ms-bar', tone)} role="meter" aria-label={t(label)} aria-valuemin={0} aria-valuemax={10} aria-valuenow={value} title={t(note)}>
      {Array.from({ length: 10 }, (_, index) => <i key={index} className={index < value ? 'on' : undefined} />)}
    </div>
  </div>
}

function Segmented({ label, value, onChange }: { label: string; value: Effort; onChange: (value: Effort) => void }) {
  return <div className="ms-config-row">
    <span className="ms-config-label">{t(label)}</span>
    <div className="ms-segmented" role="radiogroup" aria-label={t(label)}>
      {EFFORTS.map(item => <button key={item.value} type="button" role="radio" aria-checked={value === item.value} title={t(item.hint)} className={cn(value === item.value && 'on')}
        onMouseDown={event => event.preventDefault()} onClick={() => onChange(item.value)}>{t(item.label)}</button>)}
    </div>
  </div>
}

function Preview({ info, providerId, metrics, notes, effort, canReason, onEffort, current }: {
  info: ModelInfo; providerId: string; metrics: Metrics; notes: MetricNotes; effort: Effort; canReason: boolean; onEffort: (value: Effort) => void; current: boolean
}) {
  const family = familyOf(info.id)
  const host = hostOf(providerId)
  const delta = canReason ? EFFORT_DELTA[effort] : EFFORT_DELTA.auto
  const shown: Metrics = { ...metrics, intelligence: clamp(metrics.intelligence + delta.intelligence), speed: clamp(metrics.speed + delta.speed) }
  return <div className="ms-preview" role="dialog" aria-label={t('About {model}', { model: shortModelName(info.id) })}>
    <div className="ms-preview-head">
      <ModelLogo logo={modelLogo(info.id)} size={22} className="ms-preview-logo" />
      <div className="min-w-0">
        <p className="ms-preview-name" title={info.id}>{info.name || shortModelName(info.id)}</p>
        <p className="ms-preview-maker">{family.maker || t('Unknown maker')}<span className="ms-dot" />{t('via')} <ModelLogo logo={host.logo} size={11} /> {host.name}</p>
      </div>
    </div>
    <p className="ms-preview-desc">{describe(info, providerId)}</p>
    <div className="ms-metrics">
      {METRICS.map(item => item.key === 'context' && !metrics.context
        ? <div key={item.key} className="ms-metric"><div className="ms-metric-head"><span>{t('Context')}</span></div><span className="ms-metric-none">{t('Not reported')}</span></div>
        : <MetricBar key={item.key} label={item.label} value={shown[item.key]} note={notes[item.key]} tone={item.tone} />)}
    </div>
    <div className="ms-preview-facts">
      {info.contextWindow ? <span>{t('{n} context', { n: contextLabel(info.contextWindow) })}</span> : null}
      {info.vision && <span><Eye size={11} />{t('Images')}</span>}
      {info.reasoning && <span><Brain size={11} />{t('Reasoning')}</span>}
      {info.tools && <span><Wrench size={11} />{t('Tools')}</span>}
      {info.verified === 'ok' && <span className="ok"><Check size={11} />{t('Checked')}</span>}
      {info.verified === 'unavailable' && <span className="bad" title={info.verifiedReason}><CircleAlert size={11} />{t('Unavailable')}</span>}
    </div>
    {canReason && <div className="ms-config">
      <p className="ms-config-title">{t('Configuration')}</p>
      <Segmented label="Reasoning" value={effort} onChange={onEffort} />
      {!current && <p className="ms-config-note">{t('Applies when you pick this model.')}</p>}
    </div>}
  </div>
}

/** The composer's model selector: a searchable list with maker logos, a hover card that compares the model on
 *  ten-segment bars, and per-model reasoning settings that follow the model when you switch. */
export function ModelSelector({ models, value, onChange, providerId, effort, effortSupported, onEffortChange, disabled }: {
  models: ModelInfo[]; value: string; onChange: (model: string) => void; providerId: string
  effort: Effort; effortSupported: boolean; onEffortChange: (effort: Effort) => void; disabled?: boolean
}) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [filter, setFilter] = useState<ModelFilter>('all')
  const [showUnavailable, setShowUnavailable] = useState(false)
  const [active, setActive] = useState(0)
  const [preview, setPreview] = useState<string | null>(null)
  const [efforts, setEfforts] = useState(readEfforts)
  const root = useRef<HTMLDivElement>(null)
  // Where the menu fits: above the trigger when there is room (the composer sits low), else below, never past the window.
  const [place, setPlace] = useState<{ up: boolean; height: number; style: CSSProperties }>({ up: true, height: 460, style: {} })
  const popup = useRef<HTMLDivElement>(null)
  const search = useRef<HTMLInputElement>(null)
  const list = useRef<HTMLDivElement>(null)
  const hoverTimer = useRef(0)

  const shown = useMemo(() => filterModels(models, { query, filter, showUnavailable, keep: value }), [models, query, filter, showUnavailable, value])
  const groups = groupModels(shown)
  const flat = groups.flatMap(group => group.items)
  const unique = useMemo(() => uniqueShortNames(models), [models])
  const measure = useMemo(() => metricsFor(models, providerId), [models, providerId])
  const hidden = models.filter(info => info.verified === 'unavailable').length
  const current = models.find(info => info.id === value)
  const previewInfo = preview ? models.find(info => info.id === preview) : undefined

  const effortOf = (id: string) => id === value ? effort : efforts[id] ?? 'auto'
  const canReason = (info: ModelInfo) => info.id === value ? effortSupported : info.reasoning

  useEffect(() => {
    if (!open) return
    const away = (event: MouseEvent) => { if (!root.current?.contains(event.target as Node) && !popup.current?.contains(event.target as Node)) setOpen(false) }
    const close = () => setOpen(false)
    window.addEventListener('resize', close)
    document.addEventListener('mousedown', away)
    return () => { document.removeEventListener('mousedown', away); window.removeEventListener('resize', close) }
  }, [open])
  useEffect(() => { list.current?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: 'nearest' }) }, [active])
  useEffect(() => () => window.clearTimeout(hoverTimer.current), [])

  const toggle = (next: boolean) => {
    if (next && root.current) {
      // The menu is portaled and fixed, so no panel clips it. It opens above the trigger when that side
      // has room below the title bar (and any banner), else below, and is only ever as tall as the room.
      const rect = root.current.getBoundingClientRect()
      const ceiling = Math.max(document.querySelector('.titlebar')?.getBoundingClientRect().bottom ?? 0, 0) + 8
      const above = rect.top - 8 - ceiling
      const below = window.innerHeight - rect.bottom - 16
      const up = above >= 420 || above >= below
      const height = Math.max(160, Math.min(460, up ? above : below))
      const right = Math.max(8, window.innerWidth - rect.right)
      setPlace({ up, height, style: up ? { bottom: window.innerHeight - rect.top + 8, right } : { top: rect.bottom + 8, right } })
    }
    setOpen(next)
    setQuery(''); setFilter('all'); setPreview(null)
    if (next) setActive(Math.max(0, filterModels(models, { query: '', filter: 'all', showUnavailable, keep: value }).findIndex(info => info.id === value)))
  }
  const choose = (info: ModelInfo) => {
    toggle(false)
    if (info.id === value) return
    onChange(info.id)
    onEffortChange(efforts[info.id] ?? 'auto')
  }
  const setEffortFor = (id: string, next: Effort) => {
    const updated = { ...efforts, [id]: next }
    setEfforts(updated); writeEfforts(updated)
    if (id === value) onEffortChange(next)
  }
  const hover = (id: string | null, delay: number) => {
    window.clearTimeout(hoverTimer.current)
    hoverTimer.current = window.setTimeout(() => setPreview(id), delay)
  }
  const key = (event: ReactKeyboardEvent) => {
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault()
      const next = flat.length ? (active + (event.key === 'ArrowDown' ? 1 : -1) + flat.length) % flat.length : 0
      setActive(next); setPreview(flat[next]?.id ?? null)
    } else if (event.key === 'Enter') { event.preventDefault(); if (flat[active]) choose(flat[active]) }
    else if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); toggle(false) }
  }

  const triggerEffort = current && effortSupported && effort !== 'auto' ? effort : null
  const label = current ? (unique(current) ? shortModelName(current.id) : current.id) : value ? shortModelName(value) : t('Choose model')

  return <div ref={root} className="model-selector">
    <button type="button" className="ms-trigger" disabled={disabled} aria-haspopup="listbox" aria-expanded={open} aria-label={t('Model: {model}', { model: label })}
      title={current?.id ?? value} onClick={() => toggle(!open)}>
      <ModelLogo logo={modelLogo(value)} size={14} />
      <span className="ms-trigger-label">{label}</span>
      {triggerEffort && <span className="ms-badge">{t(EFFORTS.find(item => item.value === triggerEffort)?.label ?? '')}</span>}
      <ChevronDown size={13} className={cn('ms-chevron', open && 'open')} aria-hidden />
    </button>
    {open && createPortal(<div ref={popup} className={cn('ms-popup', !place.up && 'down')} style={place.style} onKeyDown={key}>
      <div className="ms-panel" style={{ maxHeight: place.height }}>
        <label className="ms-search">
          <input ref={search} autoFocus value={query} onChange={event => { setQuery(event.target.value); setActive(0); setPreview(null) }} onFocus={() => setPreview(null)} placeholder={t('Search {n} models…', { n: models.length })}
            role="combobox" aria-expanded aria-controls="ms-list" aria-activedescendant={flat[active] ? `ms-${active}` : undefined} aria-label={t('Search models')} spellCheck={false} />
          <Search size={13} aria-hidden />
        </label>
        {(models.length > 6 || hidden > 0) && <div className="ms-filters"><ModelFilters models={models} filter={filter} onFilter={next => { setFilter(next); setActive(0) }} hidden={hidden} showUnavailable={showUnavailable} onShowUnavailable={setShowUnavailable} /></div>}
        <div ref={list} id="ms-list" className="ms-list" role="listbox" aria-label={t('Models')} onMouseLeave={() => hover(null, 180)}>
          {groups.map(group => <div key={group.key} role="group" aria-label={t(group.title)}>
            {groups.length > 1 && <p className="ms-group">{t(group.title)}<span>{group.items.length}</span></p>}
            {group.items.map(info => {
              const index = flat.indexOf(info)
              const family = familyOf(info.id)
              const rowEffort = canReason(info) ? effortOf(info.id) : 'auto'
              return <div key={info.id} id={`ms-${index}`} data-index={index} role="option" aria-selected={info.id === value}
                className={cn('ms-row', index === active && 'active', info.id === value && 'selected', info.verified === 'unavailable' && 'unavailable')}
                onMouseEnter={() => { setActive(index); hover(info.id, 80) }} onMouseDown={event => event.preventDefault()} onClick={() => choose(info)}>
                <ModelLogo logo={modelLogo(info.id)} size={16} className="ms-row-logo" />
                <span className="ms-row-text">
                  <span className="ms-row-name">{unique(info) ? shortModelName(info.id) : info.id}</span>
                  <span className="ms-row-maker">{family.maker || info.id.split('/')[0]}</span>
                </span>
                <span className="ms-row-tags">
                  {rowEffort !== 'auto' && <span className="ms-badge">{t(EFFORTS.find(item => item.value === rowEffort)?.label ?? '')}</span>}
                  {info.free && <span className="ms-badge free">{t('free')}</span>}
                  {info.verified === 'unavailable' && <CircleAlert size={12} className="ms-row-bad" aria-label={t('Unavailable')} />}
                  {info.id === value && <Check size={13} className="ms-row-check" aria-hidden />}
                </span>
              </div>
            })}
          </div>)}
          {flat.length === 0 && <p className="ms-empty">{query ? t('No models match “{query}”.', { query }) : t('No models match this filter.')}</p>}
        </div>
      </div>
      {previewInfo && (() => {
        const measured = measure(previewInfo)
        return <div className="ms-preview-wrap" style={{ maxHeight: place.height }} onMouseEnter={() => hover(previewInfo.id, 0)} onMouseLeave={() => hover(null, 180)}>
          <Preview info={previewInfo} providerId={providerId} metrics={measured.metrics} notes={measured.notes} current={previewInfo.id === value}
            effort={effortOf(previewInfo.id)} canReason={canReason(previewInfo)} onEffort={next => setEffortFor(previewInfo.id, next)} />
        </div>
      })()}
    </div>, document.body)}
  </div>
}
