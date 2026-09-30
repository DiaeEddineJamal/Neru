import { createContext, useCallback, useContext, useEffect, useLayoutEffect, useRef, useState, type MouseEvent as ReactMouseEvent, type ReactNode } from 'react'
import { motion, useReducedMotion } from 'motion/react'
import { cn } from '@/lib/utils'

export type MenuItem =
  | { label: string; icon?: ReactNode; shortcut?: string; danger?: boolean; disabled?: boolean; onSelect: () => void }
  | 'separator'

type Open = (event: ReactMouseEvent | MouseEvent, items: MenuItem[]) => void

const MenuContext = createContext<Open>(() => undefined)

/** Opens the app's right-click menu with `items` at the pointer. */
export const useContextMenu = () => useContext(MenuContext)

/**
 * One right-click menu for the whole window: placed at the pointer and kept on screen, arrow keys
 * and Enter to choose, Esc or a click elsewhere to close. Each place supplies its own items.
 */
export function ContextMenuProvider({ children }: { children: ReactNode }) {
  const reduce = useReducedMotion() ?? false
  const [menu, setMenu] = useState<{ x: number; y: number; items: MenuItem[] } | null>(null)
  const [active, setActive] = useState(-1)
  const [place, setPlace] = useState<{ left: number; top: number } | null>(null)
  const panel = useRef<HTMLDivElement>(null)

  const open = useCallback<Open>((event, items) => {
    event.preventDefault()
    event.stopPropagation()
    if (items.length === 0) return
    setPlace(null)
    setActive(-1)
    setMenu({ x: event.clientX, y: event.clientY, items })
  }, [])
  const close = useCallback(() => setMenu(null), [])

  // Keep the menu inside the window, flipping left or up near the edges.
  useLayoutEffect(() => {
    if (!menu || !panel.current) return
    const { width, height } = panel.current.getBoundingClientRect()
    const left = menu.x + width > window.innerWidth - 8 ? Math.max(8, menu.x - width) : menu.x
    const top = menu.y + height > window.innerHeight - 8 ? Math.max(8, menu.y - height) : menu.y
    setPlace({ left, top })
    panel.current.focus({ preventScroll: true })
  }, [menu])

  useEffect(() => {
    if (!menu) return
    const choices = menu.items.map((item, index) => ({ item, index })).filter(({ item }) => item !== 'separator' && !item.disabled).map(({ index }) => index)
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') { event.preventDefault(); close() }
      if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
        event.preventDefault()
        const at = choices.indexOf(active)
        const next = event.key === 'ArrowDown' ? choices[(at + 1) % choices.length] : choices[(at - 1 + choices.length) % choices.length]
        setActive(next ?? -1)
      }
      if (event.key === 'Enter' && active >= 0) {
        event.preventDefault()
        const item = menu.items[active]
        if (item !== 'separator') { close(); item.onSelect() }
      }
    }
    const onPointer = (event: PointerEvent) => { if (!panel.current?.contains(event.target as Node)) close() }
    window.addEventListener('keydown', onKey, true)
    window.addEventListener('pointerdown', onPointer, true)
    window.addEventListener('blur', close)
    window.addEventListener('resize', close)
    window.addEventListener('wheel', close, { passive: true })
    return () => {
      window.removeEventListener('keydown', onKey, true)
      window.removeEventListener('pointerdown', onPointer, true)
      window.removeEventListener('blur', close)
      window.removeEventListener('resize', close)
      window.removeEventListener('wheel', close)
    }
  }, [menu, active, close])

  return <MenuContext.Provider value={open}>
    {children}
    {menu && <motion.div ref={panel} className="context-menu" role="menu" tabIndex={-1} onContextMenu={event => event.preventDefault()}
      style={{ left: place?.left ?? menu.x, top: place?.top ?? menu.y, visibility: place ? 'visible' : 'hidden' }}
      initial={reduce ? false : { opacity: 0, scale: 0.97, y: -3 }} animate={{ opacity: 1, scale: 1, y: 0 }} transition={{ duration: 0.12, ease: 'easeOut' }}>
      {menu.items.map((item, index) => item === 'separator'
        ? <div key={index} className="context-menu-separator" role="separator" />
        : <button key={index} type="button" role="menuitem" disabled={item.disabled} className={cn('context-menu-item', item.danger && 'danger', index === active && 'active')}
          onMouseEnter={() => setActive(index)} onClick={() => { close(); item.onSelect() }}>
          <span className="context-menu-icon" aria-hidden>{item.icon}</span>
          <span className="context-menu-label">{item.label}</span>
          {item.shortcut && <kbd>{item.shortcut}</kbd>}
        </button>)}
    </motion.div>}
  </MenuContext.Provider>
}
