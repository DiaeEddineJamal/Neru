import { useEffect, useRef, useState, type PointerEvent as ReactPointerEvent } from 'react'
import { BookOpen, Compass, ChevronRight, Command, Copy, Folder, FolderMinus, FolderOpen, FolderPlus, FolderSearch, GitBranch, GitBranchPlus, LoaderCircle, MessageSquare, Moon, MoreHorizontal, Pencil, Plus, Search, Settings2, SquarePen, Sun, Trash2, Users } from 'lucide-react'
import { useContextMenu } from './ContextMenu'
import { Mascot } from './Mascot'
import type { ProjectInfo, Section, SessionSummary } from '../../types'

const MIN_WIDTH = 220
const MAX_WIDTH = 440
const DEFAULT_WIDTH = 264
const WIDTH_KEY = 'neru.sidebar.width'
const EXPANDED_KEY = 'neru.sidebar.expanded'
const isMac = /Mac/i.test(navigator.platform)
const mod = isMac ? '⌘' : 'Ctrl'

const tools: { id: Section; label: string; icon: typeof Folder }[] = [
  { id: 'search', label: 'Search', icon: Search },
]

const readStorage = <T,>(key: string, fallback: T): T => { try { const raw = localStorage.getItem(key); return raw ? JSON.parse(raw) as T : fallback } catch { return fallback } }
const writeStorage = (key: string, value: unknown) => { try { localStorage.setItem(key, JSON.stringify(value)) } catch { /* storage unavailable */ } }
const baseName = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path

