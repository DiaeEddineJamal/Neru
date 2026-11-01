import { useEffect, useMemo, useState } from 'react'
import { Brain, FileText, Pencil, Plus, Search, Trash2 } from '@/components/icons'
import { api } from '../../api'
import type { MemoryCategory, MemoryPrefs, MemoryStore, SavedMemory } from '../../types'
import { cn } from '@/lib/utils'
import { t } from '../../i18n'
import { Select } from '../ui/Select'
import './MemorySettings.css'

const CATEGORY_LABELS: Record<MemoryCategory, string> = { about: 'About you', preferences: 'Preferences', work: 'Work', projects: 'Projects', other: 'Other' }
const CATEGORIES = Object.keys(CATEGORY_LABELS) as MemoryCategory[]
const TOGGLES: { key: keyof MemoryPrefs; label: string; detail: string }[] = [
  { key: 'referenceMemories', label: 'Reference saved memories', detail: 'Every model, in Chat and Code, sees the memories below as facts about you.' },
  { key: 'learnFromChats', label: 'Learn from chats automatically', detail: 'After a chat reply, a small model notes lasting facts about you. Never passwords, keys, health or other sensitive details, and it follows your privacy rule.' },
  { key: 'referenceChatHistory', label: 'Reference chat history', detail: 'Models can search your earlier chats when you mention something you discussed before.' },
]

const day = (millis: number) => new Date(millis).toLocaleDateString(undefined, { year: 'numeric', month: 'short', day: 'numeric' })

/** A category dropdown for a memory being written or edited. */
function CategorySelect({ value, onChange, label }: { value: MemoryCategory; onChange: (value: MemoryCategory) => void; label: string }) {
  return <Select className="settings-select memory-category" aria-label={label} value={value} onChange={event => onChange(event.target.value as MemoryCategory)}>
    {CATEGORIES.map(category => <option key={category} value={category}>{t(CATEGORY_LABELS[category])}</option>)}
  </Select>
}

/** Text and category for a new memory or an edit; Enter saves, Escape cancels. */
function MemoryForm({ initial, category: initialCategory, submit, onSave, onCancel }: { initial: string; category: MemoryCategory; submit: string; onSave: (text: string, category: MemoryCategory) => void; onCancel: () => void }) {
  const [text, setText] = useState(initial)
  const [category, setCategory] = useState(initialCategory)
  return <form className="memory-form" onSubmit={event => { event.preventDefault(); if (text.trim()) onSave(text.trim(), category) }} onKeyDown={event => { if (event.key === 'Escape') onCancel() }}>
    <input autoFocus value={text} maxLength={500} onChange={event => setText(event.target.value)} placeholder={t('e.g. Prefers metric units')} aria-label={t('Memory')} />
    <CategorySelect value={category} onChange={setCategory} label={t('Category')} />
    <button type="submit" className="button primary small" disabled={!text.trim()}>{t(submit)}</button>
    <button type="button" className="button subtle small" onClick={onCancel}>{t('Cancel')}</button>
  </form>
}

/**
 * Settings → Memory: what every model knows about the user (saved memories), whether Neru learns from chats
 * and searches past ones, and the list itself to search, edit and delete.
 */
