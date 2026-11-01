import { useEffect, useState } from 'react'
import { Keyboard, RotateCcw, TriangleAlert, X } from '@/components/icons'
import { t } from '../../i18n'
import { cn } from '@/lib/utils'
import './SidePanels.css'
import { SHORTCUTS, comboOf, conflictOf, defaultBindings, loadBindings, saveBindings, type Bindings, type ShortcutId } from '@/lib/shortcuts'

/** Settings → Keyboard shortcuts: rebind the app's shortcuts; clashes are flagged, not refused. */
export function ShortcutsSettings() {
  const [bindings, setBindings] = useState<Bindings>(loadBindings)
  const [recording, setRecording] = useState<ShortcutId | null>(null)
  const change = (next: Bindings) => { setBindings(next); saveBindings(next) }

  useEffect(() => {
    if (!recording) return
    // Capture before the app's own shortcuts see the keys.
    const onKey = (event: KeyboardEvent) => {
      event.preventDefault()
      event.stopPropagation()
      if (event.key === 'Escape' && !event.ctrlKey && !event.altKey && !event.shiftKey && recording !== 'stop') { setRecording(null); return }
      const combo = comboOf(event)
      if (!combo) return
      change({ ...bindings, [recording]: combo })
      setRecording(null)
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  })

  const defaults = defaultBindings()
  const changed = SHORTCUTS.some(item => bindings[item.id] !== defaults[item.id])
  return <section className="settings-section shortcut-settings"><h2>{t('Keyboard shortcuts')}</h2>
    <p className="settings-lede">{t('Click a shortcut, then press the keys you want. Ctrl stands for Cmd on a Mac. A shortcut that clashes with another one, or with keys Neru already uses, is marked.')}</p>
    <div className="shortcut-list">{SHORTCUTS.map(item => {
      const conflict = conflictOf(bindings, item.id)
      const keys = bindings[item.id]
      return <div key={item.id} className={cn('shortcut-row', conflict && 'conflict')}>
        <span className="shortcut-label">{t(item.label)}{conflict && <small className="shortcut-conflict"><TriangleAlert size={12} /> {t(conflict)}</small>}</span>
        <button type="button" className={cn('shortcut-keys', recording === item.id && 'recording')} aria-label={t('Change the shortcut for {label}', { label: t(item.label) })} onClick={() => setRecording(recording === item.id ? null : item.id)}>
          {recording === item.id ? t('Press keys…') : keys ? keys.split('+').map(part => <kbd key={part}>{part}</kbd>) : <span className="shortcut-off">{t('Off')}</span>}
        </button>
        <span className="shortcut-actions">
          {keys && <button type="button" className="icon-button small" title={t('Turn off')} aria-label={t('Turn off the shortcut for {label}', { label: t(item.label) })} onClick={() => change({ ...bindings, [item.id]: '' })}><X size={13} /></button>}
          {keys !== item.keys && <button type="button" className="icon-button small" title={t('Back to {keys}', { keys: item.keys })} aria-label={t('Reset the shortcut for {label}', { label: t(item.label) })} onClick={() => change({ ...bindings, [item.id]: item.keys })}><RotateCcw size={13} /></button>}
        </span>
      </div>
    })}</div>
    <div className="settings-actions">
      <button type="button" className="button subtle" disabled={!changed} onClick={() => change(defaultBindings())}><RotateCcw size={14} /> {t('Reset all')}</button>
    </div>
    <p className="settings-note"><Keyboard size={14} /> {t('The editor keeps its own keys (Ctrl+S to save, Ctrl+Z to undo), and Enter, Ctrl+Enter and Ctrl+Alt+Enter still send, queue and fork from the prompt.')}</p>
  </section>
}
