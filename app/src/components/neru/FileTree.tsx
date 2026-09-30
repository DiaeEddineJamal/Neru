import { createContext, useCallback, useContext, useEffect, useReducer, useRef, useState } from 'react'
import { ChevronDown, ChevronRight, Copy, CopyPlus, ExternalLink, FileCode2, FilePlus2, FileText, Folder, FolderOpen, FolderPlus, FolderSearch, LoaderCircle, MessageSquarePlus, Paperclip, Pencil, RefreshCw, Search, Trash2, X } from 'lucide-react'
import { api } from '../../api'
import type { FileEntry } from '../../types'
import { formatCount, useIndexStatus } from '../../lib/useIndex'
import { useContextMenu, type MenuItem } from './ContextMenu'

/** The most recent batch of changed paths (project-relative), from the agent, the tree's own menu or a rescan. */
export interface TreeChange { seq: number; paths: string[] }

interface TreeProps { onSelect: (path: string) => void; selected: string | null; changed: ReadonlySet<string> }

/** What the tree can ask the app to do from its right-click menu. */
export interface TreeActions {
  onAttach?: (path: string) => void
  onAsk?: (path: string) => void
  /** A file or folder changed on disk through the menu; `gone` was removed or renamed away. */
  onChanged?: (gone?: string) => void
  onError?: (message: string) => void
}

type Editing = { mode: 'rename' | 'new_file' | 'new_folder'; path: string; value: string } | null

interface TreeState {
  editing: Editing
  setEditing: (value: Editing) => void
  menu: (event: React.MouseEvent, entry: FileEntry | null) => void
  commit: () => void
  remove: (entry: FileEntry) => void
  expanded: ReadonlySet<string>
  toggleDir: (path: string, force?: boolean) => void
  listings: Map<string, FileEntry[]>
  errors: Map<string, string>
  fresh: ReadonlySet<string>
}

const noop = () => undefined
const TreeContext = createContext<TreeState>({ editing: null, setEditing: noop, menu: noop, commit: noop, remove: noop, expanded: new Set(), toggleDir: noop, listings: new Map(), errors: new Map(), fresh: new Set() })

const norm = (path: string) => path.replace(/\\/g, '/')
const parentOf = (path: string) => norm(path).includes('/') ? norm(path).slice(0, norm(path).lastIndexOf('/')) : ''
const nameOf = (path: string) => norm(path).split('/').pop() ?? path
const CODE_FILE = /\.(tsx?|jsx?|rs|json|css|html|py|go|java|md|toml|ya?ml)$/i
/** Entries drawn at once per folder; the rest wait behind a "Show more" row so a huge folder cannot stall the UI. */
const CHUNK = 300

const sameListing = (a: FileEntry[], b: FileEntry[]) => a.length === b.length && a.every((item, index) => item.path === b[index].path && item.isDir === b[index].isDir && item.size === b[index].size)

const storageKey = (project: string) => `neru.tree.expanded.${project}`
const readExpanded = (project: string): Set<string> => {
  try { const raw = localStorage.getItem(storageKey(project)); const list = raw ? JSON.parse(raw) as unknown : []; return new Set(Array.isArray(list) ? list.filter((item): item is string => typeof item === 'string').slice(0, 200) : []) } catch { return new Set() }
}