export function MemorySettings({ onError, onNotice }: { onError: (message: string) => void; onNotice: (message: string) => void }) {
  const [store, setStore] = useState<MemoryStore | null>(null)
  const [query, setQuery] = useState('')
  const [adding, setAdding] = useState(false)
  const [editing, setEditing] = useState<string | null>(null)
  const [confirmClear, setConfirmClear] = useState(false)

  useEffect(() => { void api.memoryStore().then(setStore).catch(cause => onError(String(cause))) }, [onError])
  // Facts Neru learns from a chat in the background show up while the page is open.
  useEffect(() => {
    let stop: (() => void) | undefined, live = true
    void import('@tauri-apps/api/event').then(({ listen }) => listen('memory-changed', () => { void api.memoryStore().then(setStore).catch(() => undefined) })).then(unlisten => { if (live) stop = unlisten; else unlisten() }).catch(() => undefined)
    return () => { live = false; stop?.() }
  }, [])

  /** Applies a change; resolves to whether it worked, so a form stays open after an error. */
  const run = (work: Promise<MemoryStore>, notice?: string) => work.then(next => { setStore(next); if (notice) onNotice(notice); return true }).catch(cause => { onError(String(cause)); return false })
  const toggle = (key: keyof MemoryPrefs) => { if (store) void run(api.setMemoryPrefs({ ...store.settings, [key]: !store.settings[key] })) }

  const shown = useMemo(() => {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean)
    return [...(store?.memories ?? [])]
      .filter(memory => words.every(word => `${memory.text} ${t(CATEGORY_LABELS[memory.category] ?? 'Other')}`.toLowerCase().includes(word)))
      .sort((a, b) => b.updatedAt - a.updatedAt || a.id.localeCompare(b.id))
  }, [store, query])
  const memories = store?.memories ?? []

  const row = (memory: SavedMemory) => editing === memory.id
    ? <li key={memory.id} className="memory-item editing"><MemoryForm initial={memory.text} category={memory.category} submit="Save" onCancel={() => setEditing(null)} onSave={(text, category) => void run(api.editMemory(memory.id, text, category)).then(ok => { if (ok) setEditing(null) })} /></li>
    : <li key={memory.id} className="memory-item">
      <div className="memory-body">
        <span className="memory-text">{memory.text}</span>
        <span className="memory-meta">
          <span className="memory-tag">{t(CATEGORY_LABELS[memory.category] ?? 'Other')}</span>
          <span className={cn('memory-source', memory.source)}>{memory.source === 'user' ? t('You said') : t('Neru noticed')}</span>
          <span>{day(memory.updatedAt)}</span>
        </span>
      </div>
      <div className="memory-actions">
        <button type="button" className="icon-button small" aria-label={t('Edit: {item}', { item: memory.text })} title={t('Edit')} onClick={() => { setAdding(false); setEditing(memory.id) }}><Pencil size={13} /></button>
        <button type="button" className="icon-button small" aria-label={t('Delete: {item}', { item: memory.text })} title={t('Delete')} onClick={() => void run(api.deleteMemory(memory.id))}><Trash2 size={13} /></button>
      </div>
    </li>

  return <section className="settings-section memory-settings"><h2>{t('Memory')}</h2>
    <p className="settings-lede">{t('What Neru remembers about you, for every model, free or paid. Ask in any chat: "remember that…" or "forget that…".')}</p>

    {TOGGLES.map(item => <div key={item.key} className="settings-row"><div><strong>{t(item.label)}</strong><p>{t(item.detail)}</p></div>
      <button type="button" className={cn('switch', store?.settings[item.key] && 'on')} role="switch" aria-checked={Boolean(store?.settings[item.key])} aria-label={t(item.label)} disabled={!store} onClick={() => toggle(item.key)} /></div>)}

    <h3 className="cli-heading"><Brain size={15} aria-hidden /> {t('Saved memories')}{memories.length > 0 && <span className="memory-count">{memories.length}</span>}</h3>
    <div className="memory-toolbar">
      <label className="memory-search"><Search size={14} aria-hidden /><input type="search" value={query} onChange={event => setQuery(event.target.value)} placeholder={t('Search memories')} aria-label={t('Search memories')} /></label>
      <button type="button" className="button subtle small" onClick={() => { setEditing(null); setAdding(true) }} disabled={adding}><Plus size={13} aria-hidden /> {t('Add memory')}</button>
      <button type="button" className="button subtle small memory-danger" onClick={() => setConfirmClear(true)} disabled={!memories.length || confirmClear}><Trash2 size={13} aria-hidden /> {t('Delete all')}</button>
    </div>

    {confirmClear && <div className="memory-confirm" role="alertdialog" aria-label={t('Delete all memories')}>
      <span>{memories.length === 1 ? t('Delete your 1 saved memory? This cannot be undone.') : t('Delete all {n} saved memories? This cannot be undone.', { n: memories.length })}</span>
      <button type="button" className="button primary small memory-danger-solid" onClick={() => void run(api.clearMemories(), t('Deleted all saved memories.')).then(ok => { if (ok) setConfirmClear(false) })}>{t('Delete all')}</button>
      <button type="button" className="button subtle small" onClick={() => setConfirmClear(false)}>{t('Cancel')}</button>
    </div>}

    {adding && <MemoryForm initial="" category="about" submit="Add" onCancel={() => setAdding(false)} onSave={(text, category) => void run(api.addMemory(text, category)).then(ok => { if (ok) setAdding(false) })} />}

    {store && !memories.length && !adding && <p className="cli-note">{t('Nothing saved yet. Tell Neru "remember that I prefer short answers", or leave learning on and it will note lasting facts as you chat.')}</p>}
    {memories.length > 0 && !shown.length && <p className="cli-note">{t('No memories match "{query}".', { query })}</p>}
    <ul className="memory-list">{shown.map(row)}</ul>

    <p className="cli-note memory-file">{t('Neru also reads your personal memory file, written by hand.')} <button type="button" className="button subtle small" onClick={() => void api.openMemoryFile('user').catch(cause => onError(String(cause)))}><FileText size={13} aria-hidden /> {t('Open memory file')}</button></p>
  </section>
}
