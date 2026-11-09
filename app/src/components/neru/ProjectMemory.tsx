import { useCallback, useEffect, useState } from 'react'
import { Plus, Trash2 } from '@/components/icons'
import { t } from '../../i18n'
import { toast } from '@/lib/toast'
import { memoryApi, onMemoryChanged } from '@/lib/memoryApi'
import type { SavedMemory } from '../../types'
import { MemorySummary } from './MemorySummary'

/**
 * A Chat project's own memory, kept apart from the global one: the summary Neru writes from the
 * project's chats and the facts learned in them. Only chats in the project use it.
 */
export function ProjectMemory({ projectId }: { projectId: string }) {
  const [facts, setFacts] = useState<SavedMemory[] | null>(null)
  const [adding, setAdding] = useState('')
  const onError = useCallback((message: string) => toast.error(message), [])
  const load = useCallback(() => memoryApi.projectMemories(projectId).then(store => setFacts(store.memories)).catch(() => setFacts([])), [projectId])
  useEffect(() => { void load() }, [load])
  useEffect(() => onMemoryChanged(() => { void load() }), [load])

  const apply = async (work: Promise<{ memories: SavedMemory[] }>) => {
    try { setFacts((await work).memories); return true } catch (cause) { onError(String(cause)); return false }
  }
  const sorted = [...(facts ?? [])].sort((a, b) => b.updatedAt - a.updatedAt || a.id.localeCompare(b.id))

  return <section className="cp-panel-block" aria-label={t('Project memory')}>
    <header><h2>{t('Memory')}</h2></header>
    <p className="cp-muted">{t('Only chats in this project use this memory, and it stays out of your other chats.')}</p>
    <MemorySummary projectId={projectId} onError={onError} compact />
    {sorted.length > 0 && <ul className="cp-memory-facts" aria-label={t('Project facts')}>
      {sorted.map(fact => <li key={fact.id}>
        <span>{fact.text}</span>
        <button type="button" className="icon-button small" aria-label={t('Delete: {item}', { item: fact.text })} title={t('Delete')} onClick={() => void apply(memoryApi.deleteMemory(fact.id, projectId))}><Trash2 size={13} /></button>
      </li>)}
    </ul>}
    <form className="cp-memory-add" onSubmit={event => { event.preventDefault(); const text = adding.trim(); if (text) void apply(memoryApi.addMemory(text, 'projects', projectId)).then(ok => { if (ok) setAdding('') }) }}>
      <input value={adding} maxLength={500} onChange={event => setAdding(event.target.value)} placeholder={t('Add a fact for this project')} aria-label={t('Add a fact for this project')} />
      <button type="submit" className="icon-button" disabled={!adding.trim()} aria-label={t('Add fact')}><Plus size={15} /></button>
    </form>
  </section>
}
