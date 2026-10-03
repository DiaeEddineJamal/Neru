import { useRef, useState } from 'react'
import { FileDiff as FileDiffIcon, LoaderCircle, MessageSquarePlus, Pencil, RefreshCw, Save, Send, Trash2, X } from 'lucide-react'
import { FileDiff } from '@/components/agents/file-diff'
import { api } from '../../api'
import { languageForPath } from '@/components/agents/agent-code'
import { parseUnifiedDiff } from '@/lib/diff'
import type { SessionChange } from '../../types'

type DiffView = 'unified' | 'split'
const VIEW_KEY = 'neru.diff.view'
function storedView(): DiffView {
  try { return localStorage.getItem(VIEW_KEY) === 'split' ? 'split' : 'unified' } catch { return 'unified' }
}

/** The file's current text in a plain monospace editor with a line-number gutter. */
function FileEditor({ path, value, onChange }: { path: string; value: string; onChange: (value: string) => void }) {
  const gutter = useRef<HTMLDivElement>(null)
  const count = value.split('\n').length
  return <div className="flex max-h-[420px] min-h-[160px] overflow-hidden rounded-xl border border-[var(--border)] bg-[var(--surface-2)] font-mono text-xs leading-5">
    <div ref={gutter} aria-hidden="true" className="shrink-0 select-none overflow-hidden py-2 pl-2 pr-2 text-right tabular-nums text-[var(--muted)]">
      {Array.from({ length: count }, (_, i) => <div key={i}>{i + 1}</div>)}
    </div>
    <textarea
      value={value}
      onChange={event => onChange(event.target.value)}
      onScroll={event => { if (gutter.current) gutter.current.scrollTop = event.currentTarget.scrollTop }}
      onKeyDown={event => {
        if (event.key !== 'Tab') return
        event.preventDefault()
        const area = event.currentTarget
        const { selectionStart: start, selectionEnd: end } = area
        onChange(value.slice(0, start) + '  ' + value.slice(end))
        requestAnimationFrame(() => area.setSelectionRange(start + 2, start + 2))
      }}
      wrap="off"
      spellCheck={false}
      aria-label={`Edit ${path}`}
      className="min-w-0 flex-1 resize-none bg-transparent py-2 pr-2 text-[var(--text)] outline-none"
    />
  </div>
}

export interface ReviewComment { id: string; path: string; line: string; text: string }

/** Everything Neru changed in this session in one place, with comments pinned to diff lines. */
export function ReviewPane({ changes, loading, comments, onComments, onRefresh, onClose, onSend, onOpenFile, embedded = false }: {
  /** Shown inside the dock, which has its own title and close button. */
  embedded?: boolean
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
  const [view, setView] = useState<DiffView>(storedView)
  const [editing, setEditing] = useState<{ path: string; text: string; saving: boolean; error?: string } | null>(null)
  const [openError, setOpenError] = useState<{ path: string; message: string } | null>(null)
  const chooseView = (next: DiffView) => {
    setView(next)
    try { localStorage.setItem(VIEW_KEY, next) } catch { /* storage blocked; the choice lasts this session */ }
  }
  const startEdit = async (path: string) => {
    setOpenError(null)
    try { setEditing({ path, text: await api.readFile(path), saving: false }) }
    catch (error) { setOpenError({ path, message: `Could not open this file: ${String(error)}` }) }
  }
  const saveEdit = async () => {
    if (!editing) return
    setEditing({ ...editing, saving: true, error: undefined })
    try {
      await api.saveFile(editing.path, editing.text)
      setEditing(null)
      onRefresh()
    } catch (error) {
      setEditing(current => current && { ...current, saving: false, error: String(error) })
    }
  }
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
      <span role="group" aria-label="Diff layout" className="flex items-center gap-2 pr-1">
        {(['unified', 'split'] as const).map(option => <button key={option} className={`text-action${view === option ? ' on' : ''}`} aria-pressed={view === option} onClick={() => chooseView(option)}>{option === 'unified' ? 'Unified' : 'Split'}</button>)}
      </span>
      <button className="icon-button small" onClick={onRefresh} aria-label="Refresh changes" title="Refresh">{loading ? <LoaderCircle size={14} className="animate-spin" /> : <RefreshCw size={14} />}</button>
      {!embedded && <button className="icon-button small" onClick={onClose} aria-label="Close changes" title="Close"><X size={14} /></button>}
    </header>
    <div className="review-body">
      {!loading && changes.length === 0 && <div className="empty-pane"><FileDiffIcon size={24} /><p>Neru has not changed any files in this session yet.</p></div>}
      {changes.map(change => {
        const draft = drafts[change.path] ?? { line: '', text: '' }
        const fileComments = comments.filter(comment => comment.path === change.path)
        return <section key={change.path} className="review-file">
          {editing?.path === change.path
            ? <>
              <div className="flex min-h-9 items-center gap-2 font-mono text-xs text-[var(--text)]"><Pencil size={14} className="text-[var(--muted)]" /><span className="min-w-0 flex-1 truncate">{change.path}</span></div>
              {editing.error && <p role="alert" className="text-xs text-[var(--danger)]">{editing.error}</p>}
              <FileEditor path={change.path} value={editing.text} onChange={text => setEditing(current => current && { ...current, text })} />
            </>
            : <FileDiff view={view} file={change.path} lines={parseUnifiedDiff(change.diff)} status="complete" collapseOnComplete={false} defaultOpen maxHeight={420} language={languageForPath(change.path)} copyText={change.diff} onLineClick={line => setDrafts(current => ({ ...current, [change.path]: { ...(current[change.path] ?? { line: '', text: '' }), line: String(line) } }))} comments={fileComments.map(comment => ({ line: Number(comment.line), text: comment.text }))} />}
          <div className="review-file-actions"><span className={`review-status ${change.status}`}>{change.status}</span><span className="flex items-center gap-3">
            {editing?.path === change.path
              ? <>
                <button className="text-action" onClick={() => setEditing(null)} disabled={editing.saving}>Cancel</button>
                <button className="text-action on" onClick={saveEdit} disabled={editing.saving}>{editing.saving ? <LoaderCircle size={13} className="animate-spin" /> : <Save size={13} />} Save</button>
              </>
              : <>
                {change.status !== 'deleted' && <button className="text-action" onClick={() => startEdit(change.path)} disabled={Boolean(editing)} title={editing ? 'Finish the other edit first' : 'Edit this file here'}><Pencil size={13} /> Edit</button>}
                <button className="text-action" onClick={() => onOpenFile(change.path)}>Open in editor</button>
              </>}
          </span></div>
          {openError?.path === change.path && <p role="alert" className="text-xs text-[var(--danger)]">{openError.message}</p>}
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
