import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { Check, ChevronDown, Copy, Download, FileText, LoaderCircle, MessageCircle, RotateCcw, X } from '@/components/icons'
import { api } from '../../api'
import { t } from '../../i18n'
import { cn } from '@/lib/utils'
import { closePaper, openPaper, useLiveDraft, useSaves, type LiveDraft } from '@/lib/artifactStore'
import { Quill } from './Quill'
import { EXPORTS, download, paperText, type ExportFormat } from '@/lib/paperExport'
import type { Paper, PaperCard } from '../../types'
import './ArtifactPane.css'
import { Select } from '../ui/Select'

const words = (count: number) => count.toLocaleString()
/** Papers in right-to-left languages are set right to left, and the writing shimmer runs that way too. */
const rtl = (language: string) => /arab|عرب|hebrew|עבר|persian|farsi|فارس|urdu|اردو|darija|الدارجة/i.test(language)
const message = (cause: unknown) => cause instanceof Error ? cause.message : String(cause)
/** A section's Markdown in blocks (paragraphs, headings, lists, tables), split at blank lines as paper.rs does. */
const splitBlocks = (body: string) => body.replace(/\r\n/g, '\n').split(/\n\s*\n/).map(block => block.trim()).filter(Boolean)
const link = ({ children }: { children?: ReactNode }) => <span>{children}</span>

/** One block of the page, edited where it stands: click it and type, in the page's own type; it saves
 *  when you click away (or Ctrl+Enter) and Esc puts it back. An emptied block is removed. */
function Block({ text, locked, single, components, onSave }: { text: string; locked: boolean; single?: boolean; components: object; onSave: (next: string) => Promise<void> }) {
  const [draft, setDraft] = useState<string | null>(null)
  const box = useRef<HTMLTextAreaElement>(null)
  const done = useRef(false)
  useLayoutEffect(() => {
    const area = box.current
    if (!area) return
    area.style.height = 'auto'
    area.style.height = `${area.scrollHeight}px`
  }, [draft])
  const open = (event: React.MouseEvent<HTMLDivElement>) => {
    // A click that ends a selection is for "Ask Neru", not for editing.
    if (locked || draft !== null || window.getSelection()?.toString().trim()) return
    // The caret lands about where the click was: at the same share of the text.
    const whole = event.currentTarget.textContent ?? ''
    const caret = document.caretRangeFromPoint?.(event.clientX, event.clientY)
    const at = caret ? whole.indexOf(caret.startContainer.textContent ?? '') + caret.startOffset : -1
    const share = at >= 0 && whole.length ? Math.min(1, at / whole.length) : 1
    done.current = false
    setDraft(text)
    window.setTimeout(() => { const area = box.current; if (!area) return; const index = Math.round(area.value.length * share); area.focus(); area.setSelectionRange(index, index) }, 0)
  }
  const finish = (keep: boolean) => {
    if (draft === null || done.current) return
    done.current = true
    const next = draft.trim()
    setDraft(null)
    if (keep && next !== text.trim()) void onSave(next).catch(() => undefined)
  }
  if (draft !== null) return <textarea ref={box} className={cn('paper-block-input', single && 'is-single')} value={draft} spellCheck aria-label={t('Edit paragraph')} rows={1}
    onChange={event => setDraft(single ? event.target.value.replace(/\n/g, ' ') : event.target.value)} onBlur={() => finish(true)}
    onKeyDown={event => {
      if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); finish(false) }
      if (event.key === 'Enter' && (event.ctrlKey || event.metaKey || single)) { event.preventDefault(); finish(true) }
    }} />
  return <div className={cn('paper-block', !locked && 'is-editable')} onClick={open} title={locked ? undefined : t('Click to edit')}>
    {single ? text : <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>{text}</ReactMarkdown>}
  </div>
}

/** A part made of blocks, each editable on its own. Saving one puts the part back together. */
function Blocks({ body, locked, components, onSave }: { body: string; locked: boolean; components: object; onSave: (next: string) => Promise<void> }) {
  const parts = splitBlocks(body)
  return <>{parts.map((block, index) => <Block key={`${index}:${block.slice(0, 24)}`} text={block} locked={locked} components={components}
    onSave={next => onSave(parts.map((part, at) => at === index ? next : part).filter(part => part.trim()).join('\n\n'))} />)}</>
}

/** The part the model is writing right now: what it has written so far, with the quill and the shimmer on
 *  it, between the text before and after it when it rewrites a passage in place. */
function Writing({ draft, components }: { draft: LiveDraft; components: object }) {
  const show = (text: string) => <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>{text}</ReactMarkdown>
  return <>
    {draft.before?.trim() && show(draft.before)}
    <div className="is-writing-part">{show(draft.text)}</div>
    {draft.after?.trim() && show(draft.after)}
  </>
}

