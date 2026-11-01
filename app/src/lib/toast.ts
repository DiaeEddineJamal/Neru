import { useSyncExternalStore } from 'react'
import { describeError, type ErrorAction } from './friendlyError'

// Transient notifications (Toaster.tsx). Errors from a reply stay in the conversation and problems with
// the model stay by the composer; this is for everything else: an action finished, something failed
// in the background, an update is ready. Repeats are merged, at most a few show at once, and a toast
// waits while it is hovered or focused.

export type ToastKind = 'info' | 'success' | 'warning' | 'error'
export type ToastAction = { label: string; run: () => void }

export interface Toast {
  id: number
  kind: ToastKind
  title: string
  description?: string
  /** Raw error text, behind a Details toggle. */
  detail?: string
  action?: ToastAction
  /** Milliseconds on screen; null stays until dismissed. */
  duration: number | null
  /** How many identical toasts were merged into this one. */
  count: number
  /** Replaces a toast with the same key instead of stacking (progress, "copied"). */
  key?: string
}

type Options = { description?: string; action?: ToastAction; duration?: number | null; key?: string; detail?: string }

/** Long enough to read the title and description at a calm pace, never under four seconds. */
export function readingTime(title: string, description = '') {
  const words = `${title} ${description}`.trim().split(/\s+/).filter(Boolean).length
  return Math.min(12_000, Math.max(4_000, 1_500 + words * 300))
}

const MAX = 3
let toasts: Toast[] = []
let nextId = 1
const listeners = new Set<() => void>()
const emit = () => listeners.forEach(listener => listener())

function push(kind: ToastKind, title: string, options: Options = {}) {
  const text = title.trim()
  if (!text) return 0
  const same = toasts.find(toast => (options.key && toast.key === options.key) || (!options.key && toast.kind === kind && toast.title === text && toast.description === options.description))
  // Errors wait for the reader; the rest leave after their reading time.
  const duration = options.duration !== undefined ? options.duration : kind === 'error' ? null : readingTime(text, options.description)
  if (same) {
    toasts = toasts.map(toast => toast.id === same.id ? { ...toast, title: text, description: options.description, action: options.action, detail: options.detail, duration, count: options.key ? 1 : toast.count + 1 } : toast)
  } else {
    toasts = [...toasts, { id: nextId++, kind, title: text, description: options.description, detail: options.detail, action: options.action, duration, count: 1, key: options.key }]
    // The oldest non-error leaves first when too many pile up.
    while (toasts.length > MAX) {
      const drop = toasts.find(toast => toast.kind !== 'error') ?? toasts[0]
      toasts = toasts.filter(toast => toast !== drop)
    }
  }
  emit()
  return same?.id ?? nextId - 1
}

export function dismiss(id: number) {
  toasts = toasts.filter(toast => toast.id !== id)
  emit()
}

export function dismissAll(kind?: ToastKind) {
  toasts = kind ? toasts.filter(toast => toast.kind !== kind) : []
  emit()
}

export const toast = {
  info: (title: string, options?: Options) => push('info', title, options),
  success: (title: string, options?: Options) => push('success', title, options),
  warning: (title: string, options?: Options) => push('warning', title, options),
  /** An error in plain words (friendlyError), with its one-click fix as the action when there is one. */
  error: (error: unknown, onAction?: (action: ErrorAction) => void, labels?: Record<ErrorAction, string>) => {
    const friendly = describeError(error)
    const action = friendly.action && onAction && labels ? { label: labels[friendly.action], run: () => onAction(friendly.action!) } : undefined
    return push('error', friendly.title, { description: friendly.hint, detail: friendly.detail || undefined, action })
  },
}

export const useToasts = () => useSyncExternalStore(listener => { listeners.add(listener); return () => { listeners.delete(listener) } }, () => toasts)
