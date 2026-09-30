import { useEffect, useRef, useState, type ReactNode } from 'react'
import { ChevronDown, ChevronRight, Laptop, MessageCircle } from 'lucide-react'
import { cn } from '@/lib/utils'

export type TitleMenuItem =
  | { label: string; shortcut?: string; danger?: boolean; disabled?: boolean; onSelect: () => void }
  | { label: string; disabled?: boolean; children: TitleMenuItem[] }
  | 'separator'

/** A dropdown that opens under its trigger, with hover submenus, like Claude's title-bar menus. */
function TitleMenu({ items, onClose, align = 'left' }: { items: TitleMenuItem[]; onClose: () => void; align?: 'left' | 'right' }) {
  const [openSub, setOpenSub] = useState<string | null>(null)
  // A single letter shown beside an item is also its key while the menu is open.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.ctrlKey || event.metaKey || event.altKey || event.key.length !== 1) return
      const item = items.find(entry => entry !== 'separator' && !('children' in entry) && !entry.disabled && entry.shortcut?.length === 1 && entry.shortcut.toLowerCase() === event.key.toLowerCase())
      if (item && item !== 'separator' && !('children' in item)) { event.preventDefault(); onClose(); item.onSelect() }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [items, onClose])
  return <div className={cn('title-menu', align === 'right' && 'right')} role="menu">
    {items.map((item, index) => {
      if (item === 'separator') return <div key={`s${index}`} className="title-menu-sep" role="separator" />
      if ('children' in item) {
        return <div key={item.label} className="title-menu-group" onMouseEnter={() => setOpenSub(item.label)} onMouseLeave={() => setOpenSub(current => current === item.label ? null : current)}>
          <button type="button" role="menuitem" aria-haspopup="menu" aria-expanded={openSub === item.label} disabled={item.disabled} className={cn('title-menu-row', openSub === item.label && 'active')} onClick={() => setOpenSub(item.label)}>
            <span>{item.label}</span><ChevronRight size={14} />
          </button>
          {openSub === item.label && <div className="title-menu title-submenu" role="menu">
            {item.children.map((child, at) => child === 'separator' || 'children' in child
              ? <div key={`c${at}`} className="title-menu-sep" />
              : <button key={child.label} type="button" role="menuitem" disabled={child.disabled} className="title-menu-row" onClick={() => { onClose(); child.onSelect() }}><span>{child.label}</span>{child.shortcut && <kbd>{child.shortcut}</kbd>}</button>)}
          </div>}
        </div>
      }
      return <button key={item.label} type="button" role="menuitem" disabled={item.disabled} className={cn('title-menu-row', item.danger && 'danger')} onClick={() => { onClose(); item.onSelect() }}>
        <span>{item.label}</span>{item.shortcut && <kbd>{item.shortcut}</kbd>}
      </button>
    })}
  </div>
}

/** Owns open/close for one dropdown: outside click and Escape dismiss it. */
export function useDropdown() {
  const [open, setOpen] = useState(false)
  const box = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!open) return
    const away = (event: PointerEvent) => { if (!box.current?.contains(event.target as Node)) setOpen(false) }
    const escape = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.stopPropagation(); setOpen(false) } }
    window.addEventListener('pointerdown', away); window.addEventListener('keydown', escape, true); window.addEventListener('blur', () => setOpen(false))
    return () => { window.removeEventListener('pointerdown', away); window.removeEventListener('keydown', escape, true) }
  }, [open])
  return { open, setOpen, box }
}

export interface SessionTitleProps {
  title: string
  surface: 'chat' | 'code'
  projectName?: string
  branch?: string
  sessionItems: TitleMenuItem[]
  projectItems: TitleMenuItem[]
  /** Set while the title is being renamed in place. */
  renaming: boolean
  onRename: (title: string) => void
  onCancelRename: () => void
  disabled?: boolean
  trailing?: ReactNode
}

/**
 * The session's name in the title bar, as in Claude: an icon, the title with a chevron that opens the
 * session menu, and a chip for the project that opens the project menu.
 */
export function SessionTitle(props: SessionTitleProps) {
  const session = useDropdown()
  const project = useDropdown()
  const [draft, setDraft] = useState(props.title)
  useEffect(() => { if (props.renaming) setDraft(props.title) }, [props.renaming, props.title])
  const finish = () => { const next = draft.trim(); if (next && next !== props.title) props.onRename(next); else props.onCancelRename() }
  const Icon = props.surface === 'chat' ? MessageCircle : Laptop
  return <div className="session-title" data-tauri-drag-region>
    <div className="title-anchor" ref={session.box}>
      {props.renaming
        ? <div className="title-trigger renaming"><Icon size={16} strokeWidth={1.75} aria-hidden /><input autoFocus className="title-rename" aria-label="Session name" value={draft} onChange={event => setDraft(event.target.value)} onFocus={event => event.currentTarget.select()} onBlur={finish} onKeyDown={event => { if (event.key === 'Enter') finish(); if (event.key === 'Escape') props.onCancelRename() }} /></div>
        : <button type="button" className={cn('title-trigger', session.open && 'open')} aria-haspopup="menu" aria-expanded={session.open} disabled={props.disabled} title={props.title} onClick={() => { project.setOpen(false); session.setOpen(value => !value) }}>
          <Icon size={16} strokeWidth={1.75} aria-hidden /><strong className="truncate">{props.title}</strong><ChevronDown size={14} aria-hidden />
        </button>}
      {session.open && <TitleMenu items={props.sessionItems} onClose={() => session.setOpen(false)} />}
    </div>
    {props.projectName && <div className="title-anchor" ref={project.box}>
      <button type="button" className={cn('title-chip', project.open && 'open')} aria-haspopup="menu" aria-expanded={project.open} title="Project" onClick={() => { session.setOpen(false); project.setOpen(value => !value) }}>{props.projectName}</button>
      {project.open && <TitleMenu items={props.projectItems} onClose={() => project.setOpen(false)} />}
    </div>}
    {props.branch && <span className="title-chip branch" title={`Working in its own worktree on ${props.branch}`}>{props.branch}</span>}
    {props.trailing}
  </div>
}

/** A title-bar icon button that opens a menu, aligned to the right edge. */
export function IconMenu({ icon, label, items }: { icon: ReactNode; label: string; items: TitleMenuItem[] }) {
  const menu = useDropdown()
  return <div className="title-anchor" ref={menu.box}>
    <button type="button" className={cn('titlebar-button', menu.open && 'pressed')} aria-label={label} aria-haspopup="menu" aria-expanded={menu.open} title={label} onClick={() => menu.setOpen(value => !value)}>{icon}</button>
    {menu.open && <TitleMenu align="right" items={items} onClose={() => menu.setOpen(false)} />}
  </div>
}
