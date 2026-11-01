import { useEffect, useRef, useState } from 'react'
import { Check, Columns3, LoaderCircle, MessageCircleQuestion, Plus, Send, X } from 'lucide-react'
import { api } from '../../api'
import { t } from '../../i18n'
import { cn } from '@/lib/utils'
import type { ModelInfo, ProviderView, SideAnswer } from '../../types'
import { formatForModel, providerPresets, type ApiFormat } from '../../providerCatalog'
import { RESPONSE_PROSE, ResponseMarkdown } from './Conversation'
import './SidePanels.css'

interface SideItem { id: number; question: string; answer?: SideAnswer; error?: string }
/** Side questions per session, kept while the app runs so closing the panel loses nothing. */
const asked = new Map<string, SideItem[]>()
let nextId = 1

const seconds = (ms: number) => `${(ms / 1000).toFixed(ms < 10_000 ? 1 : 0)}s`

/** `/btw`: quick questions to the same model in a side panel; nothing enters the main conversation. */
export function BtwPanel({ sessionId, seed, onClose }: { sessionId: string | null; seed: string; onClose: () => void }) {
  const key = sessionId ?? ''
  const [items, setItems] = useState<SideItem[]>(() => asked.get(key) ?? [])
  const [input, setInput] = useState('')
  const end = useRef<HTMLDivElement>(null)
  const seeded = useRef('')
  const update = (change: (current: SideItem[]) => SideItem[]) => setItems(current => { const next = change(current); asked.set(key, next); return next })

  const ask = (question: string) => {
    const text = question.trim()
    if (!text) return
    const id = nextId++
    update(current => [...current, { id, question: text }])
    api.askSideQuestion(text, sessionId)
      .then(answer => update(current => current.map(item => item.id === id ? { ...item, answer } : item)))
      .catch(cause => update(current => current.map(item => item.id === id ? { ...item, error: String(cause) } : item)))
  }
  useEffect(() => {
    if (!seed || seeded.current === seed) return
    seeded.current = seed
    ask(seed)
  })
  useEffect(() => { end.current?.scrollIntoView({ block: 'end' }) }, [items])

  return <aside className="side-panel btw-panel" aria-label={t('Side question')}>
    <header className="side-panel-head">
      <MessageCircleQuestion size={15} />
      <strong>{t('By the way')}</strong>
      <small className="side-panel-hint">{t('Not added to the conversation')}</small>
      <button type="button" className="icon-button small" onClick={onClose} title={t('Close')} aria-label={t('Close side questions')}><X size={14} /></button>
    </header>
    <div className="btw-list">
      {items.length === 0 && <p className="side-panel-empty">{t('Ask the same model something quick while the main task carries on. It sees the conversation, but nothing here goes into it.')}</p>}
      {items.map(item => <div key={item.id} className="btw-item">
        <p className="btw-question">{item.question}</p>
        {item.answer ? <>
          <div className={RESPONSE_PROSE}><ResponseMarkdown content={item.answer.text} sources={[]} idPrefix={`btw-${item.id}`} onCite={() => undefined} /></div>
          <small className="btw-meta">{item.answer.model} · {seconds(item.answer.ms)}</small>
        </> : item.error ? <p className="side-panel-error">{item.error}</p> : <p className="side-panel-empty"><LoaderCircle size={14} className="animate-spin" /> {t('Thinking…')}</p>}
      </div>)}
      <div ref={end} />
    </div>
    <form className="btw-form" onSubmit={event => { event.preventDefault(); ask(input); setInput('') }}>
      <input autoFocus value={input} onChange={event => setInput(event.target.value)} placeholder={t('Ask a side question…')} aria-label={t('Side question')}
        onKeyDown={event => { if (event.key === 'Escape') { event.stopPropagation(); onClose() } }} />
      <button type="submit" className="icon-button small" disabled={!input.trim()} aria-label={t('Ask')}><Send size={14} /></button>
    </form>
  </aside>
}

export interface CompareTarget { providerId: string; apiFormat: ApiFormat; baseUrl: string; model: string }
interface Column { target: CompareTarget; answer?: SideAnswer; error?: string }
const targetKey = (target: CompareTarget) => `${target.providerId} ${target.model}`
const providerName = (id: string) => providerPresets.find(preset => preset.id === id)?.name ?? id

