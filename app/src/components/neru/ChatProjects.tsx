import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { open as openDialog } from '@tauri-apps/plugin-dialog'
import { isTauri } from '@tauri-apps/api/core'
import { Archive, ArrowLeft, ArrowUp, FileText, Folders, LoaderCircle, MessageSquare, MoreHorizontal, Pencil, Plus, Search, Star, Trash2, Type, Upload, X, FolderMinus, Eye } from '@/components/icons'
import { api } from '../../api'
import { t } from '../../i18n'
import { toast } from '@/lib/toast'
import { extractDocumentText, isReadableDocument } from '@/lib/documents'
import { cn } from '@/lib/utils'
import { useContextMenu } from './ContextMenu'
import type { ChatProject, ChatProjectDetail, ChatProjectListItem, KnowledgeItem, SessionSummary } from '../../types'
import { capacity, lastActivity, projectChats, relativeTime, sortProjects, type ProjectSort, type ProjectsView } from '@/lib/chatProjects'
import './ChatProjects.css'
import { Select } from '../ui/Select'

export interface ChatProjectsProps {
  view: ProjectsView
  onView: (view: ProjectsView) => void
  /** Every session; a project's chats are the loose chats filed under it. */
  sessions: SessionSummary[]
  busy: boolean
  /** Starts a new chat inside the project and sends the first message. */
  onStartChat: (projectId: string, text: string) => void
  onOpenChat: (id: string) => void
  onRenameChat: (id: string, title: string) => void
  onDeleteChat: (id: string) => void
  onMoveChat: (id: string, projectId: string | null) => void
  /** Projects were created, renamed, starred, archived or deleted (the sidebar lists starred ones). */
  onChanged: () => void
}

const errorText = (cause: unknown) => cause instanceof Error ? cause.message : String(cause)

export function ChatProjects(props: ChatProjectsProps) {
  if (props.view.kind === 'new') return <NewProject onCancel={() => props.onView({ kind: 'list' })} onCreated={project => { props.onChanged(); props.onView({ kind: 'project', id: project.id }) }} />
  if (props.view.kind === 'project') return <ProjectPage key={props.view.id} id={props.view.id} {...props} />
  return <ProjectList {...props} />
}

// ---------- list ----------

function ProjectList({ sessions, onView }: ChatProjectsProps) {
  const [projects, setProjects] = useState<ChatProjectListItem[] | null>(null)
  const [query, setQuery] = useState('')
  const [sort, setSort] = useState<ProjectSort>('recent')
  const [archived, setArchived] = useState(false)
  useEffect(() => { api.listChatProjects().then(setProjects).catch(cause => { setProjects([]); if (isTauri()) toast.error(cause) }) }, [])
  const shown = useMemo(() => {
    const needle = query.trim().toLowerCase()
    const list = (projects ?? []).filter(project => project.archived === archived && (!needle || `${project.name} ${project.description}`.toLowerCase().includes(needle)))
    return sortProjects(list, sort, sessions)
  }, [projects, query, sort, archived, sessions])
  const archivedCount = (projects ?? []).filter(project => project.archived).length
  return <main className="content-view cp-view">
    <div className="cp-column">
      <header className="cp-list-head">
        <h1>{t('Projects')}</h1>
        <button type="button" className="button primary" onClick={() => onView({ kind: 'new' })}><Plus size={15} />{t('New project')}</button>
      </header>
      <div className="cp-toolbar">
        <label className="cp-search"><Search size={15} aria-hidden /><input value={query} onChange={event => setQuery(event.target.value)} placeholder={t('Search projects…')} aria-label={t('Search projects')} /></label>
        <label className="cp-sort"><span>{t('Sort by')}</span>
          <Select value={sort} onChange={event => setSort(event.target.value as ProjectSort)} aria-label={t('Sort projects')}>
            <option value="recent">{t('Recent activity')}</option>
            <option value="name">{t('Name')}</option>
            <option value="created">{t('Date created')}</option>
          </Select>
        </label>
      </div>
      {(archivedCount > 0 || archived) && <div className="cp-tabs" role="tablist">
        <button type="button" role="tab" aria-selected={!archived} className={cn(!archived && 'active')} onClick={() => setArchived(false)}>{t('Active')}</button>
        <button type="button" role="tab" aria-selected={archived} className={cn(archived && 'active')} onClick={() => setArchived(true)}>{t('Archived ({n})', { n: archivedCount })}</button>
      </div>}
      {projects === null
        ? <p className="cp-empty"><LoaderCircle size={16} className="animate-spin" /> {t('Loading projects…')}</p>
        : shown.length === 0
          ? <div className="cp-empty-card">
            <Folders size={28} aria-hidden />
            <strong>{query ? t('No projects match “{query}”.', { query }) : archived ? t('No archived projects.') : t('No projects yet')}</strong>
            {!query && !archived && <p>{t('A project keeps related chats together, with instructions and knowledge every chat in it can use.')}</p>}
            {!query && !archived && <button type="button" className="button subtle" onClick={() => onView({ kind: 'new' })}><Plus size={15} />{t('Create a project')}</button>}
          </div>
          : <div className="cp-grid" role="list">
            {shown.map(project => {
              const chats = projectChats(sessions, project.id).length
              return <button type="button" role="listitem" className="cp-card" key={project.id} onClick={() => onView({ kind: 'project', id: project.id })}>
                <span className="cp-card-title"><strong className="truncate">{project.name}</strong>{project.starred && <Star size={14} weight="fill" className="cp-star-on" aria-label={t('Starred')} />}</span>
                <span className="cp-card-desc">{project.description || <em>{t('No description')}</em>}</span>
                <span className="cp-card-meta">{t('Updated {when}', { when: relativeTime(lastActivity(project, sessions)) })} · {chats === 1 ? t('1 chat') : t('{n} chats', { n: chats })}</span>
              </button>
            })}
          </div>}
    </div>
  </main>
}

