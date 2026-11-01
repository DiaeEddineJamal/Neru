// Keyboard shortcuts the user can rebind (Settings → Keyboard shortcuts). Bindings are saved in this
// browser profile; a change is announced with the `neru-shortcuts` window event.

export type ShortcutId = 'palette' | 'newSession' | 'toggleSidebar' | 'settings' | 'sideQuestion' | 'compare' | 'documents' | 'stop'

export const SHORTCUTS: { id: ShortcutId; label: string; keys: string }[] = [
  { id: 'palette', label: 'Open the command palette', keys: 'Ctrl+K' },
  { id: 'newSession', label: 'New session', keys: 'Ctrl+N' },
  { id: 'toggleSidebar', label: 'Show or hide the sidebar', keys: 'Ctrl+B' },
  { id: 'settings', label: 'Open settings', keys: 'Ctrl+,' },
  { id: 'sideQuestion', label: 'Ask a side question (/btw)', keys: 'Ctrl+Shift+Q' },
  { id: 'compare', label: 'Compare models', keys: 'Ctrl+Shift+M' },
  { id: 'documents', label: 'Show the session’s documents', keys: 'Ctrl+Shift+D' },
  { id: 'stop', label: 'Stop the reply', keys: 'Escape' },
]

/** Keys the app or the text fields already use; binding one of these is flagged. */
export const RESERVED: Record<string, string> = {
  'Ctrl+C': 'Copy', 'Ctrl+V': 'Paste', 'Ctrl+X': 'Cut', 'Ctrl+A': 'Select all', 'Ctrl+Z': 'Undo', 'Ctrl+Y': 'Redo', 'Ctrl+Shift+Z': 'Redo',
  'Ctrl+S': 'Save (editors)', 'Ctrl+F': 'Find', 'Ctrl+Enter': 'Queue the message', 'Ctrl+Alt+Enter': 'Send in a forked session',
  'Enter': 'Send', 'Shift+Enter': 'New line', 'Tab': 'Move focus', 'Ctrl+Shift+I': 'Developer tools', 'Ctrl+R': 'Reload', 'F5': 'Reload',
}

const STORAGE = 'neru.shortcuts'
const MODIFIERS = new Set(['Control', 'Shift', 'Alt', 'Meta', 'AltGraph', 'CapsLock'])

/** The key combination of a keydown, like `Ctrl+Shift+K`; null for a bare modifier. Cmd counts as Ctrl. */
export function comboOf(event: Pick<KeyboardEvent, 'key' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey'>): string | null {
  if (MODIFIERS.has(event.key)) return null
  const key = event.key === ' ' ? 'Space' : event.key.length === 1 ? event.key.toUpperCase() : event.key
  // Shift that only picks a symbol (`?` is Shift+/) is part of the key itself.
  const shift = event.shiftKey && (key.length > 1 || /[A-Z0-9]/.test(key))
  return [event.ctrlKey || event.metaKey ? 'Ctrl' : '', event.altKey ? 'Alt' : '', shift ? 'Shift' : '', key].filter(Boolean).join('+')
}

export type Bindings = Record<ShortcutId, string>

export const defaultBindings = (): Bindings => Object.fromEntries(SHORTCUTS.map(item => [item.id, item.keys])) as Bindings

export function loadBindings(): Bindings {
  const bindings = defaultBindings()
  try {
    const saved = JSON.parse(localStorage.getItem(STORAGE) ?? '{}') as Partial<Record<string, unknown>>
    for (const item of SHORTCUTS) if (typeof saved[item.id] === 'string') bindings[item.id] = saved[item.id] as string
  } catch { /* storage unavailable: defaults */ }
  return bindings
}

export function saveBindings(bindings: Bindings) {
  const changed = Object.fromEntries(SHORTCUTS.filter(item => bindings[item.id] !== item.keys).map(item => [item.id, bindings[item.id]]))
  try { localStorage.setItem(STORAGE, JSON.stringify(changed)) } catch { /* storage unavailable */ }
  window.dispatchEvent(new Event('neru-shortcuts'))
}

/** Why a binding is a problem: another action has it, or the app already uses those keys. Empty keys are off. */
export function conflictOf(bindings: Bindings, id: ShortcutId): string | null {
  const keys = bindings[id]
  if (!keys) return null
  const other = SHORTCUTS.find(item => item.id !== id && bindings[item.id] === keys)
  if (other) return `Also bound to “${other.label}”`
  if (id !== 'stop' && RESERVED[keys]) return `Overrides ${RESERVED[keys]}`
  return null
}

/** Which action a keydown triggers, if any. */
export function actionFor(bindings: Bindings, event: KeyboardEvent): ShortcutId | null {
  const combo = comboOf(event)
  if (!combo) return null
  return SHORTCUTS.find(item => bindings[item.id] === combo)?.id ?? null
}
