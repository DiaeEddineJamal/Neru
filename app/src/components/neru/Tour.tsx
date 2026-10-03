import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { ArrowLeft, ArrowRight, X } from 'lucide-react'
import { EASE_OUT, SPRING_LAYOUT } from '@/lib/ease'
import { cn } from '@/lib/utils'
import './Tour.css'

/** One stop on a walkthrough. `target` is a `data-tour` name; without one the card sits centred. */
export type TourStep = {
  target?: string
  title: string
  body: ReactNode
  /** Runs before the step is measured, e.g. to open the panel the step points at. */
  enter?: () => void
  side?: 'right' | 'left' | 'bottom' | 'top'
}

type Rect = { top: number; left: number; width: number; height: number }

const PAD = 6
const GAP = 14
const CARD_W = 340
const MARGIN = 12

/** A `data-tour` name, or a CSS selector when it starts with `.`, `#` or `[`; `a | b` falls back to b.
 * Hidden elements count as missing. */
const find = (target?: string): HTMLElement | null => {
  for (const one of target?.split(' | ') ?? []) {
    const element = document.querySelector<HTMLElement>(/^[.#[]/.test(one) ? one : `[data-tour="${one}"]`)
    if (element && element.getClientRects().length > 0) return element
  }
  return null
}

function measure(element: HTMLElement): Rect {
  const box = element.getBoundingClientRect()
  return { top: box.top - PAD, left: box.left - PAD, width: box.width + PAD * 2, height: box.height + PAD * 2 }
}

/** Puts the card beside the spotlight on the side with the most room, kept inside the window. */
function place(spot: Rect | null, cardH: number, side?: TourStep['side']) {
  const vw = window.innerWidth, vh = window.innerHeight
  const width = Math.min(CARD_W, vw - MARGIN * 2)
  if (!spot) return { left: (vw - width) / 2, top: Math.max(MARGIN, (vh - cardH) / 2), width }
  const room = { right: vw - (spot.left + spot.width), left: spot.left, bottom: vh - (spot.top + spot.height), top: spot.top }
  const fits = (s: keyof typeof room) => s === 'right' || s === 'left' ? room[s] >= width + GAP + MARGIN : room[s] >= cardH + GAP + MARGIN
  const order: (keyof typeof room)[] = side ? [side, 'right', 'bottom', 'left', 'top'] : ['right', 'bottom', 'left', 'top']
  const chosen = order.find(fits) ?? (Object.keys(room) as (keyof typeof room)[]).sort((a, b) => room[b] - room[a])[0]
  let left = chosen === 'right' ? spot.left + spot.width + GAP
    : chosen === 'left' ? spot.left - width - GAP
    : spot.left + spot.width / 2 - width / 2
  let top = chosen === 'bottom' ? spot.top + spot.height + GAP
    : chosen === 'top' ? spot.top - cardH - GAP
    : spot.top + spot.height / 2 - cardH / 2
  left = Math.min(Math.max(MARGIN, left), vw - width - MARGIN)
  top = Math.min(Math.max(MARGIN, top), vh - cardH - MARGIN)
  return { left, top, width }
}

/**
 * A spotlight walkthrough: dims the window, cuts a hole around one element, explains it in a card
 * beside it, then glides to the next. Steps whose element is not on screen are skipped.
 * Keys: → or Enter next, ← back, Esc skip.
 */
export function Tour({ steps, onClose, label = 'Walkthrough' }: { steps: TourStep[]; onClose: (finished: boolean) => void; label?: string }) {
  const reduce = useReducedMotion() ?? false
  const [index, setIndex] = useState(0)
  const [spot, setSpot] = useState<Rect | null>(null)
  const [cardH, setCardH] = useState(180)
  const cardRef = useRef<HTMLDivElement>(null)
  const nextRef = useRef<HTMLButtonElement>(null)
  const step = steps[index]
  const last = index === steps.length - 1

  // Steps whose element is missing (a panel that is closed, a task with no worktrees) are passed over.
  const go = useCallback((from: number, direction: 1 | -1) => {
    let next = from
    while (next >= 0 && next < steps.length) {
      steps[next].enter?.()
      // A step that opens something first is trusted to render its element.
      if (!steps[next].target || steps[next].enter || find(steps[next].target)) break
      next += direction
    }
    if (next >= steps.length) { onClose(true); return }
    if (next < 0) return
    setIndex(next)
  }, [steps, onClose])

  useEffect(() => { go(0, 1) }, []) // eslint-disable-line react-hooks/exhaustive-deps

  // Measure after the step's `enter` has rendered, and follow the element as the window changes.
  useLayoutEffect(() => {
    let frame = 0
    const element = find(step?.target)
    const update = () => { cancelAnimationFrame(frame); frame = requestAnimationFrame(() => { const el = find(step?.target); setSpot(el ? measure(el) : null) }) }
    if (element) element.scrollIntoView({ block: 'nearest', inline: 'nearest' })
    update()
    const late = window.setTimeout(update, 180)
    const observer = new ResizeObserver(update)
    if (element) observer.observe(element)
    window.addEventListener('resize', update)
    window.addEventListener('scroll', update, true)
    return () => { cancelAnimationFrame(frame); window.clearTimeout(late); observer.disconnect(); window.removeEventListener('resize', update); window.removeEventListener('scroll', update, true) }
  }, [step])

  useLayoutEffect(() => { if (cardRef.current) setCardH(cardRef.current.offsetHeight) }, [index, spot])
  useEffect(() => { nextRef.current?.focus({ preventScroll: true }) }, [index])

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); onClose(false) }
      else if (event.key === 'ArrowRight') { event.preventDefault(); go(index + 1, 1) }
      else if (event.key === 'ArrowLeft') { event.preventDefault(); go(index - 1, -1) }
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [index, go, onClose])

  if (!step) return null
  const position = place(spot, cardH, step.side)
  const glide = reduce ? { duration: 0 } : SPRING_LAYOUT

  return createPortal(<div className="tour" role="dialog" aria-modal="true" aria-label={label}>
    {/* The scrim is the spotlight's own shadow, so the hole and the dimming always move together. */}
    {spot
      ? <motion.div key="spot" className="tour-spot" initial={false} animate={spot} transition={glide} />
      : <motion.div key="scrim" className="tour-scrim" initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ duration: reduce ? 0 : 0.2 }} />}
    <motion.div ref={cardRef} className="tour-card" initial={reduce ? false : { opacity: 0, scale: 0.96 }} animate={{ opacity: 1, scale: 1, left: position.left, top: position.top, width: position.width }} transition={glide}>
      <AnimatePresence mode="wait" initial={false}>
        <motion.div key={index} initial={reduce ? false : { opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }} exit={reduce ? undefined : { opacity: 0, y: -4 }} transition={{ duration: 0.16, ease: EASE_OUT }}>
          <p className="tour-eyebrow">{index + 1} of {steps.length}</p>
          <h2 className="tour-title">{step.title}</h2>
          <div className="tour-body">{step.body}</div>
        </motion.div>
      </AnimatePresence>
      <div className="tour-progress" aria-hidden>{steps.map((_, i) => <i key={i} className={cn(i === index && 'on', i < index && 'done')} />)}</div>
      <footer className="tour-actions">
        {!last && <button type="button" className="tour-skip" onClick={() => onClose(false)}>Skip tour</button>}
        <span className="tour-spacer" />
        {index > 0 && <button type="button" className="button subtle small" onClick={() => go(index - 1, -1)} aria-label="Back"><ArrowLeft size={14} /></button>}
        <button ref={nextRef} type="button" className="button primary small" onClick={() => go(index + 1, 1)}>{last ? 'Done' : index === 0 && !step.target ? 'Show me around' : 'Next'}{!last && <ArrowRight size={14} />}</button>
      </footer>
      <button type="button" className="tour-close" onClick={() => onClose(false)} aria-label="Close the walkthrough"><X size={14} /></button>
    </motion.div>
  </div>, document.body)
}

/** Shows a tour once per `key` (stored when it is finished or skipped); `start` replays it. */
export function useTour(key: string) {
  const seen = () => { try { return localStorage.getItem(key) === 'done' } catch { return true } }
  const [open, setOpen] = useState(false)
  const close = useCallback(() => { try { localStorage.setItem(key, 'done') } catch { /* storage unavailable */ } setOpen(false) }, [key])
  return { open, start: useCallback(() => setOpen(true), []), close, seen }
}
