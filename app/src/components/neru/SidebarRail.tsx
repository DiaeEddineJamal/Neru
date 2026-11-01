import { useRef, type KeyboardEvent, type ReactNode } from 'react'
import { Clock, Folders, MessageSquare, Search, Settings2, SquareKanban, SquarePen, Users, type LucideIcon } from '@/components/icons'
import { SidebarIcon } from './TitleBar'
import { t } from '../../i18n'
import { cn } from '@/lib/utils'
import type { Section } from '../../types'
import './SidebarRail.css'

const isMac = typeof navigator !== 'undefined' && /Mac/i.test(navigator.platform)
const mod = isMac ? '⌘' : 'Ctrl'

export interface SidebarRailProps {
  surface: 'chat' | 'code'
  section: Section
  /** New chat / session is unavailable (Code without a project, or while busy). */
  newDisabled: boolean
  /** Code tools that need an open project. */
  hasProject: boolean
  queued: number
  /** The user's name, for the profile initial. */
  profileName: string
  onToggle: () => void
  onNew: () => void
  onSearch: () => void
  /** Chat: shows chat history; Code: the session board. */
  onHistory: () => void
  onProjects: () => void
  onTeam: () => void
  onTasks: () => void
  onSettings: () => void
  onProfile: () => void
}

interface RailItem { key: string; label: string; tip?: string; icon: LucideIcon; active?: boolean; disabled?: boolean; badge?: number; onClick: () => void }

/**
 * The slim icon rail shown while the sidebar is collapsed, as in Claude's desktop app: the sidebar
 * toggle, new, search and the main places, with settings and the profile at the bottom. Each icon
 * has a tooltip; the active place uses the filled icon. Arrow keys move between items.
 */
export function SidebarRail(props: SidebarRailProps) {
  const { surface, section } = props
  const nav = useRef<HTMLElement>(null)
  const chat = surface === 'chat'
  const items: RailItem[] = [
    { key: 'new', label: chat ? t('New chat') : t('New session'), tip: `${chat ? t('New chat') : t('New session')} (${mod} N)`, icon: SquarePen, disabled: props.newDisabled, onClick: props.onNew },
    { key: 'search', label: t('Search'), icon: Search, active: !chat && section === 'search', disabled: !chat && !props.hasProject, onClick: props.onSearch },
    chat
      ? { key: 'history', label: t('Chats'), icon: MessageSquare, active: section === 'home', onClick: props.onHistory }
      : { key: 'history', label: t('Sessions'), icon: SquareKanban, active: section === 'board', disabled: !props.hasProject, onClick: props.onHistory },
    ...(chat ? [{ key: 'projects', label: t('Projects'), icon: Folders, active: section === 'projects', onClick: props.onProjects }] : []),
    { key: 'team', label: t('Team'), icon: Users, active: section === 'team', onClick: props.onTeam },
    { key: 'tasks', label: t('Tasks'), icon: Clock, badge: props.queued, onClick: props.onTasks },
  ]
  const initial = props.profileName.trim().charAt(0).toUpperCase()
  const move = (event: KeyboardEvent<HTMLElement>) => {
    if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return
    const buttons = Array.from(nav.current?.querySelectorAll<HTMLButtonElement>('button:not(:disabled)') ?? [])
    const index = buttons.indexOf(document.activeElement as HTMLButtonElement)
    const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : (index + (event.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length
    buttons[next]?.focus()
    event.preventDefault()
  }
  return <nav ref={nav} className="sidebar-rail" aria-label={t('Collapsed sidebar')} onKeyDown={move}>
    <RailButton label={t('Open sidebar')} tip={`${t('Open sidebar')} (${mod} B)`} onClick={props.onToggle}><SidebarIcon open={false} /></RailButton>
    <div className="rail-group">
      {items.map(item => <RailButton key={item.key} label={item.label} tip={item.tip} active={item.active} disabled={item.disabled} onClick={item.onClick}>
        <item.icon size={18} weight={item.active ? 'fill' : 'regular'} aria-hidden />
        {Boolean(item.badge) && <b className="rail-badge" aria-label={t('{n} queued', { n: item.badge ?? 0 })}>{(item.badge ?? 0) > 9 ? '9+' : item.badge}</b>}
      </RailButton>)}
    </div>
    <div className="rail-group rail-bottom">
      <RailButton label={t('Settings')} tip={`${t('Settings')} (${mod} ,)`} active={section === 'settings'} onClick={props.onSettings}><Settings2 size={18} weight={section === 'settings' ? 'fill' : 'regular'} aria-hidden /></RailButton>
      <RailButton label={props.profileName.trim() ? t('Profile: {name}', { name: props.profileName.trim() }) : t('Profile')} onClick={props.onProfile}>
        <span className="rail-avatar" aria-hidden>{initial || <Users size={14} />}</span>
      </RailButton>
    </div>
  </nav>
}

function RailButton({ label, tip, active, disabled, onClick, children }: { label: string; tip?: string; active?: boolean; disabled?: boolean; onClick: () => void; children: ReactNode }) {
  return <button type="button" className={cn('rail-button', active && 'active')} aria-label={label} aria-current={active ? 'page' : undefined} data-tip={tip ?? label} disabled={disabled} onClick={onClick}>{children}</button>
}
