import { createContext, useContext, useEffect, useRef, useState } from 'react'
import { ChevronDown, ChevronRight, Copy, CopyPlus, ExternalLink, FileCode2, FilePlus2, FileText, Folder, FolderOpen, FolderPlus, FolderSearch, MessageSquarePlus, Paperclip, Pencil, RefreshCw, Trash2 } from 'lucide-react'
import { api } from '../../api'
import type { FileEntry } from '../../types'
import { useContextMenu, type MenuItem } from './ContextMenu'

interface TreeProps { onSelect: (path: string) => void; selected: string | null; version: number; changed: ReadonlySet<string> }

/** What the tree can ask the app to do from its right-click menu. */
export interface TreeActions {
  onAttach?: (path: string) => void
  onAsk?: (path: string) => void
  /** A file or folder changed on disk through the menu; `gone` was removed or renamed away. */
  onChanged?: (gone?: string) => void
  onError?: (message: string) => void
}

type Editing = { mode: 'rename' | 'new_file' | 'new_folder'; path: string; value: string } | null

const TreeContext = createContext<{ editing: Editing; setEditing: (value: Editing) => void; menu: (event: React.MouseEvent, entry: FileEntry | null) => void; commit: () => void; remove: (entry: FileEntry) => void }>({ editing: null, setEditing: () => undefined, menu: () => undefined, commit: () => undefined, remove: () => undefined })

const norm = (path: string) => path.replace(/\\/g, '/')
const parentOf = (path: string) => norm(path).includes('/') ? norm(path).slice(0, norm(path).lastIndexOf('/')) : ''
const nameOf = (path: string) => norm(path).split('/').pop() ?? path

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

function TreeNode({ entry, depth, ...props }: TreeProps & { entry: FileEntry; depth: number }) {
  const { onSelect, selected, version, changed } = props
  const { editing, menu, setEditing, remove } = useContext(TreeContext)
  const [open, setOpen] = useState(false)
  const [children, setChildren] = useState<FileEntry[] | null>(null)
  const [error, setError] = useState('')
  const creatingHere = editing && editing.mode !== 'rename' && norm(editing.path) === norm(entry.path)

  // Creating something inside a folder opens it.
  useEffect(() => { if (creatingHere && !open) setOpen(true) }, [creatingHere, open])
  // When files change on disk, an open folder reloads its entries and stays open.
  useEffect(() => {
    if (!open || !entry.isDir) return
    let active = true
    api.listDirectory(entry.path).then(items => { if (active) { setChildren(items); setError('') } }).catch(cause => { if (active) setError(String(cause)) })
    return () => { active = false }
  }, [version, open, entry.isDir, entry.path])

  const toggle = () => {
    if (!entry.isDir) { onSelect(entry.path); return }
    setOpen(!open)
  }

  const path = norm(entry.path)
  const touched = entry.isDir ? [...changed].some(item => item.startsWith(`${path}/`)) : changed.has(path)
  const Icon = entry.isDir ? (open ? FolderOpen : Folder) : /\.(tsx?|jsx?|rs|json|css|html)$/.test(entry.name) ? FileCode2 : FileText
  const renaming = editing?.mode === 'rename' && norm(editing.path) === path
  return (
    <>
      {renaming ? <div style={{ paddingLeft: 4 + depth * 14 }}><NameInput /></div>
        : <button className={`tree-row ${selected === entry.path ? 'selected' : ''} ${touched ? 'changed' : ''}`} style={{ paddingLeft: 12 + depth * 14 }} onClick={toggle} onContextMenu={event => menu(event, entry)} onKeyDown={event => { if (event.key === 'F2') { event.preventDefault(); setEditing({ mode: 'rename', path: entry.path, value: entry.name }) } if (event.key === 'Delete') { event.preventDefault(); remove(entry) } }} title={touched ? `${entry.path} · changed by Neru` : entry.path}>
          {entry.isDir ? (open ? <ChevronDown size={13} /> : <ChevronRight size={13} />) : <span className="tree-spacer" />}
          <Icon size={14} strokeWidth={1.7} />
          <span className="truncate">{entry.name}</span>
          {touched && <i className="tree-dot" aria-label="Changed" />}
        </button>}
      {open && creatingHere && <div style={{ paddingLeft: 4 + (depth + 1) * 14 }}><NameInput /></div>}
      {open && children?.map(child => <TreeNode key={child.path} entry={child} depth={depth + 1} {...props} />)}
      {open && error && <div className="tree-error">{error}</div>}
    </>
  )
}

export function FileTree({ projectKey, onSelect, selected, version = 0, changed = new Set<string>(), onAttach, onAsk, onChanged, onError }: { projectKey: string; onSelect: (path: string) => void; selected: string | null; version?: number; changed?: ReadonlySet<string> } & TreeActions) {
  const [entries, setEntries] = useState<FileEntry[]>([])
  const [error, setError] = useState('')
  const [editing, setEditing] = useState<Editing>(null)
  const [refresh, setRefresh] = useState(0)
  const openMenu = useContextMenu()
  const committing = useRef(false)
  useEffect(() => {
    let active = true
    api.listDirectory().then(items => { if (active) { setEntries(items); setError('') } }).catch(cause => { if (active) setError(String(cause)) })
    return () => { active = false }
  }, [projectKey, version, refresh])

  const absolute = (path: string) => `${projectKey.replace(/^\\\\\?\\/, '').replace(/[\\/]$/, '')}${path ? `\\${path.replace(/\//g, '\\')}` : ''}`
  const fail = (cause: unknown) => onError?.(String(cause).replace(/^Error:\s*/, ''))
  const done = (gone?: string) => { setRefresh(value => value + 1); onChanged?.(gone) }
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
      openMenu(event, [...create, 'separator', { label: 'Open File Location', icon: <FolderSearch size={14} />, onSelect: () => api.revealPath(absolute('')).catch(fail) }, { label: 'Refresh', icon: <RefreshCw size={14} />, onSelect: () => setRefresh(value => value + 1) }])
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

  const creatingAtRoot = editing && editing.mode !== 'rename' && editing.path === ''
  return <TreeContext.Provider value={{ editing, setEditing, menu, commit, remove: entry => void remove(entry) }}>
    <div className="file-tree" onContextMenu={event => { if (event.target === event.currentTarget) menu(event, null) }}>
      {error && <div className="inline-error">{error}</div>}
      {creatingAtRoot && <div style={{ paddingLeft: 4 }}><NameInput /></div>}
      {entries.map(entry => <TreeNode key={entry.path} entry={entry} depth={0} onSelect={onSelect} selected={selected} version={version + refresh} changed={changed} />)}
      {entries.length === 0 && !error && !creatingAtRoot && <div className="empty-small" onContextMenu={event => menu(event, null)}>This folder is empty. Ask Neru to create the first files, or right-click to add one.</div>}
    </div>
  </TreeContext.Provider>
}