// ---------- new project ----------

function NewProject({ onCancel, onCreated }: { onCancel: () => void; onCreated: (project: ChatProject) => void }) {
  const [name, setName] = useState('')
  const [description, setDescription] = useState('')
  const [saving, setSaving] = useState(false)
  const create = async () => {
    if (!name.trim() || saving) return
    setSaving(true)
    try { const project = await api.createChatProject(name.trim(), description.trim()); toast.success(t('Created {name}.', { name: project.name })); onCreated(project) }
    catch (cause) { toast.error(cause) } finally { setSaving(false) }
  }
  return <main className="content-view cp-view">
    <form className="cp-new" onSubmit={event => { event.preventDefault(); void create() }}>
      <h1>{t('Create a personal project')}</h1>
      <label className="cp-field"><span>{t('What are you working on?')}</span>
        <input autoFocus value={name} maxLength={120} onChange={event => setName(event.target.value)} placeholder={t('Name your project')} aria-label={t('What are you working on?')} />
      </label>
      <label className="cp-field"><span>{t('What are you trying to achieve?')}</span>
        <textarea value={description} maxLength={2000} rows={4} onChange={event => setDescription(event.target.value)} placeholder={t('Describe your project, goals, subject, etc…')} aria-label={t('What are you trying to achieve?')} />
      </label>
      <div className="cp-actions">
        <button type="button" className="button subtle" onClick={onCancel}>{t('Cancel')}</button>
        <button type="submit" className="button primary" disabled={!name.trim() || saving}>{saving ? t('Creating…') : t('Create project')}</button>
      </div>
    </form>
  </main>
}

// ---------- dialog ----------

function Dialog({ title, onClose, children, wide }: { title: string; onClose: () => void; children: ReactNode; wide?: boolean }) {
  useEffect(() => {
    const escape = (event: KeyboardEvent) => { if (event.key === 'Escape') onClose() }
    window.addEventListener('keydown', escape)
    return () => window.removeEventListener('keydown', escape)
  }, [onClose])
  return <div className="cp-backdrop" onMouseDown={event => { if (event.target === event.currentTarget) onClose() }}>
    <div className={cn('cp-dialog', wide && 'wide')} role="dialog" aria-modal="true" aria-label={title}>
      <header><h2>{title}</h2><button type="button" className="icon-button" onClick={onClose} aria-label={t('Close')}><X size={16} /></button></header>
      {children}
    </div>
  </div>
}

// ---------- project page ----------

type Modal = { kind: 'instructions' } | { kind: 'text' } | { kind: 'view'; item: KnowledgeItem; text: string | null } | null