function relativeTime(ms: number) {
  const minutes = Math.floor((Date.now() - ms) / 60000)
  if (minutes < 1) return 'now'
  if (minutes < 60) return `${minutes}m`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours}h`
  const days = Math.floor(hours / 24)
  if (days < 7) return `${days}d`
  return new Date(ms).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })
}

export interface SidebarProps {
  surface: 'chat' | 'code'
  project: ProjectInfo | null
  recent: string[]
  sessions: SessionSummary[]
  activeSessionId: string | null
  section: Section
  busy: boolean
  light: boolean
  model: string
  onSection: (section: Section) => void
  onNewSession: (projectPath?: string, worktree?: boolean) => void
  onOpenSession: (projectPath: string, id: string) => void
  onOpenProject: () => void
  onCloneProject: () => void
  onRenameSession: (id: string, title: string) => void
  onDeleteSession: (id: string) => void
  /** Takes a project off the list; its files and saved sessions are kept. */
  onForgetProject: (path: string) => void
  onRevealProject: (path: string) => void
  onToggleTheme: () => void
  onPalette: () => void
  onGuide: () => void
  onTour: () => void
  /** Collapse when docked; dock (keep open) when shown on hover. */
  onCollapse: () => void
  /** Rendered over the workspace while the pointer hovers the left edge. */
  floating?: boolean
}

export function Sidebar(props: SidebarProps) {
  const { project, recent, sessions, activeSessionId, section, busy, light } = props
  const [width, setWidth] = useState(() => Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, readStorage(WIDTH_KEY, DEFAULT_WIDTH))))
  const [resizing, setResizing] = useState(false)
  const [expanded, setExpanded] = useState<Record<string, boolean>>(() => readStorage(EXPANDED_KEY, {}))
  const [renamingId, setRenamingId] = useState<string | null>(null)
  const openMenu = useContextMenu()
  const projectMenu = (event: React.MouseEvent, path: string, name: string, current: boolean) => openMenu(event, [
    { label: 'New session', icon: <Plus size={14} />, disabled: busy, onSelect: () => props.onNewSession(path) },
    ...(current && project?.git ? [{ label: 'New worktree session', icon: <GitBranchPlus size={14} />, disabled: busy, onSelect: () => props.onNewSession(path, true) }] : []),
    'separator',
    { label: 'Open File Location', icon: <FolderSearch size={14} />, onSelect: () => props.onRevealProject(path) },
    { label: 'Copy path', icon: <Copy size={14} />, onSelect: () => void navigator.clipboard.writeText(path.replace(/^\\\\\?\\/, '')) },
    'separator',
    { label: `Remove ${name} from Neru`, icon: <FolderMinus size={14} />, danger: true, disabled: busy, onSelect: () => props.onForgetProject(path) },
  ])
  const sessionMenu = (event: React.MouseEvent, path: string, session: SessionSummary, current: boolean) => openMenu(event, [
    { label: 'Open', icon: <MessageSquare size={14} />, disabled: busy, onSelect: () => props.onOpenSession(path, session.id) },
    { label: 'Rename…', icon: <Pencil size={14} />, disabled: busy || !current, onSelect: () => { setRenamingId(session.id); setRenameValue(session.title) } },
    'separator',
    { label: 'Delete session', icon: <Trash2 size={14} />, danger: true, disabled: busy || session.running || !current, onSelect: () => props.onDeleteSession(session.id) },
  ])
  const [renameValue, setRenameValue] = useState('')
  const [accountOpen, setAccountOpen] = useState(false)
  const [addOpen, setAddOpen] = useState(false)
  const accountRef = useRef<HTMLDivElement>(null)
  const addRef = useRef<HTMLDivElement>(null)

  useEffect(() => writeStorage(EXPANDED_KEY, expanded), [expanded])
  useEffect(() => {
    if (!accountOpen && !addOpen) return
    const close = (event: MouseEvent) => {
      if (accountRef.current?.contains(event.target as Node) || addRef.current?.contains(event.target as Node)) return
      setAccountOpen(false); setAddOpen(false)
    }
    const escape = (event: KeyboardEvent) => { if (event.key === 'Escape') { setAccountOpen(false); setAddOpen(false) } }
    window.addEventListener('mousedown', close); window.addEventListener('keydown', escape)
    return () => { window.removeEventListener('mousedown', close); window.removeEventListener('keydown', escape) }
  }, [accountOpen, addOpen])

  const startResize = (event: ReactPointerEvent<HTMLDivElement>) => {
    event.preventDefault()
    const startX = event.clientX
    const startWidth = width
    let next = startWidth
    setResizing(true)
    const move = (moveEvent: PointerEvent) => { next = Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, startWidth + moveEvent.clientX - startX)); setWidth(next) }
    const up = () => { setResizing(false); writeStorage(WIDTH_KEY, next); window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up) }
    window.addEventListener('pointermove', move); window.addEventListener('pointerup', up)
  }
  const resetWidth = () => { setWidth(DEFAULT_WIDTH); writeStorage(WIDTH_KEY, DEFAULT_WIDTH) }

  const projectPaths = [...new Set([...(project ? [project.path] : []), ...recent])]
  const isExpanded = (path: string) => expanded[path] ?? path === project?.path
  const toggle = (path: string) => setExpanded(current => ({ ...current, [path]: !isExpanded(path) }))
  const finishRename = () => { if (renamingId && renameValue.trim()) props.onRenameSession(renamingId, renameValue.trim()); setRenamingId(null) }

  return <aside className={`sidebar ${resizing ? 'resizing' : ''} ${props.floating ? 'floating' : ''}`} style={{ width, flexBasis: width }}>
    <div className="sidebar-brand-row">
      <button className="sidebar-brand" onClick={() => props.onSection('home')} title="Neru home"><Mascot size={36} className="brand-mascot" /><strong>Neru</strong><span lang="ja">練る</span></button>
    </div>

    <nav className="sidebar-nav" aria-label="Main navigation">
      <button className="nav-item new-session" data-tour="new-session" onClick={() => props.onNewSession()} disabled={(props.surface === 'code' && !project) || busy}><SquarePen size={16} /><span>New session</span><kbd>{mod} N</kbd></button>
      <button data-tour="team-nav" className={`nav-item ${section === 'team' ? 'active' : ''}`} onClick={() => props.onSection('team')} title="Claude Code, Codex and more in one shared thread"><Users size={16} strokeWidth={1.75} /><span>Team</span></button>
      {props.surface === 'code' && tools.map(item => <button key={item.id} className={`nav-item ${section === item.id ? 'active' : ''}`} onClick={() => props.onSection(item.id)} disabled={!project}><item.icon size={16} strokeWidth={1.75} /><span>{item.label}</span></button>)}
    </nav>

    <div className="sidebar-scroll" data-tour="sessions">
      {props.surface === 'chat' ? <>
        <div className="sidebar-label"><span>Chat history</span></div>
        {sessions.filter(session => !session.projectPath).length === 0 && <p className="sidebar-empty">No chats yet.</p>}
        <div className="session-list">
          {sessions.filter(session => !session.projectPath).map(session => <div className={`session-row ${session.id === activeSessionId && section === 'home' ? 'active' : ''}`} key={session.id}>
            {renamingId === session.id
              ? <input className="session-rename" autoFocus aria-label="Rename session" value={renameValue} onChange={event => setRenameValue(event.target.value)} onBlur={finishRename} onKeyDown={event => { if (event.key === 'Enter') finishRename(); if (event.key === 'Escape') setRenamingId(null) }} />
              : <>
                <button className="session-select" disabled={busy} onClick={() => props.onOpenSession('', session.id)} title={session.title}><span className="truncate">{session.title}</span></button>
                {session.running
                  ? <span className="session-running" title="Responding"><LoaderCircle size={13} className="animate-spin" aria-label="Responding" /></span>
                  : <span className="session-time">{relativeTime(session.updatedAt)}</span>}
                <span className="session-actions">
                  <button disabled={busy} aria-label={`Rename ${session.title}`} title="Rename" onClick={() => { setRenamingId(session.id); setRenameValue(session.title) }}><Pencil size={13} /></button>
                  <button disabled={busy || session.running} aria-label={`Delete ${session.title}`} title="Delete" onClick={() => props.onDeleteSession(session.id)}><Trash2 size={13} /></button>
                </span>
              </>}
          </div>)}
        </div>
      </> : <>
      <div className="sidebar-label">
        <span>Projects</span>
        <div className="menu-anchor" ref={addRef}>
          <button className="icon-button small" onClick={() => setAddOpen(value => !value)} disabled={busy} aria-label="Add project" title="Add project" aria-expanded={addOpen}><Plus size={14} /></button>
          {addOpen && <div className="menu menu-down" role="menu">
            <button role="menuitem" onClick={() => { setAddOpen(false); props.onOpenProject() }}><FolderPlus size={15} />Open folder…</button>
            <button role="menuitem" onClick={() => { setAddOpen(false); props.onCloneProject() }}><GitBranch size={15} />Clone repository…</button>
          </div>}
        </div>
      </div>
      {projectPaths.length === 0 && <p className="sidebar-empty">Open a folder to start your first session.</p>}
      {projectPaths.map(path => {
        const open = isExpanded(path)
        const current = path === project?.path
        const projectSessions = sessions.filter(session => session.projectPath === path)
        const name = current && project ? project.name : baseName(path)
        return <section className={`project-group ${current ? 'current' : ''}`} key={path} aria-label={`${name} sessions`}>
          <div className="project-row">
            <button className="project-toggle" onClick={() => toggle(path)} onContextMenu={event => projectMenu(event, path, name, current)} aria-expanded={open} title={path}>
              <span className="project-icon">{open ? <FolderOpen size={16} strokeWidth={1.75} /> : <Folder size={16} strokeWidth={1.75} />}<ChevronRight size={14} className={`chevron ${open ? 'open' : ''}`} /></span>
              <span className="truncate">{name}</span>
            </button>
            {current && project?.git && <button className="icon-button small row-action" onClick={() => props.onNewSession(path, true)} disabled={busy} aria-label={`New worktree session in ${name}`} title="New session in its own Git worktree, for parallel work"><GitBranchPlus size={14} /></button>}
            <button className="icon-button small row-action" onClick={event => projectMenu(event, path, name, current)} aria-label={`More actions for ${name}`} title="More actions"><MoreHorizontal size={14} /></button>
            <button className="icon-button small row-action" onClick={() => props.onNewSession(path)} disabled={busy} aria-label={`New session in ${name}`} title={`New session in ${name}`}><Plus size={14} /></button>
          </div>
          {open && <div className="session-list">
            {projectSessions.length === 0 && <p className="session-empty">No sessions yet</p>}
            {projectSessions.map(session => <div className={`session-row ${session.id === activeSessionId && section === 'home' ? 'active' : ''}`} key={session.id} onContextMenu={event => sessionMenu(event, path, session, current)}>
              {renamingId === session.id
                ? <input className="session-rename" autoFocus aria-label="Rename session" value={renameValue} onChange={event => setRenameValue(event.target.value)} onBlur={finishRename} onKeyDown={event => { if (event.key === 'Enter') finishRename(); if (event.key === 'Escape') setRenamingId(null) }} />
                : <>
                  <button className="session-select" disabled={busy} onClick={() => props.onOpenSession(path, session.id)} title={session.worktree ? `${session.title} · worktree ${session.worktree.branch}` : session.title}>{session.worktree && <GitBranch size={12} className="session-worktree" aria-label="Worktree session" />}<span className="truncate">{session.title}</span></button>
                  {session.running
                    ? <span className="session-running" title="Responding"><LoaderCircle size={13} className="animate-spin" aria-label="Responding" /></span>
                    : <span className="session-time">{relativeTime(session.updatedAt)}</span>}
                  {current && <span className="session-actions">
                    <button disabled={busy} aria-label={`Rename ${session.title}`} title="Rename" onClick={() => { setRenamingId(session.id); setRenameValue(session.title) }}><Pencil size={13} /></button>
                    <button disabled={busy || session.running} aria-label={`Delete ${session.title}`} title={session.worktree ? 'Delete (its worktree is removed if it has no uncommitted work)' : 'Delete'} onClick={() => props.onDeleteSession(session.id)}><Trash2 size={13} /></button>
                  </span>}
                </>}
            </div>)}
          </div>}
        </section>
      })}
      </>}
    </div>

    <div className="sidebar-footer menu-anchor" ref={accountRef}>
      {accountOpen && <div className="menu menu-up" role="menu">
        <button role="menuitem" onClick={() => { setAccountOpen(false); props.onPalette() }}><Command size={15} />Command palette<kbd>{mod} K</kbd></button>
        <button role="menuitem" onClick={props.onToggleTheme}>{light ? <Moon size={15} /> : <Sun size={15} />}{light ? 'Dark theme' : 'Light theme'}</button>
        <div className="menu-separator" />
        <button role="menuitem" onClick={() => { setAccountOpen(false); props.onGuide() }}><BookOpen size={15} />Getting started</button>
        <button role="menuitem" onClick={() => { setAccountOpen(false); props.onTour() }}><Compass size={15} />Take the tour</button>
      </div>}
      <div className="sidebar-footer-row" data-tour="settings">
        <button className={`footer-settings ${section === 'settings' ? 'active' : ''}`} onClick={() => { setAccountOpen(false); props.onSection('settings') }} title={`Settings (${mod} ,)`}>
          <Settings2 size={16} strokeWidth={1.75} /><span>Settings</span><kbd>{mod} ,</kbd>
        </button>
        <button className={`icon-button footer-more ${accountOpen ? 'active' : ''}`} onClick={() => setAccountOpen(value => !value)} aria-expanded={accountOpen} aria-haspopup="menu" aria-label="More options" title="More"><MoreHorizontal size={16} /></button>
      </div>
    </div>

    <div className="sidebar-resizer" onPointerDown={startResize} onDoubleClick={resetWidth} role="separator" aria-orientation="vertical" aria-label="Resize sidebar" title="Drag to resize, double-click to reset" />
  </aside>
}
