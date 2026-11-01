import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from 'react'
import { CheckCircle2, ChevronDown, CircleAlert, Info, TriangleAlert, X } from '@/components/icons'
import { cn } from '@/lib/utils'
import { dismiss, useToasts, type Toast } from '@/lib/toast'
import { t } from '../../i18n'
import './Toaster.css'

const ICONS = { info: Info, success: CheckCircle2, warning: TriangleAlert, error: CircleAlert }

function Item({ toast, index, total, expanded, onPause }: { toast: Toast; index: number; total: number; expanded: boolean; onPause: (paused: boolean) => void }) {
  const [details, setDetails] = useState(false)
  const [leaving, setLeaving] = useState(false)
  const remaining = useRef(toast.duration)
  const started = useRef(0)
  const timer = useRef(0)
  const Icon = ICONS[toast.kind]
  const close = () => { setLeaving(true); window.setTimeout(() => dismiss(toast.id), 140) }

  // The clock runs only while nobody reads the stack; hovering or focusing it pauses every toast.
  const run = () => {
    if (remaining.current === null) return
    started.current = performance.now()
    timer.current = window.setTimeout(close, remaining.current)
  }
  const pause = () => {
    if (remaining.current === null || !timer.current) return
    window.clearTimeout(timer.current)
    timer.current = 0
    remaining.current = Math.max(800, remaining.current - (performance.now() - started.current))
  }
  useEffect(() => {
    remaining.current = toast.duration
    window.clearTimeout(timer.current)
    timer.current = 0
    if (!expanded) run()
    return () => window.clearTimeout(timer.current)
    // A merged repeat (count) or a new duration restarts the clock.
  }, [toast.duration, toast.count, toast.title]) // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => { if (expanded) pause(); else if (!timer.current) run() }, [expanded]) // eslint-disable-line react-hooks/exhaustive-deps

  // Newest on top of the pile; older ones peek out behind it until the stack is hovered.
  const depth = total - 1 - index
  const style = expanded ? undefined : { '--depth': depth } as CSSProperties
  return <li className={cn('toast', `is-${toast.kind}`, leaving && 'is-leaving', !expanded && depth > 0 && 'is-behind')} style={style}
    role={toast.kind === 'error' ? 'alert' : 'status'} aria-live={toast.kind === 'error' ? 'assertive' : 'polite'} aria-atomic="true"
    onKeyDown={event => { if (event.key === 'Escape') { event.stopPropagation(); close() } }}
    onFocus={() => onPause(true)} onBlur={event => { if (!event.currentTarget.contains(event.relatedTarget as Node)) onPause(false) }}>
    <Icon size={18} weight="fill" className="toast-icon" aria-hidden />
    <div className="toast-body">
      <strong>{toast.title}{toast.count > 1 && <span className="toast-count" aria-label={t('{n} times', { n: toast.count })}>×{toast.count}</span>}</strong>
      {toast.description && <p>{toast.description}</p>}
      {(toast.action || toast.detail) && <div className="toast-actions">
        {toast.action && <button type="button" className="toast-action" onClick={() => { toast.action!.run(); close() }}>{toast.action.label}</button>}
        {toast.detail && <button type="button" className="toast-link" aria-expanded={details} onClick={() => setDetails(value => !value)}>{t('Details')}<ChevronDown size={12} className={cn(details && 'open')} /></button>}
      </div>}
      {details && toast.detail && <pre className="toast-detail">{toast.detail}</pre>}
    </div>
    <button type="button" className="toast-close" onClick={close} aria-label={t('Dismiss')} title={t('Dismiss (Esc)')}><X size={14} /></button>
  </li>
}

/** Where transient notifications appear: bottom-right, above the composer's reach, newest in front. */
export function Toaster({ className, style }: { className?: string; style?: CSSProperties }) {
  const toasts = useToasts()
  const [hovered, setHovered] = useState(false)
  const [focused, setFocused] = useState(false)
  const expanded = hovered || focused
  // Never over the composer: when the corner would cover it, the stack sits just above it instead.
  const [lift, setLift] = useState<number | null>(null)
  const root = useRef<HTMLElement>(null)
  useLayoutEffect(() => {
    if (!toasts.length) return
    const place = () => {
      const box = root.current?.getBoundingClientRect()
      const composer = document.querySelector('.composer')?.getBoundingClientRect()
      if (!box || !composer || composer.height === 0) { setLift(null); return }
      const overlaps = box.left < composer.right && box.right > composer.left && composer.bottom > window.innerHeight - 260
      setLift(overlaps ? window.innerHeight - composer.top + 12 : null)
    }
    place()
    window.addEventListener('resize', place)
    return () => window.removeEventListener('resize', place)
  }, [toasts.length])
  if (!toasts.length) return null
  return <section ref={root} className={cn('toaster', expanded && 'is-expanded', className)} style={lift === null ? style : { ...style, bottom: Math.max(lift, Number(style?.bottom ?? 0)) }} aria-label={t('Notifications')}
    onMouseEnter={() => setHovered(true)} onMouseLeave={() => setHovered(false)}>
    <ol>
      {toasts.map((toast, index) => <Item key={toast.id} toast={toast} index={index} total={toasts.length} expanded={expanded} onPause={setFocused} />)}
    </ol>
  </section>
}