function ProjectPage(props: ChatProjectsProps & { id: string }) {
  const { id, sessions, busy, onView } = props
  const [detail, setDetail] = useState<ChatProjectDetail | null>(null)
  const [missing, setMissing] = useState(false)
  const [editing, setEditing] = useState(false)
  const [draftName, setDraftName] = useState('')
  const [draftDescription, setDraftDescription] = useState('')
  const [message, setMessage] = useState('')
  const [modal, setModal] = useState<Modal>(null)
  const [reading, setReading] = useState(0)
  const [renaming, setRenaming] = useState<string | null>(null)
  const [renameValue, setRenameValue] = useState('')
  const openMenu = useContextMenu()
  const composer = useRef<HTMLTextAreaElement>(null)

  const load = useCallback(() => api.getChatProject(id).then(next => { setDetail(next); setMissing(false) }).catch(() => setMissing(true)), [id])
  useEffect(() => { void load() }, [load])

  const project = detail?.project
  const chats = projectChats(sessions, id)
  const update = async (patch: Parameters<typeof api.updateChatProject>[1], done?: string) => {
    try { const next = await api.updateChatProject(id, patch); setDetail(current => current ? { ...current, project: next } : current); props.onChanged(); if (done) toast.success(done); return true }
    catch (cause) { toast.error(cause); return false }
  }
  const remove = async () => {
    if (!project || !window.confirm(t('Delete “{name}”? Its instructions and knowledge are deleted. Its chats are kept and move to your chat history.', { name: project.name }))) return
    try { await api.deleteChatProject(id); toast.success(t('Deleted {name}.', { name: project.name })); props.onChanged(); onView({ kind: 'list' }) }
    catch (cause) { toast.error(cause) }
  }
  const startEdit = () => { if (!project) return; setDraftName(project.name); setDraftDescription(project.description); setEditing(true) }
  const saveEdit = async () => { if (!draftName.trim()) return; if (await update({ name: draftName.trim(), description: draftDescription.trim() })) setEditing(false) }
  const projectMenu = (event: React.MouseEvent) => project && openMenu(event, [
    { label: t('Edit details'), icon: <Pencil size={14} />, onSelect: startEdit },
    { label: project.starred ? t('Unstar') : t('Star'), icon: <Star size={14} />, onSelect: () => void update({ starred: !project.starred }) },
    { label: project.archived ? t('Unarchive') : t('Archive'), icon: <Archive size={14} />, onSelect: () => void update({ archived: !project.archived }, project.archived ? t('Project restored.') : t('Project archived.')) },
    'separator',
    { label: t('Delete project'), icon: <Trash2 size={14} />, danger: true, onSelect: () => void remove() },
  ])
  const chatMenu = (event: React.MouseEvent, chat: SessionSummary) => openMenu(event, [
    { label: t('Open'), icon: <MessageSquare size={14} />, disabled: busy, onSelect: () => props.onOpenChat(chat.id) },
    { label: t('Rename…'), icon: <Pencil size={14} />, disabled: busy, onSelect: () => { setRenaming(chat.id); setRenameValue(chat.title) } },
    { label: t('Remove from project'), icon: <FolderMinus size={14} />, disabled: busy, onSelect: () => props.onMoveChat(chat.id, null) },
    'separator',
    { label: t('Delete chat'), icon: <Trash2 size={14} />, danger: true, disabled: busy || chat.running, onSelect: () => props.onDeleteChat(chat.id) },
  ])
  const finishRename = () => { if (renaming && renameValue.trim()) props.onRenameChat(renaming, renameValue.trim()); setRenaming(null) }
  const send = () => { const text = message.trim(); if (!text || busy) return; setMessage(''); props.onStartChat(id, text) }

  const uploadFiles = async () => {
    if (!isTauri()) { toast.info(t('Open the Neru desktop window to add files.')); return }
    try {
      const picked = await openDialog({ multiple: true, title: t('Add files to project knowledge'), filters: [
        { name: t('Documents'), extensions: ['pdf', 'docx', 'pptx', 'xlsx', 'odt', 'odp', 'ods', 'rtf', 'txt', 'md', 'markdown', 'csv', 'tsv', 'json', 'jsonl', 'yaml', 'yml', 'toml', 'xml', 'html', 'htm', 'log', 'tex', 'sql'] },
        { name: t('All files'), extensions: ['*'] },
      ] })
      const paths = Array.isArray(picked) ? picked : picked ? [picked] : []
      if (paths.length === 0) return
      const inspected = await api.inspectDocuments(paths)
      setReading(count => count + inspected.length)
      for (const doc of inspected) {
        try {
          if (doc.kind === 'image') throw new Error(t('Images cannot be project knowledge yet. Add text, PDF or Office files.'))
          const text = isReadableDocument(doc.kind) ? await extractDocumentText(doc.kind, await api.documentBytes(doc.path)) : undefined
          setDetail(await api.addChatProjectFile(id, doc.path, text))
          toast.success(t('Added {name}.', { name: doc.name }))
        } catch (cause) { toast.error(`${doc.name}: ${errorText(cause)}`) }
        finally { setReading(count => count - 1) }
      }
      props.onChanged()
    } catch (cause) { toast.error(cause) }
  }
  const removeItem = async (item: KnowledgeItem) => {
    if (!window.confirm(t('Remove “{title}” from the project knowledge?', { title: item.title }))) return
    try { setDetail(await api.removeChatProjectKnowledge(id, item.id)); props.onChanged() } catch (cause) { toast.error(cause) }
  }
  const viewItem = async (item: KnowledgeItem) => {
    setModal({ kind: 'view', item, text: null })
    try { const text = await api.readChatProjectKnowledge(id, item.id); setModal(current => current?.kind === 'view' && current.item.id === item.id ? { ...current, text } : current) }
    catch (cause) { toast.error(cause); setModal(null) }
  }
  const closeModal = useCallback(() => setModal(null), [])

  if (missing) return <main className="content-view cp-view"><div className="cp-column"><button type="button" className="cp-back" onClick={() => onView({ kind: 'list' })}><ArrowLeft size={14} />{t('All projects')}</button><p className="cp-empty">{t('This project no longer exists.')}</p></div></main>
  if (!detail || !project) return <main className="content-view cp-view"><p className="cp-empty cp-loading"><LoaderCircle size={16} className="animate-spin" /> {t('Loading project…')}</p></main>

  return <main className="content-view cp-view">
    <div className="cp-project">
      <section className="cp-main" aria-label={project.name}>
        <button type="button" className="cp-back" onClick={() => onView({ kind: 'list' })}><ArrowLeft size={14} />{t('All projects')}</button>
        {editing
          ? <form className="cp-edit" onSubmit={event => { event.preventDefault(); void saveEdit() }}>
            <input autoFocus value={draftName} maxLength={120} onChange={event => setDraftName(event.target.value)} aria-label={t('Project name')} />
            <textarea value={draftDescription} maxLength={2000} rows={3} onChange={event => setDraftDescription(event.target.value)} placeholder={t('What are you trying to achieve?')} aria-label={t('Project description')} />
            <div className="cp-actions"><button type="button" className="button subtle small" onClick={() => setEditing(false)}>{t('Cancel')}</button><button type="submit" className="button primary small" disabled={!draftName.trim()}>{t('Save')}</button></div>
          </form>
          : <header className="cp-project-head">
            <div className="cp-title-row">
              <h1>{project.name}</h1>
              {project.archived && <span className="cp-badge">{t('Archived')}</span>}
              <span className="cp-head-actions">
                <button type="button" className={cn('icon-button', project.starred && 'cp-starred')} aria-pressed={project.starred} aria-label={project.starred ? t('Unstar project') : t('Star project')} title={project.starred ? t('Unstar') : t('Star')} onClick={() => void update({ starred: !project.starred })}><Star size={17} weight={project.starred ? 'fill' : 'regular'} /></button>
                <button type="button" className="icon-button" aria-label={t('Project menu')} title={t('More actions')} onClick={projectMenu}><MoreHorizontal size={17} /></button>
              </span>
            </div>
            <button type="button" className="cp-description" onClick={startEdit} title={t('Edit details')}>{project.description || <em>{t('Add a description…')}</em>}</button>
          </header>}

        <form className="cp-composer" onSubmit={event => { event.preventDefault(); send() }} onClick={() => composer.current?.focus()}>
          <textarea ref={composer} value={message} rows={2} onChange={event => setMessage(event.target.value)} placeholder={t('How can I help you today?')} aria-label={t('Start a chat in {name}', { name: project.name })}
            onKeyDown={event => { if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); send() } }} />
          <div className="cp-composer-row"><span className="cp-composer-hint">{t('Starts a new chat in this project')}</span><button type="submit" className="cp-send" disabled={!message.trim() || busy} aria-label={t('Send')}><ArrowUp size={16} weight="bold" /></button></div>
        </form>

        <div className="cp-chats">
          <h2>{t('Chats')}</h2>
          {chats.length === 0
            ? <p className="cp-muted">{t('Start a chat to keep it in this project.')}</p>
            : <ul>{chats.map(chat => <li key={chat.id} className="cp-chat" onContextMenu={event => chatMenu(event, chat)}>
              {renaming === chat.id
                ? <input className="cp-chat-rename" autoFocus value={renameValue} aria-label={t('Rename chat')} onChange={event => setRenameValue(event.target.value)} onBlur={finishRename} onKeyDown={event => { if (event.key === 'Enter') finishRename(); if (event.key === 'Escape') setRenaming(null) }} />
                : <button type="button" className="cp-chat-open" disabled={busy} onClick={() => props.onOpenChat(chat.id)}>
                  <strong className="truncate">{chat.title}</strong>
                  <span>{chat.running ? t('Responding…') : t('Last message {when}', { when: relativeTime(chat.updatedAt) })}</span>
                </button>}
              <button type="button" className="icon-button small" aria-label={t('Actions for {name}', { name: chat.title })} onClick={event => chatMenu(event, chat)}><MoreHorizontal size={15} /></button>
            </li>)}</ul>}
        </div>
      </section>

      <KnowledgePanel detail={detail} reading={reading} onInstructions={() => setModal({ kind: 'instructions' })} onUpload={() => void uploadFiles()} onAddText={() => setModal({ kind: 'text' })} onView={item => void viewItem(item)} onRemove={item => void removeItem(item)} />
    </div>

    {modal?.kind === 'instructions' && <InstructionsDialog initial={project.instructions} name={project.name} onClose={closeModal} onSave={async text => { if (await update({ instructions: text }, t('Instructions saved.'))) closeModal() }} />}
    {modal?.kind === 'text' && <TextDialog onClose={closeModal} onSave={async (title, text) => {
      try { setDetail(await api.addChatProjectText(id, title, text)); props.onChanged(); toast.success(t('Added {name}.', { name: title })); closeModal() } catch (cause) { toast.error(cause) }
    }} />}
    {modal?.kind === 'view' && <Dialog title={modal.item.title} onClose={closeModal} wide>
      <div className="cp-view-meta">{modal.item.kind === 'file' ? t('File') : t('Text')} · {t('~{n} tokens', { n: modal.item.tokens.toLocaleString() })} · {t('Added {when}', { when: relativeTime(modal.item.added) })}</div>
      {modal.text === null ? <p className="cp-empty"><LoaderCircle size={16} className="animate-spin" /></p> : <pre className="cp-view-text">{modal.text}</pre>}
    </Dialog>}
  </main>
}

