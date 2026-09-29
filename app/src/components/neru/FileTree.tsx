import { useEffect, useState } from 'react'
import { ChevronDown, ChevronRight, FileCode2, FileText, Folder, FolderOpen } from 'lucide-react'
import { api } from '../../api'
import type { FileEntry } from '../../types'

interface TreeProps { onSelect: (path: string) => void; selected: string | null; version: number; changed: ReadonlySet<string> }

const norm = (path: string) => path.replace(/\\/g, '/')

function TreeNode({ entry, depth, ...props }: TreeProps & { entry: FileEntry; depth: number }) {
  const { onSelect, selected, version, changed } = props
  const [open, setOpen] = useState(false)
  const [children, setChildren] = useState<FileEntry[] | null>(null)
  const [error, setError] = useState('')

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
  return (
    <>
      <button className={`tree-row ${selected === entry.path ? 'selected' : ''} ${touched ? 'changed' : ''}`} style={{ paddingLeft: 12 + depth * 14 }} onClick={toggle} title={touched ? `${entry.path} · changed by Neru` : entry.path}>
        {entry.isDir ? (open ? <ChevronDown size={13} /> : <ChevronRight size={13} />) : <span className="tree-spacer" />}
        <Icon size={14} strokeWidth={1.7} />
        <span className="truncate">{entry.name}</span>
        {touched && <i className="tree-dot" aria-label="Changed" />}
      </button>
      {open && children?.map(child => <TreeNode key={child.path} entry={child} depth={depth + 1} {...props} />)}
      {open && error && <div className="tree-error">{error}</div>}
    </>
  )
}

export function FileTree({ projectKey, onSelect, selected, version = 0, changed = new Set<string>() }: { projectKey: string; onSelect: (path: string) => void; selected: string | null; version?: number; changed?: ReadonlySet<string> }) {
  const [entries, setEntries] = useState<FileEntry[]>([])
  const [error, setError] = useState('')
  useEffect(() => {
    let active = true
    api.listDirectory().then(items => { if (active) { setEntries(items); setError('') } }).catch(cause => { if (active) setError(String(cause)) })
    return () => { active = false }
  }, [projectKey, version])
  return <div className="file-tree">{error && <div className="inline-error">{error}</div>}{entries.map(entry => <TreeNode key={entry.path} entry={entry} depth={0} onSelect={onSelect} selected={selected} version={version} changed={changed} />)}{entries.length === 0 && !error && <div className="empty-small">This folder is empty. Ask Neru to create the first files.</div>}</div>
}
