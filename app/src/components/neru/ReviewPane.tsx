import { useState } from 'react'
import { FileDiff as FileDiffIcon, LoaderCircle, MessageSquarePlus, RefreshCw, Send, Trash2, X } from 'lucide-react'
import { FileDiff } from '@/components/agents/file-diff'
import { languageForPath } from '@/components/agents/agent-code'
import { parseUnifiedDiff } from '@/lib/diff'
import type { SessionChange } from '../../types'

export interface ReviewComment { id: string; path: string; line: string; text: string }

/** Everything Neru changed in this session in one place, with comments pinned to diff lines. */
export function ReviewPane({ changes, loading, comments, onComments, onRefresh, onClose, onSend, onOpenFile }: {
  changes: SessionChange[]
  loading: boolean
  comments: ReviewComment[]
  onComments: (comments: ReviewComment[]) => void
  onRefresh: () => void
  onClose: () => void
  onSend: (message: string) => void
  onOpenFile: (path: string) => void
}) {
  const [drafts, setDrafts] = useState<Record<string, { line: string; text: string }>>({})
  const additions = changes.reduce((sum, change) => sum + change.additions, 0)
  const deletions = changes.reduce((sum, change) => sum + change.deletions, 0)
  const addComment = (path: string) => {
    const draft = drafts[path]
    if (!draft?.text.trim()) return
    onComments([...comments, { id: crypto.randomUUID(), path, line: draft.line.trim(), text: draft.text.trim() }])
    setDrafts(current => ({ ...current, [path]: { line: '', text: '' } }))
  }
  const send = () => {
    const lines = comments.map(comment => `- ${comment.path}${comment.line ? `:${comment.line}` : ''}: ${comment.text}`)
    onSend(`Please address these review comments on the changes in this session:\n${lines.join('\n')}`)
  }

  return <aside className="review-pane" aria-label="Session changes">
    <header className="review-head">
      <div><strong>Changes in this session</strong><span>{changes.length} file{changes.length === 1 ? '' : 's'} · <b className="add">+{additions}</b> <b className="del">−{deletions}</b></span></div>
      <button className="icon-button small" onClick={onRefresh} aria-label="Refresh changes" title="Refresh">{loading ? <LoaderCircle size={14} className="animate-spin" /> : <RefreshCw size={14} />}</button>
      <button className="icon-button small" onClick={onClose} aria-label="Close changes" title="Close"><X size={14} /></button>
    </header>
    <div className="review-body">
      {!loading && changes.length === 0 && <div className="empty-pane"><FileDiffIcon size={24} /><p>Neru has not changed any files in this session yet.</p></div>}
      {changes.map(change => {
        const draft = drafts[change.path] ?? { line: '', text: '' }
        const fileComments = comments.filter(comment => comment.path === change.path)
        return <section key={change.path} className="review-file">
          <FileDiff file={change.path} lines={parseUnifiedDiff(change.diff)} status="complete" collapseOnComplete={false} defaultOpen maxHeight={420} language={languageForPath(change.path)} copyText={change.diff} onLineClick={line => setDrafts(current => ({ ...current, [change.path]: { ...(current[change.path] ?? { line: '', text: '' }), line: String(line) } }))} comments={fileComments.map(comment => ({ line: Number(comment.line), text: comment.text }))} />
          <div className="review-file-actions"><span className={`review-status ${change.status}`}>{change.status}</span><button className="text-action" onClick={() => onOpenFile(change.path)}>Open in editor</button></div>
          {fileComments.map(comment => <div key={comment.id} className="review-comment"><span>{comment.line ? `Line ${comment.line}` : 'File'}</span><p>{comment.text}</p><button className="icon-button small" onClick={() => onComments(comments.filter(item => item.id !== comment.id))} aria-label="Delete comment"><Trash2 size={12} /></button></div>)}
          <div className="review-compose">
            <input className="review-line" value={draft.line} onChange={event => setDrafts(current => ({ ...current, [change.path]: { ...draft, line: event.target.value.replace(/[^\d-]/g, '') } }))} placeholder="Line" aria-label={`Line in ${change.path}`} />
            <input value={draft.text} onChange={event => setDrafts(current => ({ ...current, [change.path]: { ...draft, text: event.target.value } }))} onKeyDown={event => { if (event.key === 'Enter') addComment(change.path) }} placeholder="Add a review comment…" aria-label={`Comment on ${change.path}`} />
            <button className="icon-button small" onClick={() => addComment(change.path)} disabled={!draft.text.trim()} aria-label="Add comment" title="Add comment"><MessageSquarePlus size={14} /></button>
          </div>
        </section>
      })}
    </div>
    {comments.length > 0 && <footer className="review-foot"><button className="button primary" onClick={send}><Send size={14} /> Send {comments.length} comment{comments.length === 1 ? '' : 's'} to Neru</button></footer>}
  </aside>
}
