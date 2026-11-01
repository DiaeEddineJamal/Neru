import { useEffect, useMemo, useState } from 'react'
import { Check, FileText, LoaderCircle, MessageSquarePlus, PenLine, Send, Trash2, X } from 'lucide-react'
import { api } from '../../api'
import { t } from '../../i18n'
import { cn } from '@/lib/utils'
import { SPEC_STEPS, markdownBlocks, reviewMessage, type BlockComment } from '@/lib/review'
import type { SessionDocument } from '../../types'
import { ArtifactEditor } from './team/ArtifactEditor'
import { RESPONSE_PROSE, ResponseMarkdown } from './Conversation'
import './SidePanels.css'

interface Props {
  sessionId: string
  documents: SessionDocument[]
  open: string | null
  onOpen: (name: string) => void
  onClose: () => void
  /** Sends a review message into the conversation. */
  onSend: (message: string) => void
  /** Approves a Spec document; the caller moves on to the next phase. */
  onApprove: (doc: SessionDocument, edited: string | null) => void
  onError: (message: string) => void
  /** A reply is running or an approval waits: sending waits too. */
  busy: boolean
}

/** The documents the agent wrote in this session, beside the conversation, to edit or comment on. */
export function DocumentsPanel(props: Props) {
  const { documents, open, onOpen, onClose } = props
  const doc = documents.find(item => item.name === open) ?? documents[0] ?? null
  return <aside className="side-panel documents-panel" aria-label={t('Documents')}>
    <header className="side-panel-head">
      <FileText size={15} />
      <strong>{t('Documents')}</strong>
      {documents.length > 1 && <div className="doc-tabs" role="tablist">{documents.map(item => <button key={item.name} role="tab" aria-selected={item.name === doc?.name} className={cn('doc-tab', item.name === doc?.name && 'active')} onClick={() => onOpen(item.name)}>
        {item.title}{item.total > 0 && <small>{item.done}/{item.total}</small>}
      </button>)}</div>}
      <button type="button" className="icon-button small" onClick={onClose} title={t('Close')} aria-label={t('Close documents')}><X size={14} /></button>
    </header>
    {doc
      // A new document, or a new version from the agent, starts a fresh review.
      ? <DocumentReview key={`${props.sessionId}:${doc.name}:${doc.modified}`} {...props} doc={doc} />
      : <p className="side-panel-empty">{t('No documents yet. In Spec mode Neru writes requirements, design and tasks here; it can also leave a plan or a walkthrough for you to review.')}</p>}
  </aside>
}

