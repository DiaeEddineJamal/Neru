import { Children, Fragment, isValidElement, useEffect, useId, useLayoutEffect, useRef, useState, type CSSProperties, type ReactElement, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { Check, ChevronDown } from '@/components/icons'
import { cn } from '@/lib/utils'
import './Select.css'

// Neru's dropdown: a drop-in for <select> that takes the same <option> and <optgroup> children and calls
// onChange with { target: { value } }, but opens a list drawn in the app's own style instead of the
// system's. Keyboard: arrows, Home/End, Enter or Space, Escape, and typing the start of an option.

type Item = { value: string; label: ReactNode; text: string; disabled: boolean; title?: string; group?: string }
type OptionProps = { value?: string | number; disabled?: boolean; title?: string; children?: ReactNode }

const textOf = (node: ReactNode): string => {
  if (node === null || node === undefined || typeof node === 'boolean') return ''
  if (typeof node === 'string' || typeof node === 'number') return String(node)
  if (Array.isArray(node)) return node.map(textOf).join('')
  if (isValidElement<{ children?: ReactNode }>(node)) return textOf(node.props.children)
  return ''
}

/** The options inside the children, through fragments, arrays and optgroups. */
function collect(children: ReactNode, group?: string, out: Item[] = []): Item[] {
  Children.forEach(children, child => {
    if (!isValidElement(child)) return
    const element = child as ReactElement<OptionProps & { label?: string }>
    if (element.type === Fragment) collect(element.props.children, group, out)
    else if (element.type === 'optgroup') collect(element.props.children, element.props.label, out)
    else if (element.type === 'option') {
      const text = textOf(element.props.children)
      out.push({ value: String(element.props.value ?? text), label: element.props.children, text, disabled: Boolean(element.props.disabled), title: element.props.title, group })
    }
  })
  return out
}

export type SelectProps = {
  value: string | number
  onChange?: (event: { target: { value: string } }) => void
  children: ReactNode
  disabled?: boolean
  className?: string
  id?: string
  title?: string
  style?: CSSProperties
  'aria-label'?: string
}

export function Select({ value, onChange, children, disabled, className, id, title, style, 'aria-label': ariaLabel }: SelectProps) {
  const items = collect(children)
  const current = items.find(item => item.value === String(value))
  const [open, setOpen] = useState(false)
  const [active, setActive] = useState(0)
  const [place, setPlace] = useState<CSSProperties>({})
  const trigger = useRef<HTMLButtonElement>(null)
  const list = useRef<HTMLDivElement>(null)
  const typed = useRef({ text: '', at: 0 })
  const listId = useId()

  const close = (focus = true) => { setOpen(false); if (focus) trigger.current?.focus() }
  const choose = (item: Item | undefined) => {
    if (!item || item.disabled) return
    if (item.value !== String(value)) onChange?.({ target: { value: item.value } })
    close()
  }
  const openList = () => {
    if (disabled || !items.length) return
    setActive(Math.max(0, items.findIndex(item => item.value === String(value))))
    setOpen(true)
  }

  // The list opens under the trigger, or above it when there is more room there, never past the window.
  useLayoutEffect(() => {
    if (!open) return
    const measure = () => {
      const box = trigger.current?.getBoundingClientRect()
      if (!box) return
      const below = window.innerHeight - box.bottom - 8, above = box.top - 8
      const up = below < 220 && above > below
      const height = Math.min(320, Math.max(120, up ? above : below))
      const width = Math.max(box.width, 180)
      const left = Math.min(Math.max(8, box.left), window.innerWidth - width - 8)
      setPlace(up ? { left, width, bottom: window.innerHeight - box.top + 4, maxHeight: height } : { left, width, top: box.bottom + 4, maxHeight: height })
    }
    measure()
    window.addEventListener('resize', measure)
    return () => window.removeEventListener('resize', measure)
  }, [open])
  useEffect(() => {
    if (!open) return
    const away = (event: MouseEvent) => { if (!list.current?.contains(event.target as Node) && !trigger.current?.contains(event.target as Node)) close(false) }
    const scrolled = (event: Event) => { if (!list.current?.contains(event.target as Node)) close(false) }
    document.addEventListener('mousedown', away)
    window.addEventListener('scroll', scrolled, true)
    return () => { document.removeEventListener('mousedown', away); window.removeEventListener('scroll', scrolled, true) }
  }, [open])
  useEffect(() => { if (open) list.current?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: 'nearest' }) }, [open, active])

  const move = (from: number, step: number) => {
    for (let index = from + step; index >= 0 && index < items.length; index += step) if (!items[index].disabled) return index
    return from
  }
  const onKeyDown = (event: React.KeyboardEvent) => {
    if (disabled) return
    if (!open) {
      if (['ArrowDown', 'ArrowUp', 'Enter', ' '].includes(event.key)) { event.preventDefault(); openList() }
      return
    }
    switch (event.key) {
      case 'ArrowDown': event.preventDefault(); setActive(index => move(index, 1)); return
      case 'ArrowUp': event.preventDefault(); setActive(index => move(index, -1)); return
      case 'Home': event.preventDefault(); setActive(move(-1, 1)); return
      case 'End': event.preventDefault(); setActive(move(items.length, -1)); return
      case 'Enter': case ' ': event.preventDefault(); choose(items[active]); return
      case 'Escape': event.preventDefault(); event.stopPropagation(); close(); return
      case 'Tab': close(false); return
    }
    if (event.key.length === 1) {
      const now = Date.now()
      typed.current = { text: now - typed.current.at < 700 ? typed.current.text + event.key.toLowerCase() : event.key.toLowerCase(), at: now }
      const found = items.findIndex(item => !item.disabled && item.text.toLowerCase().startsWith(typed.current.text))
      if (found >= 0) setActive(found)
    }
  }

  return <>
    <button ref={trigger} type="button" id={id} className={cn('nselect', open && 'is-open', className)} style={style} disabled={disabled} title={title ?? current?.title}
      aria-haspopup="listbox" aria-expanded={open} aria-controls={open ? listId : undefined} aria-label={ariaLabel}
      onClick={() => (open ? close() : openList())} onKeyDown={onKeyDown}>
      <span className="nselect-value">{current?.label ?? ' '}</span>
      <ChevronDown size={14} className="nselect-caret" aria-hidden />
    </button>
    {open && createPortal(<div ref={list} id={listId} className="nselect-menu" role="listbox" aria-label={ariaLabel} style={place} onKeyDown={onKeyDown}>
      {items.map((item, index) => {
        const header = item.group && item.group !== items[index - 1]?.group ? <div key={`group-${item.group}`} className="nselect-group" role="presentation">{item.group}</div> : null
        const selected = item.value === String(value)
        return <Fragment key={`${item.value}-${index}`}>
          {header}
          <div data-index={index} role="option" aria-selected={selected} aria-disabled={item.disabled || undefined} title={item.title}
            className={cn('nselect-option', selected && 'is-selected', index === active && 'is-active', item.disabled && 'is-disabled')}
            onMouseEnter={() => !item.disabled && setActive(index)} onMouseDown={event => event.preventDefault()} onClick={() => choose(item)}>
            <span>{item.label}</span>{selected && <Check size={14} className="nselect-check" aria-hidden />}
          </div>
        </Fragment>
      })}
    </div>, document.body)}
  </>
}
