import { useCallback, useEffect, useState } from 'react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { LoaderCircle, Pencil, RefreshCw, Sparkles } from '@/components/icons'
import { t } from '../../i18n'
import { memoryApi, memoryTime, onMemoryChanged, type MemorySummary as Summary } from '@/lib/memoryApi'
import './MemorySummary.css'

/**
 * The memory summary Neru writes from past chats, about once a day: the global one (no `projectId`) in
 * Settings → Memory, or a Chat project's own in its Memory panel. The user can read it, edit it, have
 * it rewritten now, or tell Neru what to change.
 */
export function MemorySummary({ projectId, onError, compact }: { projectId?: string; onError: (message: string) => void; compact?: boolean }) {
  const [summary, setSummary] = useState<Summary | null>(null)
  const [draft, setDraft] = useState<string | null>(null)
  const [instruction, setInstruction] = useState('')
  const [working, setWorking] = useState<'regenerate' | 'revise' | 'save' | null>(null)

  const load = useCallback(() => memoryApi.summary(projectId).then(setSummary).catch(cause => onError(String(cause))), [projectId, onError])
  useEffect(() => { void load() }, [load])
  useEffect(() => onMemoryChanged(() => { void memoryApi.summary(projectId).then(setSummary).catch(() => undefined) }), [projectId])
  // A background rewrite shows up when it is done.
  useEffect(() => {
    if (!summary?.generating || working) return
    const timer = window.setInterval(() => { void memoryApi.summary(projectId).then(setSummary).catch(() => undefined) }, 3000)
    return () => window.clearInterval(timer)
  }, [summary?.generating, working, projectId])

  const run = async (kind: 'regenerate' | 'revise' | 'save', work: () => Promise<Summary>) => {
    setWorking(kind)
    try { setSummary(await work()); return true } catch (cause) { onError(String(cause)); return false } finally { setWorking(null) }
  }
  const generating = working === 'regenerate' || working === 'revise' || Boolean(summary?.generating)
  const text = summary?.text.trim() ?? ''
  const place = projectId ? t('the chats in this project') : t('your past chats')

  return <section className={compact ? 'memory-summary compact' : 'memory-summary'} aria-label={t('Memory summary')} aria-busy={generating}>
    <header className="memory-summary-head">
      <h3>{t('Memory summary')}</h3>
      <span className="memory-summary-when">
        {summary && (summary.updatedAt
          ? summary.edited ? t('Edited by you {when}', { when: memoryTime(summary.updatedAt) }) : t('Updated {when}', { when: memoryTime(summary.updatedAt) })
          : t('Not written yet'))}
      </span>
      <span className="memory-summary-actions">
        {draft === null && <button type="button" className="button subtle small" disabled={!summary || generating} onClick={() => setDraft(summary?.text ?? '')}><Pencil size={13} aria-hidden /> {t('Edit')}</button>}
        <button type="button" className="button subtle small" disabled={!summary || generating || draft !== null || !summary.enabled} title={summary && !summary.enabled ? t('Turn on "Generate memory from chat history" first.') : undefined}
          onClick={() => void run('regenerate', () => memoryApi.regenerateSummary(projectId))}>
          {generating ? <LoaderCircle size={13} className="animate-spin" aria-hidden /> : <RefreshCw size={13} aria-hidden />} {generating ? t('Updating…') : t('Regenerate')}
        </button>
      </span>
    </header>

    {draft !== null
      ? <form className="memory-summary-edit" onSubmit={event => { event.preventDefault(); void run('save', () => memoryApi.setSummary(draft, projectId)).then(ok => { if (ok) setDraft(null) }) }}>
        <textarea autoFocus rows={compact ? 8 : 12} maxLength={8000} value={draft} onChange={event => setDraft(event.target.value)} aria-label={t('Memory summary')} onKeyDown={event => { if (event.key === 'Escape') setDraft(null) }} />
        <div className="memory-summary-row">
          <button type="button" className="button subtle small" onClick={() => setDraft(null)}>{t('Cancel')}</button>
          <button type="submit" className="button primary small" disabled={working === 'save' || draft === summary?.text}>{t('Save summary')}</button>
        </div>
      </form>
      : text
        ? <div className="memory-summary-text"><ReactMarkdown remarkPlugins={[remarkGfm]}>{text}</ReactMarkdown></div>
        : summary && <p className="cli-note">{summary.enabled
          ? t('Neru writes this from {place} about once a day. Chat a little, or regenerate it now.', { place })
          : t('Memory from chat history is off, so Neru does not write a summary. You can still write one yourself.')}</p>}

    {draft === null && text && <form className="memory-summary-row memory-summary-revise" onSubmit={event => { event.preventDefault(); const ask = instruction.trim(); if (ask) void run('revise', () => memoryApi.reviseSummary(ask, projectId)).then(ok => { if (ok) setInstruction('') }) }}>
      <Sparkles size={14} aria-hidden />
      <input value={instruction} maxLength={2000} onChange={event => setInstruction(event.target.value)} placeholder={t('Tell Neru what to change, e.g. "forget my old job"')} aria-label={t('Tell Neru what to change in the summary')} disabled={generating} />
      <button type="submit" className="button subtle small" disabled={!instruction.trim() || generating}>{t('Apply')}</button>
    </form>}
  </section>
}