function DocumentReview({ sessionId, doc, onSend, onApprove, onError, busy }: Props & { doc: SessionDocument }) {
  const [text, setText] = useState<string | null>(null)
  const [draft, setDraft] = useState<string | null>(null)
  const [edited, setEdited] = useState<string | null>(null)
  const [comments, setComments] = useState<BlockComment[]>([])
  const [general, setGeneral] = useState('')
  const [adding, setAdding] = useState<number | null>(null)
  const [note, setNote] = useState('')
  const [saving, setSaving] = useState(false)

  useEffect(() => {
    let alive = true
    api.readSessionDocument(sessionId, doc.name).then(value => { if (alive) setText(value) }).catch(cause => onError(String(cause)))
    return () => { alive = false }
  }, [sessionId, doc.name, onError])

  const blocks = useMemo(() => markdownBlocks(text ?? ''), [text])
  const spec = (SPEC_STEPS as readonly string[]).includes(doc.name)
  const reviewCount = comments.length + (general.trim() ? 1 : 0)

  const addComment = () => {
    if (adding === null || !note.trim()) return
    setComments(current => [...current, { id: `${Date.now()}-${current.length}`, block: adding, text: note }])
    setAdding(null); setNote('')
  }
  const saveEdit = async () => {
    if (draft === null) return
    setSaving(true)
    try {
      // The list is not refreshed here: a new version stamp would restart the review and drop the
      // note that the user edited the text.
      await api.writeSessionDocument(sessionId, doc.name, draft)
      setText(draft); setEdited(draft); setDraft(null)
    } catch (cause) { onError(String(cause)) } finally { setSaving(false) }
  }
  const send = () => {
    if (!reviewCount && edited === null) return
    onSend(reviewMessage(doc, blocks, comments, general, edited))
    setComments([]); setGeneral(''); setEdited(null)
  }

  if (text === null) return <p className="side-panel-empty"><LoaderCircle size={14} className="animate-spin" /> {t('Opening…')}</p>
  if (draft !== null) return <div className="doc-editor">
    <ArtifactEditor value={draft} onChange={setDraft} onSave={() => void saveEdit()} onCancel={() => setDraft(null)}
      renderPreview={markdown => <div className={RESPONSE_PROSE}><ResponseMarkdown content={markdown} sources={[]} idPrefix="doc-edit" onCite={() => undefined} /></div>} />
    <div className="doc-actions">
      <button type="button" className="button primary small" disabled={saving} onClick={() => void saveEdit()}>{saving ? <LoaderCircle size={13} className="animate-spin" /> : <Check size={13} />} {t('Save')}</button>
      <button type="button" className="button subtle small" onClick={() => setDraft(null)}>{t('Cancel')}</button>
    </div>
  </div>
  return <>
    <div className="doc-body">
      {blocks.map((block, index) => {
        const own = comments.filter(comment => comment.block === index)
        return <div key={index} className={cn('doc-block', own.length > 0 && 'commented')}>
          <div className={RESPONSE_PROSE}><ResponseMarkdown content={block} sources={[]} idPrefix={`doc-${index}`} onCite={() => undefined} /></div>
          <button type="button" className="doc-comment-add" title={t('Comment on this')} aria-label={t('Comment on this part')} onClick={() => { setAdding(index); setNote('') }}><MessageSquarePlus size={14} /></button>
          {own.map(comment => <div key={comment.id} className="doc-comment">
            <span>{comment.text}</span>
            <button type="button" className="icon-button small" aria-label={t('Remove comment')} onClick={() => setComments(current => current.filter(item => item.id !== comment.id))}><Trash2 size={12} /></button>
          </div>)}
          {adding === index && <div className="doc-comment-form">
            <textarea autoFocus rows={2} value={note} placeholder={t('What should change here?')} onChange={event => setNote(event.target.value)}
              onKeyDown={event => { if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) { event.preventDefault(); addComment() } else if (event.key === 'Escape') { event.stopPropagation(); setAdding(null) } }} />
            <div className="doc-actions"><button type="button" className="button primary small" disabled={!note.trim()} onClick={addComment}>{t('Add comment')}</button><button type="button" className="button subtle small" onClick={() => setAdding(null)}>{t('Cancel')}</button></div>
          </div>}
        </div>
      })}
    </div>
    <footer className="doc-footer">
      <textarea rows={2} value={general} placeholder={t('A comment on the whole document (optional)')} onChange={event => setGeneral(event.target.value)} />
      <div className="doc-actions">
        <button type="button" className="button subtle small" onClick={() => setDraft(text)}><PenLine size={13} /> {t('Edit')}</button>
        <span className="doc-spacer" />
        <button type="button" className="button small" disabled={busy || (!reviewCount && edited === null)} onClick={send}><Send size={13} /> {reviewCount ? t('Send {n} comments', { n: reviewCount }) : t('Send my edits')}</button>
        {spec && <button type="button" className="button primary small" disabled={busy || reviewCount > 0} title={reviewCount ? t('Send or remove your comments first') : undefined} onClick={() => { onApprove(doc, edited); setEdited(null) }}><Check size={13} /> {doc.name === 'tasks' ? t('Approve and build') : t('Approve')}</button>}
      </div>
    </footer>
  </>
}