/** Sends one prompt to two or three models on any saved key and shows the answers side by side. */
export function ComparePanel({ sessionId, seed, provider, models, savedKeys, onKeep, onClose, onError }: {
  sessionId: string | null
  seed: string
  provider: ProviderView
  /** The current provider's models, already loaded. */
  models: ModelInfo[]
  /** Providers with a saved key. */
  savedKeys: string[]
  onKeep: (prompt: string, answer: SideAnswer, target: CompareTarget) => void
  onClose: () => void
  onError: (message: string) => void
}) {
  const [prompt, setPrompt] = useState(seed)
  const [picked, setPicked] = useState<CompareTarget[]>(() => provider.model ? [{ providerId: provider.providerId, apiFormat: provider.apiFormat, baseUrl: provider.baseUrl, model: provider.model }] : [])
  const [choices, setChoices] = useState<CompareTarget[]>(() => models.filter(info => info.verified !== 'unavailable').map(info => ({ providerId: provider.providerId, apiFormat: formatForModel(provider.providerId, info.id, provider.apiFormat, provider.baseUrl), baseUrl: provider.baseUrl, model: info.id })))
  const otherKeys = (keys: string[]) => providerPresets.filter(preset => keys.includes(preset.id) && preset.id !== provider.providerId)
  const [loading, setLoading] = useState(() => otherKeys(savedKeys).length)
  const [filter, setFilter] = useState('')
  const [columns, setColumns] = useState<Column[]>([])
  const [sent, setSent] = useState('')

  useEffect(() => {
    // Models on the other saved keys, listed with each key as it is saved.
    let alive = true
    for (const preset of providerPresets.filter(preset => savedKeys.includes(preset.id) && preset.id !== provider.providerId)) {
      api.listModels(preset.id, preset.format, preset.baseUrl, '')
        .then(list => { if (alive) setChoices(current => [...current, ...list.filter(info => info.verified !== 'unavailable').map(info => ({ providerId: preset.id, apiFormat: formatForModel(preset.id, info.id, preset.format, preset.baseUrl), baseUrl: preset.baseUrl, model: info.id }))]) })
        .catch(() => undefined)
        .finally(() => { if (alive) setLoading(count => count - 1) })
    }
    return () => { alive = false }
  }, [savedKeys, provider.providerId])

  const terms = filter.toLowerCase().split(/\s+/).filter(Boolean)
  const matches = choices.filter(choice => !picked.some(item => targetKey(item) === targetKey(choice)) && terms.every(term => `${choice.model} ${choice.providerId}`.toLowerCase().includes(term))).slice(0, 40)
  const running = columns.some(column => !column.answer && !column.error)

  const run = () => {
    const text = prompt.trim()
    if (!text || picked.length < 2) return
    setSent(text)
    setColumns(picked.map(target => ({ target })))
    for (const target of picked) {
      api.compareModel(text, target, sessionId)
        .then(answer => setColumns(current => current.map(column => targetKey(column.target) === targetKey(target) ? { ...column, answer } : column)))
        .catch(cause => setColumns(current => current.map(column => targetKey(column.target) === targetKey(target) ? { ...column, error: String(cause) } : column)))
    }
  }

  return <div className="compare-overlay" role="dialog" aria-modal="true" aria-label={t('Compare models')} onKeyDown={event => { if (event.key === 'Escape') { event.stopPropagation(); onClose() } }}>
    <div className="compare-panel">
      <header className="side-panel-head">
        <Columns3 size={15} />
        <strong>{t('Compare models')}</strong>
        <small className="side-panel-hint">{t('One prompt, up to three models. Keep the answer you like.')}</small>
        <button type="button" className="icon-button small" onClick={onClose} title={t('Close')} aria-label={t('Close compare')}><X size={14} /></button>
      </header>
      <div className="compare-setup">
        <textarea rows={3} value={prompt} autoFocus onChange={event => setPrompt(event.target.value)} placeholder={t('What should every model answer?')} aria-label={t('Prompt')}
          onKeyDown={event => { if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) { event.preventDefault(); run() } }} />
        <div className="compare-picked">
          {picked.map(target => <span key={targetKey(target)} className="compare-chip"><small>{providerName(target.providerId)}</small> {target.model}
            <button type="button" aria-label={t('Remove {model}', { model: target.model })} onClick={() => setPicked(current => current.filter(item => targetKey(item) !== targetKey(target)))}><X size={12} /></button></span>)}
          {picked.length < 3 && <input className="compare-filter" value={filter} onChange={event => setFilter(event.target.value)} placeholder={loading > 0 ? t('Loading models…') : t('Add a model…')} aria-label={t('Find a model to add')} />}
          <button type="button" className="button primary small" disabled={picked.length < 2 || !prompt.trim() || running} onClick={run} title={picked.length < 2 ? t('Pick at least two models') : undefined}>{running ? <LoaderCircle size={13} className="animate-spin" /> : <Send size={13} />} {t('Compare')}</button>
        </div>
        {picked.length < 3 && filter && <div className="compare-choices" role="listbox" aria-label={t('Models')}>
          {matches.length === 0 && <span className="side-panel-empty">{loading > 0 ? t('Loading models…') : t('No model matches.')}</span>}
          {matches.map(choice => <button key={targetKey(choice)} type="button" className="slash-item" onClick={() => { setPicked(current => [...current, choice].slice(0, 3)); setFilter('') }}>
            <Plus size={12} /><code>{choice.model}</code><span>{providerName(choice.providerId)}</span>
          </button>)}
        </div>}
      </div>
      {columns.length > 0 && <div className={cn('compare-columns', `count-${columns.length}`)}>{columns.map(column => <section key={targetKey(column.target)} className="compare-column">
        <header><strong>{column.target.model}</strong><small>{providerName(column.target.providerId)}{column.answer ? ` · ${seconds(column.answer.ms)}` : ''}</small></header>
        <div className="compare-answer">
          {column.answer ? <div className={RESPONSE_PROSE}><ResponseMarkdown content={column.answer.text} sources={[]} idPrefix={`compare-${targetKey(column.target).replace(/\W+/g, '-')}`} onCite={() => undefined} /></div>
            : column.error ? <p className="side-panel-error">{column.error}</p>
            : <p className="side-panel-empty"><LoaderCircle size={14} className="animate-spin" /> {t('Waiting for the answer…')}</p>}
        </div>
        {column.answer && <footer><button type="button" className="button small" onClick={() => { const answer = column.answer; if (!answer) return; try { onKeep(sent, answer, column.target) } catch (cause) { onError(String(cause)) } }}><Check size={13} /> {t('Continue with this')}</button></footer>}
      </section>)}</div>}
    </div>
  </div>
}
