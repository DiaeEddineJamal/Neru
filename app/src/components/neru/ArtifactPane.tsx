import { useEffect, useRef, useState } from 'react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { Check, ChevronDown, Copy, Download, FileText, LoaderCircle, X } from 'lucide-react'
import { api } from '../../api'
import { t } from '../../i18n'
import { cn } from '@/lib/utils'
import { closePaper, openPaper } from '@/lib/artifactStore'
import { EXPORTS, download, paperText, type ExportFormat } from '@/lib/paperExport'
import type { Paper, PaperCard } from '../../types'
import './ArtifactPane.css'

const words = (count: number) => count.toLocaleString()

/** The document panel beside the chat, as in the Claude desktop app: the paper typeset on a page, its
 *  versions in this session, copy, and downloads in the format the student asked for or any other. */
export function ArtifactPane({ sessionId, id, onError, onNotice }: { sessionId: string; id: string; onError: (message: string) => void; onNotice: (message: string) => void }) {
  const [paper, setPaper] = useState<Paper | null>(null)
  const [versions, setVersions] = useState<PaperCard[]>([])
  const [menu, setMenu] = useState(false)
  const [saving, setSaving] = useState<ExportFormat | null>(null)
  const [copied, setCopied] = useState(false)
  const menuRoot = useRef<HTMLDivElement>(null)

  useEffect(() => {
    let live = true
    void api.readPaper(sessionId, id).then(next => { if (live) setPaper(next) }).catch(cause => { if (live) { onError(cause instanceof Error ? cause.message : String(cause)); closePaper() } })
    void api.listPapers(sessionId).then(list => { if (live) setVersions(list) }).catch(() => undefined)
    return () => { live = false }
  }, [sessionId, id, onError])
  useEffect(() => {
    if (!menu) return
    const away = (event: MouseEvent) => { if (!menuRoot.current?.contains(event.target as Node)) setMenu(false) }
    document.addEventListener('mousedown', away)
    return () => document.removeEventListener('mousedown', away)
  }, [menu])
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape' && !menu) closePaper() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [menu])

  const save = async (format: ExportFormat) => {
    if (!paper || saving) return
    setMenu(false); setSaving(format)
    try {
      const path = await download(paper, format)
      if (path) onNotice(t('Saved {file}', { file: path.split(/[\\/]/).pop() ?? path }))
    } catch (cause) { onError(t('Could not save the {format} file: {reason}', { format: format.toUpperCase(), reason: cause instanceof Error ? cause.message : String(cause) })) }
    finally { setSaving(null) }
  }
  const copy = async () => {
    if (!paper) return
    await navigator.clipboard.writeText(paperText(paper)).catch(() => undefined)
    setCopied(true); window.setTimeout(() => setCopied(false), 1400)
  }
  const primary: ExportFormat = paper?.format === 'pdf' ? 'pdf' : paper?.format === 'tex' ? 'tex' : paper?.format === 'html' ? 'html' : paper?.format === 'md' ? 'md' : 'docx'
  const primaryLabel = EXPORTS.find(item => item.format === primary)?.label ?? 'Word document'

  return <aside className="artifact-pane" aria-label={t('Document')}>
    <header className="artifact-head">
      <FileText size={15} className="artifact-icon" aria-hidden />
      <div className="artifact-title">
        <strong title={paper?.title}>{paper?.title ?? t('Loading…')}</strong>
        {versions.length > 1
          ? <select className="artifact-version" value={id} onChange={event => openPaper(event.target.value)} aria-label={t('Version')}>
            {versions.map((version, index) => <option key={version.id} value={version.id}>{t('Version {n}', { n: versions.length - index })} · {words(version.words)} {t('words')}</option>)}
          </select>
          : paper && <span>{paper.kind} · {words(paper.words)} {t('words')} · {paper.references.length} {t('references')}</span>}
      </div>
      <button type="button" className="artifact-tool" onClick={() => void copy()} disabled={!paper} title={t('Copy as Markdown')} aria-label={t('Copy as Markdown')}>{copied ? <Check size={15} /> : <Copy size={15} />}</button>
      <div ref={menuRoot} className="artifact-download">
        <button type="button" className="artifact-primary" onClick={() => void save(primary)} disabled={!paper || Boolean(saving)} title={t('Download as {format}', { format: primaryLabel })}>
          {saving === primary ? <LoaderCircle size={14} className="animate-spin" /> : <Download size={14} />}{primary === 'docx' ? 'Word' : primary.toUpperCase()}
        </button>
        <button type="button" className="artifact-more" onClick={() => setMenu(!menu)} disabled={!paper || Boolean(saving)} aria-haspopup="menu" aria-expanded={menu} aria-label={t('Other formats')}><ChevronDown size={14} /></button>
        {menu && <div className="artifact-menu" role="menu">
          {EXPORTS.map(item => <button key={item.format} type="button" role="menuitem" onClick={() => void save(item.format)}>
            <span>{t(item.label)}<small>.{item.extension}</small></span><em>{t(item.hint)}</em>
          </button>)}
        </div>}
      </div>
      <button type="button" className="artifact-tool" onClick={closePaper} title={t('Close (Esc)')} aria-label={t('Close document')}><X size={16} /></button>
    </header>
    <div className="artifact-scroll">
      {!paper
        ? <div className="artifact-loading"><LoaderCircle size={18} className="animate-spin" /></div>
        : <article className="paper-page">
          <p className="paper-kind">{paper.kind}</p>
          <h1 className="paper-title">{paper.title}</h1>
          {(paper.authors.length > 0 || paper.affiliation) && <p className="paper-byline">{[paper.authors.join(', '), paper.affiliation].filter(Boolean).join(' · ')}</p>}
          <p className="paper-date">{paper.date}</p>
          {paper.abstractText && <section className="paper-abstract">
            <h2>{t('Abstract')}</h2>
            <p>{paper.abstractText}</p>
            {paper.keywords.length > 0 && <p className="paper-keywords"><em>{t('Keywords')}:</em> {paper.keywords.join(', ')}</p>}
          </section>}
          {paper.sections.map((section, index) => <section key={index} className="paper-section">
            <h2>{index + 1}. {section.heading}</h2>
            <ReactMarkdown remarkPlugins={[remarkGfm]} components={{ h2: 'h3', a: ({ children }) => <span>{children}</span> }}>{section.body}</ReactMarkdown>
          </section>)}
          {paper.references.length > 0 && <section className={cn('paper-references', paper.style !== 'ieee' && 'hanging')}>
            <h2>{t('References')}</h2>
            {paper.references.map(reference => <div key={reference.number} className="paper-reference"><ReactMarkdown components={{ p: 'span' }}>{reference.text}</ReactMarkdown></div>)}
          </section>}
        </article>}
    </div>
  </aside>
}

/** The chat card for a paper: opens the panel, and downloads the format the student asked for. */
export function DocumentCard({ code }: { code: string }) {
  let card: PaperCard | null = null
  try { card = JSON.parse(code) as PaperCard } catch { card = null }
  if (!card?.id) return <pre>{code}</pre>
  const { id } = card
  return <button type="button" className="document-card" onClick={() => openPaper(id)}>
    <span className="document-card-icon"><FileText size={18} /></span>
    <span className="document-card-text">
      <strong>{card.title}</strong>
      <small>{card.kind} · {words(card.words)} {t('words')} · {card.sources} {t('references')}{card.format ? ` · ${card.format === 'docx' ? 'Word' : card.format.toUpperCase()}` : ''}</small>
    </span>
    <span className="document-card-open">{t('Open')}</span>
  </button>
}