function NameInput() {
  const { editing, setEditing, commit } = useContext(TreeContext)
  const input = useRef<HTMLInputElement>(null)
  useEffect(() => {
    const node = input.current
    if (!node || !editing) return
    node.focus()
    // Select the name without its extension, like VS Code.
    const dot = editing.value.lastIndexOf('.')
    node.setSelectionRange(0, editing.mode === 'rename' && dot > 0 ? dot : editing.value.length)
    // Only on first show.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])
  if (!editing) return null
  return <input ref={input} className="tree-rename" aria-label={editing.mode === 'rename' ? 'New name' : editing.mode === 'new_file' ? 'New file name' : 'New folder name'} value={editing.value}
    placeholder={editing.mode === 'new_folder' ? 'folder name' : 'file name'}
    onChange={event => setEditing({ ...editing, value: event.target.value })}
    onKeyDown={event => { if (event.key === 'Enter') commit(); if (event.key === 'Escape') setEditing(null) }}
    onBlur={() => commit()} />
}

function TreeNode({ entry, depth, onSelect, selected, changed }: TreeProps & { entry: FileEntry; depth: number }) {
  const { editing, menu, setEditing, remove, expanded, toggleDir, listings, errors, fresh } = useContext(TreeContext)
  const path = norm(entry.path)
  const open = entry.isDir && expanded.has(path)
  const children = open ? listings.get(path) : undefined
  const error = open ? errors.get(path) : undefined
  const [shown, setShown] = useState(CHUNK)
  const creatingHere = editing && editing.mode !== 'rename' && norm(editing.path) === path

  // Creating something inside a folder opens it.
  useEffect(() => { if (creatingHere && !open) toggleDir(path, true) }, [creatingHere, open, path, toggleDir])

  const toggle = () => {
    if (!entry.isDir) { onSelect(entry.path); return }
    toggleDir(path)
  }

  const touched = entry.isDir ? [...changed].some(item => item.startsWith(`${path}/`)) : changed.has(path)
  const flashing = fresh.has(path) || (entry.isDir && !open && [...fresh].some(item => item.startsWith(`${path}/`)))
  const Icon = entry.isDir ? (open ? FolderOpen : Folder) : CODE_FILE.test(entry.name) ? FileCode2 : FileText
  const renaming = editing?.mode === 'rename' && norm(editing.path) === path
  return (
    <>
      {renaming ? <div style={{ paddingLeft: 4 + depth * 14 }}><NameInput /></div>
        : <button className={`tree-row ${selected === entry.path ? 'selected' : ''} ${touched ? 'changed' : ''} ${flashing ? 'fresh' : ''}`} style={{ paddingLeft: 12 + depth * 14 }} onClick={toggle} onContextMenu={event => menu(event, entry)} onKeyDown={event => { if (event.key === 'F2') { event.preventDefault(); setEditing({ mode: 'rename', path: entry.path, value: entry.name }) } if (event.key === 'Delete') { event.preventDefault(); remove(entry) } }} title={touched ? `${entry.path} · changed by Neru` : entry.path}>
          {entry.isDir ? (open ? <ChevronDown size={13} /> : <ChevronRight size={13} />) : <span className="tree-spacer" />}
          <Icon size={14} strokeWidth={1.7} />
          <span className="truncate">{entry.name}</span>
          {touched && <i className="tree-dot" aria-label="Changed" />}
        </button>}
      {open && creatingHere && <div style={{ paddingLeft: 4 + (depth + 1) * 14 }}><NameInput /></div>}
      {children?.slice(0, shown).map(child => <TreeNode key={child.path} entry={child} depth={depth + 1} onSelect={onSelect} selected={selected} changed={changed} />)}
      {children && children.length > shown && <button type="button" className="tree-more" style={{ paddingLeft: 12 + (depth + 1) * 14 }} onClick={() => setShown(value => value + 500)}>Show {Math.min(500, children.length - shown)} more of {children.length - shown}</button>}
      {open && !children && !error && <div className="tree-loading" style={{ paddingLeft: 12 + (depth + 1) * 14 }} aria-hidden><i /><i /></div>}
      {open && error && <div className="tree-error">{error}</div>}
    </>
  )
}

export function FileTree({ projectKey, onSelect, selected, version = 0, changed = new Set<string>(), lastChange, onAttach, onAsk, onChanged, onError }: { projectKey: string; onSelect: (path: string) => void; selected: string | null; version?: number; changed?: ReadonlySet<string>; lastChange?: TreeChange } & TreeActions) {
  const [expanded, setExpanded] = useState<Set<string>>(() => readExpanded(projectKey))
  const [editing, setEditing] = useState<Editing>(null)
  const [fresh, setFresh] = useState<ReadonlySet<string>>(() => new Set())
  const [filter, setFilter] = useState('')
  const [matches, setMatches] = useState<string[] | null>(null)
  const [rootShown, setRootShown] = useState(CHUNK)
  const listings = useRef(new Map<string, FileEntry[]>())
  const errors = useRef(new Map<string, string>())
  const [, bump] = useReducer((count: number) => count + 1, 0)
  const queued = useRef(new Set<string>())
  const timer = useRef(0)
  const alive = useRef(true)
  const expandedRef = useRef(expanded)
  expandedRef.current = expanded
  const openMenu = useContextMenu()
  const committing = useRef(false)
  const status = useIndexStatus(projectKey)

  // Reloads the listed folders together, drawing only if some listing actually differs, so a burst
  // of file writes redraws the tree once and an unchanged folder does not redraw at all.
  const flush = useCallback(async () => {
    const dirs = [...queued.current]
    queued.current.clear()
    let dirty = false
    await Promise.all(dirs.map(async dir => {
      try {
        const items = await api.listDirectory(dir || undefined)
        const before = listings.current.get(dir)
        if (!before || !sameListing(before, items)) { listings.current.set(dir, items); dirty = true }
        if (errors.current.delete(dir)) dirty = true
      } catch (cause) {
        const message = String(cause).replace(/^Error:\s*/, '')
        if (errors.current.get(dir) !== message) { errors.current.set(dir, message); dirty = true }
        // A folder that was deleted or renamed away folds itself.
        if (dir && /os error (2|3)|cannot find|not found|no such file/i.test(message)) {
          listings.current.delete(dir); errors.current.delete(dir)
          setExpanded(current => { if (!current.has(dir)) return current; const next = new Set(current); next.delete(dir); return next })
        }
      }
    }))
    if (dirty && alive.current) bump()
  }, [])
  const reload = useCallback((dirs: Iterable<string>) => {
    for (const dir of dirs) queued.current.add(dir)
    window.clearTimeout(timer.current)
    timer.current = window.setTimeout(() => void flush(), 45)
  }, [flush])
  useEffect(() => { alive.current = true; return () => { alive.current = false; window.clearTimeout(timer.current) } }, [])

  // First load, and a manual refresh: the root and every folder that is open.
  useEffect(() => { reload(['', ...expandedRef.current]) }, [projectKey, version, reload])

  // Files changed (by the agent, a menu action, or something outside Neru): reload only the open
  // folders they sit in, and pulse the changed rows for a few seconds.
  const seq = lastChange?.seq ?? 0
  const lastPaths = useRef<string[]>([])
  lastPaths.current = lastChange?.paths ?? []
  useEffect(() => {
    if (seq === 0) return
    const paths = lastPaths.current.map(norm).filter(Boolean)
    if (paths.length === 0) return
    reload(['', ...expandedRef.current].filter(dir => dir === '' || paths.some(path => path === dir || path.startsWith(`${dir}/`))))
    const batch = paths.slice(0, 400)
    setFresh(current => new Set([...current, ...batch]))
    const clear = window.setTimeout(() => setFresh(current => { const next = new Set(current); batch.forEach(path => next.delete(path)); return next }), 4500)
    return () => window.clearTimeout(clear)
  }, [seq, reload])

  useEffect(() => { try { localStorage.setItem(storageKey(projectKey), JSON.stringify([...expanded].slice(0, 200))) } catch { /* storage unavailable */ } }, [expanded, projectKey])

  const toggleDir = useCallback((path: string, force?: boolean) => {
    const opening = force ?? !expandedRef.current.has(path)
    setExpanded(current => {
      if (current.has(path) === opening) return current
      const next = new Set(current)
      if (opening) next.add(path); else next.delete(path)
      return next
    })
    if (opening) reload([path])
  }, [reload])

  // Filter box: a fuzzy lookup over the project index, answered without touching the tree.
  useEffect(() => {
    const text = filter.trim()
    if (!text) { setMatches(null); return }
    let active = true
    const timer = window.setTimeout(() => { api.searchFiles(text, 80).then(found => { if (active) setMatches(found) }).catch(() => { if (active) setMatches([]) }) }, 70)
    return () => { active = false; window.clearTimeout(timer) }
  }, [filter])

  const absolute = (path: string) => `${projectKey.replace(/^\\\\\?\\/, '').replace(/[\\/]$/, '')}${path ? `\\${path.replace(/\//g, '\\')}` : ''}`
  const fail = (cause: unknown) => onError?.(String(cause).replace(/^Error:\s*/, ''))
  const done = (gone?: string) => { reload(['', ...expandedRef.current]); onChanged?.(gone) }
  const copy = (text: string) => { void navigator.clipboard.writeText(text).catch(fail) }

  const commit = () => {
    if (!editing || committing.current) return
    const current = editing
    const name = current.value.trim()
    setEditing(null)
    if (!name || (current.mode === 'rename' && name === nameOf(current.path))) return
    if (/[\\/:*?"<>|]/.test(name) && current.mode === 'rename') { onError?.('A name cannot contain \\ / : * ? " < > |'); return }
    committing.current = true
    const target = current.mode === 'rename' ? [parentOf(current.path), name].filter(Boolean).join('/') : [norm(current.path), name].filter(Boolean).join('/')
    const run = current.mode === 'rename' ? api.fileAction('rename', current.path, target) : api.fileAction(current.mode, target)
    void run.then(() => { done(current.mode === 'rename' ? current.path : undefined); if (current.mode === 'new_file') onSelect(target) }).catch(fail).finally(() => { committing.current = false })
  }

  const remove = async (entry: FileEntry) => {
    const what = entry.isDir ? `the folder “${entry.name}” and everything in it` : `“${entry.name}”`
    let sure = false
    try { const { ask } = await import('@tauri-apps/plugin-dialog'); sure = await ask(`Delete ${what}? Neru keeps a checkpoint, so Rewind can bring it back.`, { title: 'Delete', kind: 'warning', okLabel: 'Delete', cancelLabel: 'Cancel' }) } catch { sure = window.confirm(`Delete ${what}?`) }
    if (!sure) return
    api.fileAction('delete', entry.path).then(() => done(entry.path)).catch(fail)
  }

  const menu = (event: React.MouseEvent, entry: FileEntry | null) => {
    const folder = entry ? (entry.isDir ? entry.path : parentOf(entry.path)) : ''
    const create: MenuItem[] = [
      { label: 'New file…', icon: <FilePlus2 size={14} />, onSelect: () => setEditing({ mode: 'new_file', path: folder, value: '' }) },
      { label: 'New folder…', icon: <FolderPlus size={14} />, onSelect: () => setEditing({ mode: 'new_folder', path: folder, value: '' }) },
    ]
    if (!entry) {
      openMenu(event, [...create, 'separator', { label: 'Open File Location', icon: <FolderSearch size={14} />, onSelect: () => api.revealPath(absolute('')).catch(fail) }, { label: 'Refresh', icon: <RefreshCw size={14} />, onSelect: () => { void api.indexRefresh().catch(() => undefined); done() } }])
      return
    }
    openMenu(event, [
      ...(entry.isDir ? create : [{ label: 'Open', icon: <FileCode2 size={14} />, onSelect: () => onSelect(entry.path) } as MenuItem]),
      ...(!entry.isDir ? [{ label: 'Open in external editor', icon: <ExternalLink size={14} />, onSelect: () => api.openInEditor(entry.path).catch(fail) } as MenuItem] : []),
      'separator',
      ...(!entry.isDir && onAttach ? [{ label: 'Attach to message', icon: <Paperclip size={14} />, onSelect: () => onAttach(entry.path) } as MenuItem] : []),
      ...(onAsk ? [{ label: 'Ask Neru about this', icon: <MessageSquarePlus size={14} />, onSelect: () => onAsk(entry.path) } as MenuItem] : []),
      'separator',
      { label: 'Rename…', icon: <Pencil size={14} />, shortcut: 'F2', onSelect: () => setEditing({ mode: 'rename', path: entry.path, value: entry.name }) },
      ...(!entry.isDir ? [{ label: 'Duplicate', icon: <CopyPlus size={14} />, onSelect: () => api.fileAction('duplicate', entry.path).then(() => done()).catch(fail) } as MenuItem] : []),
      'separator',
      { label: 'Copy path', icon: <Copy size={14} />, onSelect: () => copy(absolute(norm(entry.path))) },
      { label: 'Copy relative path', icon: <Copy size={14} />, onSelect: () => copy(norm(entry.path)) },
      { label: 'Open File Location', icon: <FolderSearch size={14} />, onSelect: () => api.revealPath(entry.path).catch(fail) },
      'separator',
      { label: 'Delete', icon: <Trash2 size={14} />, danger: true, shortcut: 'Del', onSelect: () => void remove(entry) },
    ])
  }

  const rootEntries = listings.current.get('')
  const rootError = errors.current.get('')
  const creatingAtRoot = editing && editing.mode !== 'rename' && editing.path === ''
  const filtering = filter.trim().length > 0
  return <TreeContext.Provider value={{ editing, setEditing, menu, commit, remove: entry => void remove(entry), expanded, toggleDir, listings: listings.current, errors: errors.current, fresh }}>
    <div className="file-tree" onContextMenu={event => { if (event.target === event.currentTarget) menu(event, null) }}>
      <div className="tree-filter">
        <Search size={13} aria-hidden />
        <input value={filter} onChange={event => setFilter(event.target.value)} placeholder="Filter files…" aria-label="Filter files" spellCheck={false}
          onKeyDown={event => { if (event.key === 'Escape') setFilter(''); if (event.key === 'Enter' && matches?.[0]) onSelect(matches[0]) }} />
        {filtering && <button type="button" className="tree-filter-clear" aria-label="Clear filter" onClick={() => setFilter('')}><X size={12} /></button>}
      </div>
      {rootError && <div className="inline-error">{rootError}</div>}
      {filtering ? <>
        {matches?.map(path => <button type="button" key={path} className={`tree-row tree-hit ${selected === path ? 'selected' : ''}`} style={{ paddingLeft: 12 }} onClick={() => onSelect(path)} title={path}>
          {CODE_FILE.test(path) ? <FileCode2 size={14} strokeWidth={1.7} /> : <FileText size={14} strokeWidth={1.7} />}
          <span className="truncate">{nameOf(path)}</span>
          <span className="tree-hit-dir truncate">{parentOf(path)}</span>
        </button>)}
        {matches && matches.length === 0 && <div className="empty-small">No files match “{filter.trim()}”.</div>}
      </> : <>
        {creatingAtRoot && <div style={{ paddingLeft: 4 }}><NameInput /></div>}
        {rootEntries?.slice(0, rootShown).map(entry => <TreeNode key={entry.path} entry={entry} depth={0} onSelect={onSelect} selected={selected} changed={changed} />)}
        {rootEntries && rootEntries.length > rootShown && <button type="button" className="tree-more" onClick={() => setRootShown(value => value + 500)}>Show {Math.min(500, rootEntries.length - rootShown)} more of {rootEntries.length - rootShown}</button>}
        {rootEntries && rootEntries.length === 0 && !rootError && !creatingAtRoot && <div className="empty-small" onContextMenu={event => menu(event, null)}>This folder is empty. Ask Neru to create the first files, or right-click to add one.</div>}
      </>}
      {status && <div className="tree-index" role="status" aria-live="polite" title={status.state === 'ready' ? `${formatCount(status.files)} files and ${formatCount(status.symbols)} symbols indexed for instant search` : undefined}>
        {status.state === 'indexing' ? <><LoaderCircle size={11} className="animate-spin" aria-hidden /> Indexing… {formatCount(status.files)}{status.total > 0 ? ` of ${formatCount(status.total)}` : ''} files</>
          : status.state === 'ready' ? <><i className="tree-index-dot" aria-hidden /> Indexed {formatCount(status.files)} files{status.symbols > 0 ? ` · ${formatCount(status.symbols)} symbols` : ''}</>
          : 'Search index unavailable'}
      </div>}
    </div>
  </TreeContext.Provider>
}
