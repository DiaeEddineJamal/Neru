import { useEffect, useMemo, useRef, useState } from 'react'
import { CaseSensitive, ChevronDown, FileText, LoaderCircle, Regex, Search, WholeWord } from 'lucide-react'
import { api } from '../../api'
import type { SearchHit } from '../../types'
import { cn } from '@/lib/utils'

/** Project-wide search, like VS Code's: results as you type, grouped by file, match highlighted. */
export function SearchPanel({ projectKey, onOpen }: { projectKey: string; onOpen: (path: string, line: number) => void }) {
  const [query, setQuery] = useState('')
  const [caseSensitive, setCaseSensitive] = useState(false)
  const [wholeWord, setWholeWord] = useState(false)
  const [regex, setRegex] = useState(false)
  const [hits, setHits] = useState<SearchHit[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set())
  const run = useRef(0)
  const input = useRef<HTMLInputElement>(null)

  useEffect(() => { input.current?.focus() }, [projectKey])
  // Debounced so each keystroke does not walk the whole project; stale answers are dropped.
  useEffect(() => {
    const text = query.trim()
    if (text.length < 2) { setHits([]); setError(''); setBusy(false); return }
    const id = ++run.current
    setBusy(true)
    const timer = window.setTimeout(() => {
      api.searchText(text, { caseSensitive, wholeWord, regex })
        .then(found => { if (id === run.current) { setHits(found); setError('') } })
        .catch(cause => { if (id === run.current) { setHits([]); setError(String(cause)) } })
        .finally(() => { if (id === run.current) setBusy(false) })
    }, 220)
    return () => window.clearTimeout(timer)
  }, [query, caseSensitive, wholeWord, regex, projectKey])

  const groups = useMemo(() => {
    const byFile = new Map<string, SearchHit[]>()
    for (const hit of hits) byFile.set(hit.path, [...(byFile.get(hit.path) ?? []), hit])
    return [...byFile.entries()]
  }, [hits])

  const toggle = (value: boolean, set: (next: boolean) => void) => () => set(!value)
  return <div className="search-panel">
    <div className="search-form">
      <Search size={17} aria-hidden />
      <input ref={input} value={query} onChange={event => setQuery(event.target.value)} placeholder="Search text across the project…" aria-label="Search the project" spellCheck={false} />
      {busy && <LoaderCircle size={15} className="animate-spin search-spinner" aria-label="Searching" />}
      <button type="button" className={cn('search-option', caseSensitive && 'on')} aria-pressed={caseSensitive} title="Match case" onClick={toggle(caseSensitive, setCaseSensitive)}><CaseSensitive size={16} /></button>
      <button type="button" className={cn('search-option', wholeWord && 'on')} aria-pressed={wholeWord} title="Whole word" onClick={toggle(wholeWord, setWholeWord)}><WholeWord size={16} /></button>
      <button type="button" className={cn('search-option', regex && 'on')} aria-pressed={regex} title="Regular expression" onClick={toggle(regex, setRegex)}><Regex size={16} /></button>
    </div>
    <p className="search-summary" aria-live="polite">
      {error ? <span className="search-error">{error.replace(/^Error:\s*/, '')}</span>
        : query.trim().length < 2 ? 'Type at least two characters. Files Git ignores are skipped.'
        : busy && hits.length === 0 ? 'Searching…'
        : hits.length === 0 ? `No results for “${query.trim()}”.`
        : `${hits.length >= 500 ? '500+' : hits.length} ${hits.length === 1 ? 'result' : 'results'} in ${groups.length} ${groups.length === 1 ? 'file' : 'files'}`}
    </p>
    <div className="search-groups">
      {groups.map(([path, fileHits]) => {
        const closed = collapsed.has(path)
        return <section key={path} className="search-group">
          <button type="button" className="search-file" aria-expanded={!closed} onClick={() => setCollapsed(current => { const next = new Set(current); if (closed) next.delete(path); else next.add(path); return next })}>
            <ChevronDown size={14} className={cn('search-chevron', closed && 'closed')} aria-hidden />
            <FileText size={14} aria-hidden />
            <span className="search-file-name">{path.split(/[\\/]/).pop()}</span>
            <span className="search-file-dir truncate">{path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : ''}</span>
            <span className="search-file-count">{fileHits.length}</span>
          </button>
          {!closed && fileHits.map((hit, index) => {
            const before = [...hit.preview].slice(0, hit.column).join('')
            const match = [...hit.preview].slice(hit.column, hit.column + hit.length).join('')
            const after = [...hit.preview].slice(hit.column + hit.length).join('')
            return <button type="button" key={`${hit.line}-${index}`} className="search-hit" onClick={() => onOpen(hit.path, hit.line)} title={`${hit.path}:${hit.line}`}>
              <span className="search-line">{hit.line}</span>
              <code>{before}<mark>{match}</mark>{after}</code>
            </button>
          })}
        </section>
      })}
    </div>
  </div>
}