function KnowledgePanel({ detail, reading, onInstructions, onUpload, onAddText, onView, onRemove }: {
  detail: ChatProjectDetail; reading: number; onInstructions: () => void; onUpload: () => void; onAddText: () => void; onView: (item: KnowledgeItem) => void; onRemove: (item: KnowledgeItem) => void
}) {
  const { percent, threshold, retrieval } = capacity(detail)
  const [addOpen, setAddOpen] = useState(false)
  const shown = Math.min(100, percent)
  return <aside className="cp-knowledge" aria-label={t('Project knowledge')}>
    <section className="cp-panel-block">
      <header><h2>{t('Instructions')}</h2><button type="button" className="button subtle small" onClick={onInstructions}><Pencil size={13} />{detail.project.instructions ? t('Edit') : t('Set')}</button></header>
      {detail.project.instructions
        ? <p className="cp-instructions-preview">{detail.project.instructions}</p>
        : <button type="button" className="cp-placeholder" onClick={onInstructions}>{t('Set project instructions')}<span>{t('Tailor how Neru answers in every chat in this project.')}</span></button>}
    </section>
    <section className="cp-panel-block">
      <header>
        <h2>{t('Project knowledge')}</h2>
        <div className="cp-add">
          <button type="button" className="icon-button" aria-label={t('Add knowledge')} aria-expanded={addOpen} aria-haspopup="menu" onClick={() => setAddOpen(value => !value)}><Plus size={16} /></button>
          {addOpen && <div className="menu menu-down cp-add-menu" role="menu" onMouseLeave={() => setAddOpen(false)}>
            <button type="button" role="menuitem" onClick={() => { setAddOpen(false); onUpload() }}><Upload size={15} />{t('Upload from device')}</button>
            <button type="button" role="menuitem" onClick={() => { setAddOpen(false); onAddText() }}><Type size={15} />{t('Add text content')}</button>
          </div>}
        </div>
      </header>
      <div className="cp-meter" role="meter" aria-label={t('Project knowledge capacity')} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(shown)}>
        <div className={cn('cp-meter-bar', retrieval && 'over')} style={{ width: `${shown}%` }} />
        <i style={{ left: `${threshold}%` }} aria-hidden />
      </div>
      <p className="cp-meter-label">{t('{p}% of project capacity used', { p: percent < 1 && percent > 0 ? '<1' : Math.round(percent) })} · {t('~{n} tokens', { n: detail.knowledgeTokens.toLocaleString() })}</p>
      {retrieval && <p className="cp-meter-note">{t('Too large to read in full: Neru searches the knowledge for each question.')}</p>}
      {reading > 0 && <p className="cp-muted"><LoaderCircle size={13} className="animate-spin" /> {t('Reading files…')}</p>}
      {detail.knowledge.length === 0 && reading === 0
        ? <div className="cp-drop">
          <FileText size={22} aria-hidden />
          <p>{t('No knowledge added yet. Add PDFs, documents or other text for Neru to reference in every chat here.')}</p>
          <div className="cp-actions"><button type="button" className="button subtle small" onClick={onUpload}><Upload size={13} />{t('Upload')}</button><button type="button" className="button subtle small" onClick={onAddText}><Type size={13} />{t('Add text')}</button></div>
        </div>
        : <ul className="cp-items">{detail.knowledge.map(item => <li key={item.id} className="cp-item">
          <button type="button" className="cp-item-open" onClick={() => onView(item)} title={t('View {name}', { name: item.title })}>
            <span className="cp-item-icon">{item.kind === 'file' ? <FileText size={15} /> : <Type size={15} />}</span>
            <span className="cp-item-text"><strong className="truncate">{item.title}</strong><span>{t('~{n} tokens', { n: item.tokens.toLocaleString() })}</span></span>
          </button>
          <span className="cp-item-actions">
            <button type="button" className="icon-button small" onClick={() => onView(item)} aria-label={t('View {name}', { name: item.title })}><Eye size={14} /></button>
            <button type="button" className="icon-button small" onClick={() => onRemove(item)} aria-label={t('Remove {name}', { name: item.title })}><Trash2 size={14} /></button>
          </span>
        </li>)}</ul>}
    </section>
  </aside>
}

