import { useEffect, useRef, useState, type ReactNode } from 'react'
import { isTauri } from '@tauri-apps/api/core'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { ArrowLeft, ArrowRight, ChevronRight } from 'lucide-react'
import { cn } from '@/lib/utils'

const isMac = /Mac/i.test(navigator.platform)
/** The frameless window, or null outside the desktop shell; never throws so the UI can't blank out. */
const appWindow = () => { if (!isTauri()) return null; try { return getCurrentWindow() } catch { return null } }
const mod = isMac ? '⌘' : 'Ctrl'

/** Three even strokes, as in Claude's app menu button. */
export function MenuIcon() {
  return <svg width="18" height="18" viewBox="0 0 20 20" fill="none" aria-hidden><path d="M3.5 5.5h13M3.5 10h13M3.5 14.5h13" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" /></svg>
}

/** Rounded panel with a left rail, as in Claude's sidebar toggle. The rail fills when the sidebar is open. */
export function SidebarIcon({ open }: { open: boolean }) {
  return <svg width="18" height="18" viewBox="0 0 20 20" fill="none" aria-hidden>
    <rect x="2.75" y="3.75" width="14.5" height="12.5" rx="2.75" stroke="currentColor" strokeWidth="1.5" />
    <path d="M7.75 4v12" stroke="currentColor" strokeWidth="1.5" />
    {open && <rect x="3.5" y="4.5" width="3.5" height="11" rx="1.75" fill="currentColor" opacity=".28" />}
  </svg>
}

export interface AppMenuItem { label: string; shortcut?: string; disabled?: boolean; onSelect: () => void }
export interface AppMenuSection { label: string; items: (AppMenuItem | 'separator')[] }

/** The ☰ application menu: File / Edit / View / Help with hover-opened submenus. */
export function AppMenu({ sections }: { sections: AppMenuSection[] }) {
  const [open, setOpen] = useState(false)
  const [active, setActive] = useState<string | null>(null)
  const root = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!open) return
    const close = (event: PointerEvent) => { if (!root.current?.contains(event.target as Node)) { setOpen(false); setActive(null) } }
    const escape = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.stopPropagation(); setOpen(false); setActive(null) } }
    window.addEventListener('pointerdown', close)
    window.addEventListener('keydown', escape, true)
    return () => { window.removeEventListener('pointerdown', close); window.removeEventListener('keydown', escape, true) }
  }, [open])
  const choose = (item: AppMenuItem) => { setOpen(false); setActive(null); item.onSelect() }
  return <div className="app-menu" ref={root}>
    <button type="button" className={cn('titlebar-button', open && 'pressed')} aria-label="Application menu" aria-haspopup="menu" aria-expanded={open} title="Menu" onClick={() => { setOpen(value => !value); setActive(null) }}><MenuIcon /></button>
    {open && <div className="app-menu-panel" role="menu">
      {sections.map(section => <div key={section.label} className="app-menu-group" onMouseEnter={() => setActive(section.label)}>
        <button type="button" role="menuitem" aria-haspopup="menu" aria-expanded={active === section.label} className={cn('app-menu-row', active === section.label && 'active')} onClick={() => setActive(section.label)} onKeyDown={event => { if (event.key === 'ArrowRight') setActive(section.label) }}>
          <span>{section.label}</span><ChevronRight size={14} />
        </button>
        {active === section.label && <div className="app-menu-sub" role="menu">
          {section.items.map((item, index) => item === 'separator'
            ? <div key={`s${index}`} className="app-menu-separator" />
            : <button key={item.label} type="button" role="menuitem" className="app-menu-row" disabled={item.disabled} onClick={() => choose(item)}><span>{item.label}</span>{item.shortcut && <kbd>{item.shortcut.replace('Mod', mod)}</kbd>}</button>)}
        </div>}
      </div>)}
    </div>}
  </div>
}

export interface TitleBarLeadingProps {
  sections: AppMenuSection[]
  sidebarOpen: boolean
  onToggleSidebar: () => void
  onPeek?: () => void
  onUnpeek?: () => void
  canBack: boolean
  canForward: boolean
  onBack: () => void
  onForward: () => void
}

/** Claude's leading title-bar cluster: app menu, sidebar toggle, then back and forward. */
export function TitleBarLeading(props: TitleBarLeadingProps): ReactNode {
  const { sidebarOpen } = props
  return <div className="titlebar-leading">
    <AppMenu sections={props.sections} />
    <button type="button" className="titlebar-button" onClick={props.onToggleSidebar} onMouseEnter={props.onPeek} onMouseLeave={props.onUnpeek} aria-label={sidebarOpen ? 'Close sidebar' : 'Open sidebar'} aria-pressed={sidebarOpen} title={`${sidebarOpen ? 'Close' : 'Open'} sidebar (${mod} B)`}><SidebarIcon open={sidebarOpen} /></button>
    <button type="button" className="titlebar-button" onClick={props.onBack} disabled={!props.canBack} aria-label="Back" title="Back (Alt ←)"><ArrowLeft size={17} strokeWidth={1.75} /></button>
    <button type="button" className="titlebar-button" onClick={props.onForward} disabled={!props.canForward} aria-label="Forward" title="Forward (Alt →)"><ArrowRight size={17} strokeWidth={1.75} /></button>
  </div>
}

/** Windows-style caption buttons for the frameless window. */
export function WindowControls() {
  const [maximized, setMaximized] = useState(false)
  const desktop = isTauri()
  useEffect(() => {
    const current = desktop ? appWindow() : null
    if (!current) return
    const sync = () => void current.isMaximized().then(setMaximized).catch(() => undefined)
    sync()
    const unlisten = current.onResized(sync).catch(() => () => undefined)
    return () => { void unlisten.then(stop => stop()) }
  }, [desktop])
  if (isMac) return null
  const run = (action: 'minimize' | 'toggleMaximize' | 'close') => { void appWindow()?.[action]().catch(() => undefined) }
  return <div className="window-controls">
    <button type="button" className="window-control" aria-label="Minimize" title="Minimize" onClick={() => run('minimize')}><svg width="10" height="10" viewBox="0 0 10 10" aria-hidden><path d="M0 5.5h10" stroke="currentColor" /></svg></button>
    <button type="button" className="window-control" aria-label={maximized ? 'Restore' : 'Maximize'} title={maximized ? 'Restore' : 'Maximize'} onClick={() => run('toggleMaximize')}>
      {maximized
        ? <svg width="10" height="10" viewBox="0 0 10 10" fill="none" aria-hidden><rect x=".5" y="2.5" width="7" height="7" rx="1" stroke="currentColor" /><path d="M2.5 2.5V1.5a1 1 0 0 1 1-1h5a1 1 0 0 1 1 1v5a1 1 0 0 1-1 1h-1" stroke="currentColor" /></svg>
        : <svg width="10" height="10" viewBox="0 0 10 10" fill="none" aria-hidden><rect x=".5" y=".5" width="9" height="9" rx="1" stroke="currentColor" /></svg>}
    </button>
    <button type="button" className="window-control close" aria-label="Close" title="Close" onClick={() => run('close')}><svg width="10" height="10" viewBox="0 0 10 10" aria-hidden><path d="M.5.5l9 9M9.5.5l-9 9" stroke="currentColor" /></svg></button>
  </div>
}
