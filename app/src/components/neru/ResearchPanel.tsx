import { useEffect, useState } from 'react'
import { BookMarked, Check, CircleAlert, Copy, Download, LoaderCircle } from 'lucide-react'
import { useReducedMotion } from 'motion/react'
import { cn } from '@/lib/utils'
import { setCitationStyle, useCitationStyle, saveTextFile } from '@/lib/citations'
import { api } from '../../api'
import type { CitationStyle, ResearchStep, Source } from '../../types'

const STYLES: { value: CitationStyle; label: string }[] = [
  { value: 'apa', label: 'APA 7' },
  { value: 'mla', label: 'MLA 9' },
  { value: 'chicago', label: 'Chicago' },
  { value: 'ieee', label: 'IEEE' },
  { value: 'harvard', label: 'Harvard' },
]

/** *Italic* spans in a formatted reference, without a Markdown parser. */
function Entry({ text }: { text: string }) {
  return <>{text.split(/(\*[^*]+\*)/g).map((part, index) => part.startsWith('*') && part.endsWith('*') && part.length > 2 ? <em key={index}>{part.slice(1, -1)}</em> : part)}</>
}

/** Research mode's plan and progress while Neru gathers sources. */
export function ResearchProgress({ steps, live }: { steps: ResearchStep[]; live: boolean }) {
  const reduce = useReducedMotion() ?? false
  if (!steps.length) return null
  return <div className="mb-3 rounded-xl border border-border px-3 py-2.5 text-sm" role="status" aria-live="polite" aria-label="Research progress">
    <p className="mb-1.5 flex items-center gap-1.5 text-xs font-medium uppercase tracking-wide text-muted-foreground"><BookMarked size={13} aria-hidden />Research plan</p>
    <ol className="flex flex-col gap-1">
      {steps.map((step, index) => <li key={index} className="flex items-start gap-2">
        <span className="mt-0.5 shrink-0" aria-hidden>
          {step.status === 'running' && live ? <LoaderCircle size={13} className={cn(!reduce && 'animate-spin')} />
            : step.status === 'error' ? <CircleAlert size={13} className="text-destructive" />
            : <Check size={13} className="text-[var(--sage)]" />}
        </span>
        <span className="min-w-0"><span className="text-foreground">{step.label}</span>{step.detail && <span className="text-muted-foreground"> · {step.detail}</span>}</span>
      </li>)}
    </ol>
  </div>
}

const scholarly = (sources: Source[]) => sources.some(source => Boolean(source.doi) || Boolean(source.authors?.length))

/** The answer's sources as a reference list in the chosen style, to copy or export. Shown for scholarly sources. */
export function Bibliography({ sources }: { sources: Source[] }) {
  const current = useCitationStyle()
  const [entries, setEntries] = useState<string[]>([])
  const [note, setNote] = useState('')
  const show = scholarly(sources)
  useEffect(() => {
    if (!show) return
    let live = true
    void api.formatBibliography(sources, current, true).then(list => { if (live) setEntries(list) }).catch(() => undefined)
    return () => { live = false }
  }, [sources, current, show])
  useEffect(() => { if (!note) return; const timer = window.setTimeout(() => setNote(''), 2_500); return () => window.clearTimeout(timer) }, [note])
  if (!show) return null
  const copy = async () => {
    try {
      const list = await api.formatBibliography(sources, current, false)
      await navigator.clipboard.writeText(list.map(entry => entry.replace(/\*([^*]+)\*/g, '$1')).join('\n'))
      setNote('Copied')
    } catch { setNote('Could not copy') }
  }
  const exportAs = async (format: 'bibtex' | 'ris') => {
    try {
      const text = await api.exportCitations(sources, format)
      const saved = await saveTextFile(format === 'bibtex' ? 'references.bib' : 'references.ris', format === 'bibtex' ? { name: 'BibTeX', extension: 'bib' } : { name: 'RIS', extension: 'ris' }, text)
      if (saved) setNote('Saved')
    } catch (cause) { setNote(cause instanceof Error ? cause.message : String(cause)) }
  }
  const button = 'inline-flex h-7 items-center gap-1 rounded-lg px-2 text-xs text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring outline-none'
  return <section className="mt-4 rounded-xl border border-border px-3 py-2.5" aria-label="Bibliography">
    <div className="mb-2 flex flex-wrap items-center gap-1">
      <p className="mr-auto text-xs font-medium uppercase tracking-wide text-muted-foreground">Bibliography</p>
      <label className="sr-only" htmlFor="citation-style">Citation style</label>
      <select id="citation-style" value={current} onChange={event => setCitationStyle(event.target.value as CitationStyle)} className="h-7 rounded-lg border-0 bg-transparent px-1.5 text-xs text-muted-foreground hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring outline-none">
        {STYLES.map(item => <option key={item.value} value={item.value}>{item.label}</option>)}
      </select>
      <button type="button" className={button} onClick={() => void copy()} title="Copy the reference list"><Copy size={12} aria-hidden />Copy</button>
      <button type="button" className={button} onClick={() => void exportAs('bibtex')} title="Save as BibTeX for LaTeX, Zotero or Mendeley"><Download size={12} aria-hidden />BibTeX</button>
      <button type="button" className={button} onClick={() => void exportAs('ris')} title="Save as RIS for EndNote, Zotero or Mendeley"><Download size={12} aria-hidden />RIS</button>
      {note && <span className="text-xs text-muted-foreground" role="status">{note}</span>}
    </div>
    <ol className="flex flex-col gap-1.5 text-sm leading-6 text-foreground">
      {entries.map((entry, index) => <li key={index} className="break-words [overflow-wrap:anywhere]"><Entry text={entry} /></li>)}
    </ol>
  </section>
}