function InstructionsDialog({ initial, name, onClose, onSave }: { initial: string; name: string; onClose: () => void; onSave: (text: string) => Promise<void> }) {
  const [text, setText] = useState(initial)
  const [saving, setSaving] = useState(false)
  return <Dialog title={t('Set project instructions')} onClose={onClose}>
    <p className="cp-muted">{t('Neru follows these in every chat in {name}.', { name })}</p>
    <textarea className="cp-textarea" autoFocus rows={10} maxLength={20000} value={text} onChange={event => setText(event.target.value)} placeholder={t('Think step by step. Answer in a formal register. Use the terminology from my course notes…')} aria-label={t('Project instructions')} />
    <div className="cp-actions"><button type="button" className="button subtle" onClick={onClose}>{t('Cancel')}</button><button type="button" className="button primary" disabled={saving || text === initial} onClick={async () => { setSaving(true); await onSave(text); setSaving(false) }}>{t('Save instructions')}</button></div>
  </Dialog>
}

function TextDialog({ onClose, onSave }: { onClose: () => void; onSave: (title: string, text: string) => Promise<void> }) {
  const [title, setTitle] = useState('')
  const [text, setText] = useState('')
  const [saving, setSaving] = useState(false)
  return <Dialog title={t('Add text content')} onClose={onClose}>
    <label className="cp-field"><span>{t('Title')}</span><input autoFocus value={title} maxLength={200} onChange={event => setTitle(event.target.value)} placeholder={t('Name your content')} aria-label={t('Title')} /></label>
    <label className="cp-field"><span>{t('Content')}</span><textarea className="cp-textarea" rows={10} value={text} onChange={event => setText(event.target.value)} placeholder={t('Type or paste in content…')} aria-label={t('Content')} /></label>
    <div className="cp-actions"><button type="button" className="button subtle" onClick={onClose}>{t('Cancel')}</button><button type="button" className="button primary" disabled={saving || !title.trim() || !text.trim()} onClick={async () => { setSaving(true); await onSave(title.trim(), text); setSaving(false) }}>{t('Add content')}</button></div>
  </Dialog>
}