/** The document panel beside the chat, as in the Claude desktop app: the paper or document typeset on a
 *  page, written live with a quill at the point the model has reached, editable paragraph by paragraph,
 *  with its versions, copy, and downloads. */
export function ArtifactPane({ sessionId, id, onError, onNotice, onAsk }: { sessionId: string; id: string; onError: (message: string) => void; onNotice: (message: string) => void; onAsk?: (quote: string) => void }) {
  const [paper, setPaper] = useState<Paper | null>(null)
  const [versions, setVersions] = useState<PaperCard[]>([])
  const [menu, setMenu] = useState(false)
  const [saving, setSaving] = useState<ExportFormat | null>(null)
  const [copied, setCopied] = useState(false)
  const [saved, setSaved] = useState(false)
  const [pen, setPen] = useState<{ left: number; top: number } | null>(null)
  const [selection, setSelection] = useState<{ text: string; left: number; top: number } | null>(null)
  const menuRoot = useRef<HTMLDivElement>(null)
  const scroller = useRef<HTMLDivElement>(null)
  const page = useRef<HTMLElement>(null)
  // Follows the writing down the page until the reader scrolls away from it, as Claude does.
  const follow = useRef(true)
  const draft = useLiveDraft()
  const saves = useSaves()
  const streaming = draft?.id === id ? draft : null

  useEffect(() => {
    let live = true
    void api.readPaper(sessionId, id).then(next => { if (live) setPaper(next) }).catch(cause => { if (live) { onError(message(cause)); closePaper() } })
    void api.listPapers(sessionId).then(list => { if (live) setVersions(list) }).catch(() => undefined)
    return () => { live = false }
  }, [sessionId, id, onError, saves])
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

  // The quill sits at the end of the text being written: the last character of the writing block.
  useLayoutEffect(() => {
    const root = page.current
    const block = root?.querySelector('.is-writing-part, .paper-title.is-writing')
    if (!root || !block || !streaming) { setPen(null); return }
    const walker = document.createTreeWalker(block, NodeFilter.SHOW_TEXT, { acceptNode: node => node.textContent?.trim() ? NodeFilter.FILTER_ACCEPT : NodeFilter.FILTER_REJECT })
    let last: Node | null = null
    while (walker.nextNode()) last = walker.currentNode
    const range = document.createRange()
    if (last) { range.setStart(last, last.textContent?.length ?? 0); range.collapse(true) } else range.selectNodeContents(block)
    const rects = typeof range.getClientRects === 'function' ? range.getClientRects() : []
    const at = rects.length ? rects[rects.length - 1] : block.getBoundingClientRect()
    const box = root.getBoundingClientRect()
    const right = root.getAttribute('dir') === 'rtl'
    setPen({ left: (right ? at.left - 20 : at.right + 3) - box.left, top: at.top - box.top + at.height / 2 - 11 })
    const view = scroller.current
    if (view && follow.current) {
      const target = at.top - view.getBoundingClientRect().top
      if (target > view.clientHeight - 96) view.scrollTop += target - view.clientHeight + 140
      else if (target < 24) view.scrollTop += target - 120
    }
  }, [streaming, paper])
  const onScroll = () => {
    const view = scroller.current, mark = page.current?.querySelector('.paper-pen')
    setSelection(null)
    if (!view || !mark) return
    const at = mark.getBoundingClientRect().top - view.getBoundingClientRect().top
    follow.current = at > -40 && at < view.clientHeight + 40
  }
  // Selected text, on the page or in a paragraph being edited, gets an "Ask Neru" button that quotes it.
  const onMouseUp = () => {
    if (!onAsk) return
    window.setTimeout(() => {
      const view = scroller.current
      if (!view) return
      const frame = view.getBoundingClientRect()
      const place = (rect: DOMRect) => ({ left: Math.min(Math.max(8, rect.left - frame.left + rect.width / 2 - 60), frame.width - 140), top: rect.top - frame.top + view.scrollTop - 40 })
      const area = document.activeElement instanceof HTMLTextAreaElement && page.current?.contains(document.activeElement) ? document.activeElement : null
      if (area && area.selectionEnd > area.selectionStart) {
        setSelection({ text: area.value.slice(area.selectionStart, area.selectionEnd).trim(), ...place(area.getBoundingClientRect()) })
        return
      }
      const picked = window.getSelection()
      const text = picked?.toString().trim() ?? ''
      if (!text || !picked?.rangeCount || !page.current?.contains(picked.anchorNode)) { setSelection(null); return }
      setSelection({ text, ...place(picked.getRangeAt(0).getBoundingClientRect()) })
    }, 0)
  }

  const save = async (format: ExportFormat) => {
    if (!paper || saving) return
    setMenu(false); setSaving(format)
    try {
      const path = await download(paper, format)
      if (path) onNotice(t('Saved {file}', { file: path.split(/[\\/]/).pop() ?? path }))
    } catch (cause) { onError(t('Could not save the {format} file: {reason}', { format: format.toUpperCase(), reason: message(cause) })) }
    finally { setSaving(null) }
  }
  const copy = async () => {
    if (!paper) return
    await navigator.clipboard.writeText(paperText(paper)).catch(() => undefined)
    setCopied(true); window.setTimeout(() => setCopied(false), 1400)
  }
  // An edit shows at once and is saved in the background; if saving fails the page goes back.
  const edit = (part: string) => async (text: string) => {
    if (!paper) return
    const before = paper
    setPaper(part === 'title' ? { ...paper, title: text } : part === 'abstract' ? { ...paper, abstractText: text } : { ...paper, sections: paper.sections.map((section, index) => String(index) === part ? { ...section, body: text } : section) })
    try {
      setPaper(await api.updatePaperPart(sessionId, id, part, text))
      setSaved(true); window.setTimeout(() => setSaved(false), 1600)
    } catch (cause) { setPaper(before); onError(message(cause)); throw cause }
  }
  const primary: ExportFormat = paper?.format === 'pdf' ? 'pdf' : paper?.format === 'tex' ? 'tex' : paper?.format === 'html' ? 'html' : paper?.format === 'md' ? 'md' : 'docx'
  const primaryLabel = EXPORTS.find(item => item.format === primary)?.label ?? 'Word document'
  const plainDoc = Boolean(paper?.plain)
  const exports = plainDoc ? EXPORTS.filter(item => item.format !== 'bib' && item.format !== 'tex') : EXPORTS
  // The model's part is the one it is writing; everything already written is the user's to edit too.
  const locked = (index: number) => Boolean(streaming && streaming.index === index) || !paper || index >= paper.sections.length
  const markdown = plainDoc ? { a: link } : { h2: 'h3' as const, a: link }

  return <aside className="artifact-pane" aria-label={t('Document')}>
    <header className="artifact-head">
      <FileText size={15} className="artifact-icon" aria-hidden />
      <div className="artifact-title">
        <strong title={paper?.title}>{paper?.title ?? t('Loading…')}</strong>
        {versions.length > 1
          ? <Select className="artifact-version" value={id} onChange={event => openPaper(event.target.value)} aria-label={t('Version')}>
            {versions.map((version, index) => <option key={version.id} value={version.id}>{t('Version {n}', { n: versions.length - index })} · {words(version.words)} {t('words')}</option>)}
          </Select>
          : paper && <span>{paper.kind} · {words(paper.words)} {t('words')}{paper.status === 'done' ? plainDoc ? '' : ` · ${paper.references.length} ${t('references')}` : paper.status === 'paused' ? ` · ${t('paused')}` : ` · ${t('writing…')}`}</span>}
      </div>
      {saved && <span className="artifact-saved" role="status"><Check size={13} />{t('Saved')}</span>}
      <button type="button" className="artifact-tool" onClick={() => void copy()} disabled={!paper} title={t('Copy as Markdown')} aria-label={t('Copy as Markdown')}>{copied ? <Check size={15} /> : <Copy size={15} />}</button>
      <div ref={menuRoot} className="artifact-download">
        <button type="button" className="artifact-primary" onClick={() => void save(primary)} disabled={!paper || Boolean(saving)} title={t('Download as {format}', { format: primaryLabel })}>
          {saving === primary ? <LoaderCircle size={14} className="animate-spin" /> : <Download size={14} />}{primary === 'docx' ? 'Word' : primary.toUpperCase()}
        </button>
        <button type="button" className="artifact-more" onClick={() => setMenu(!menu)} disabled={!paper || Boolean(saving)} aria-haspopup="menu" aria-expanded={menu} aria-label={t('Other formats')}><ChevronDown size={14} /></button>
        {menu && <div className="artifact-menu" role="menu">
          {exports.map(item => <button key={item.format} type="button" role="menuitem" onClick={() => void save(item.format)}>
            <span>{t(item.label)}<small>.{item.extension}</small></span><em>{t(item.hint)}</em>
          </button>)}
        </div>}
      </div>
      <button type="button" className="artifact-tool" onClick={closePaper} title={t('Close (Esc)')} aria-label={t('Close document')}><X size={16} /></button>
    </header>
    <div ref={scroller} className="artifact-scroll" onScroll={onScroll} onMouseUp={onMouseUp}>
      {!paper
        ? <div className="artifact-loading"><LoaderCircle size={18} className="animate-spin" /></div>
        : <>
        {paper.status === 'writing' && <div className="paper-status"><Quill /><span>{streaming ? (streaming.before || streaming.after ? t('Editing “{heading}”…', { heading: streaming.heading || paper.title }) : streaming.heading ? t('Writing “{heading}”…', { heading: streaming.heading }) : t('Writing…')) : plainDoc ? t('Starting…') : t('Writing section {n} of {total}…', { n: Math.min(paper.sections.length + 1, paper.outline.length || 1), total: paper.outline.length || 1 })}</span></div>}
        {paper.status === 'paused' && <div className="paper-status paused"><RotateCcw size={14} /><span>{t('Paused after {n} of {total} sections. Pick another model if you like, then press Retry or say “continue”.', { n: paper.sections.length, total: paper.outline.length })}</span></div>}
        <article ref={page} className={cn('paper-page', plainDoc && 'is-document')} dir={rtl(paper.language) ? 'rtl' : undefined}>
          {!plainDoc && <p className="paper-kind">{paper.kind}</p>}
          {streaming?.heading === 'Title'
            ? <h1 className="paper-title is-writing">{streaming.text}</h1>
            : <h1 className="paper-title"><Block text={paper.title} single locked={paper.status === 'writing' && !plainDoc && !paper.sections.length} components={markdown} onSave={edit('title')} /></h1>}
          {!plainDoc && (paper.authors.length > 0 || paper.affiliation) && <p className="paper-byline">{[paper.authors.join(', '), paper.affiliation].filter(Boolean).join(' · ')}</p>}
          {!plainDoc && <p className="paper-date">{paper.date}</p>}
          {(paper.abstractText || streaming?.heading === 'Abstract') && <section className={cn('paper-abstract', streaming?.heading === 'Abstract' && 'is-writing')}>
            <h2>{t('Abstract')}</h2>
            {streaming?.heading === 'Abstract'
              ? <Writing draft={streaming} components={markdown} />
              : <Blocks body={paper.abstractText} locked={false} components={markdown} onSave={edit('abstract')} />}
            {paper.keywords.length > 0 && <p className="paper-keywords"><em>{t('Keywords')}:</em> {paper.keywords.join(', ')}</p>}
          </section>}
          {(paper.outline.length ? paper.outline : paper.sections).map((planned, index) => {
            const section = paper.sections[index]
            const writingHere = streaming && streaming.index === index
            const title = planned.heading.trim() ? (plainDoc ? planned.heading : `${index + 1}. ${planned.heading}`) : ''
            if (!section && !writingHere) return title ? <section key={index} className="paper-section planned" aria-label={t('Not written yet')}><h2>{title}</h2></section> : null
            return <section key={index} className={cn('paper-section', writingHere && 'is-writing')}>
              {title && <h2>{title}</h2>}
              {writingHere
                ? <Writing draft={streaming} components={markdown} />
                : <Blocks body={section.body} locked={locked(index)} components={markdown} onSave={edit(String(index))} />}
            </section>
          })}
          {paper.references.length > 0 && <section className={cn('paper-references', paper.style !== 'ieee' && 'hanging')}>
            <h2>{t('References')}</h2>
            {paper.references.map(reference => <div key={reference.number} className="paper-reference"><ReactMarkdown components={{ p: 'span' }}>{reference.text}</ReactMarkdown></div>)}
          </section>}
          {pen && <span className="paper-pen" style={{ left: pen.left, top: pen.top }} aria-hidden><Quill size={20} /></span>}
        </article>
        {selection && <button type="button" className="paper-ask" style={{ left: selection.left, top: selection.top }} onMouseDown={event => event.preventDefault()}
          onClick={() => { onAsk?.(selection.text); setSelection(null); window.getSelection()?.removeAllRanges() }}><MessageCircle size={13} />{t('Ask Neru')}</button>}
        </>}
    </div>
  </aside>
}

/** The chat card for a paper or document: opens the panel. */
export function DocumentCard({ code }: { code: string }) {
  let card: PaperCard | null = null
  try { card = JSON.parse(code) as PaperCard } catch { card = null }
  if (!card?.id) return <pre>{code}</pre>
  const { id } = card
  return <button type="button" className="document-card" onClick={() => openPaper(id)}>
    <span className="document-card-icon"><FileText size={18} /></span>
    <span className="document-card-text">
      <strong>{card.title}</strong>
      <small>{card.kind} · {words(card.words)} {t('words')}{card.sources ? ` · ${card.sources} ${t('references')}` : ''}{card.format ? ` · ${card.format === 'docx' ? 'Word' : card.format.toUpperCase()}` : ''}</small>
    </span>
    <span className="document-card-open">{t('Open')}</span>
  </button>
}
