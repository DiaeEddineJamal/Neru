import { useEffect } from 'react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { ArrowUpCircle, LoaderCircle, Sparkles, X } from 'lucide-react'
import { EASE_OUT } from '@/lib/ease'
import changelog from '../../changelog.json'

export interface ChangelogEntry { version: string; date: string; title: string; summary: string; sections: { title: string; items: string[] }[] }
export const releases = changelog as ChangelogEntry[]

/** What changed in a release, from the same changelog the GitHub release notes are built from. */
export function WhatsNew({ open, version, onClose }: { open: boolean; version?: string; onClose: () => void }) {
  const reduce = useReducedMotion() ?? false
  const entry = releases.find(item => item.version === version) ?? releases[0]
  const older = releases.filter(item => item !== entry).slice(0, 3)
  useEffect(() => {
    if (!open) return
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') onClose() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open, onClose])
  return <AnimatePresence>{open && entry && <motion.div className="whats-new-backdrop" onClick={onClose} initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} transition={{ duration: reduce ? 0 : 0.2 }}>
    <motion.section className="whats-new" role="dialog" aria-modal="true" aria-labelledby="whats-new-title" onClick={event => event.stopPropagation()}
      initial={reduce ? { opacity: 0 } : { opacity: 0, y: 14, scale: 0.98 }} animate={{ opacity: 1, y: 0, scale: 1 }} exit={reduce ? { opacity: 0 } : { opacity: 0, y: 8, scale: 0.98 }} transition={{ duration: reduce ? 0 : 0.32, ease: EASE_OUT }}>
      <header>
        <span className="whats-new-eyebrow"><Sparkles size={14} /> What’s new in {entry.version}</span>
        <button type="button" className="icon-button" onClick={onClose} aria-label="Close"><X size={16} /></button>
      </header>
      <h2 id="whats-new-title">{entry.title}</h2>
      <p className="whats-new-summary">{entry.summary}</p>
      <div className="whats-new-body">
        {entry.sections.map(part => <div key={part.title} className="whats-new-part"><h3>{part.title}</h3><ul>{part.items.map(item => <li key={item}>{item}</li>)}</ul></div>)}
        {older.length > 0 && <div className="whats-new-part whats-new-older"><h3>Earlier releases</h3><ul>{older.map(item => <li key={item.version}><strong>{item.version}</strong> · {item.title}</li>)}</ul></div>}
      </div>
      <footer><span>{entry.date}</span><button type="button" className="button primary" onClick={onClose}>Got it</button></footer>
    </motion.section>
  </motion.div>}</AnimatePresence>
}

/** A quiet toast when a new version is ready: see what changed, or install and restart. */
export function UpdateToast({ version, progress, onInstall, onDetails, onDismiss }: { version: string; progress: number | null | undefined; onInstall: () => void; onDetails: () => void; onDismiss: () => void }) {
  const reduce = useReducedMotion() ?? false
  const installing = progress !== undefined
  return <motion.aside className="update-toast" role="status" initial={reduce ? { opacity: 0 } : { opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: reduce ? 0 : 0.3, ease: EASE_OUT }}>
    <ArrowUpCircle size={18} className="update-toast-icon" />
    <div className="update-toast-text"><strong>Neru {version} is available</strong><span>{installing ? (progress === null ? 'Downloading…' : `Downloading… ${Math.round((progress ?? 0) * 100)}%`) : 'Restart to update. Your sessions are kept.'}</span>
      {installing && <i className="update-toast-bar" style={{ ['--done' as string]: `${Math.round((progress ?? 0) * 100)}%` }} />}
    </div>
    <div className="update-toast-actions">
      {!installing && <button type="button" className="button subtle" onClick={onDetails}>What’s new</button>}
      <button type="button" className="button primary" onClick={onInstall} disabled={installing}>{installing ? <LoaderCircle size={14} className="animate-spin" /> : null}{installing ? 'Updating' : 'Restart to update'}</button>
      {!installing && <button type="button" className="icon-button" onClick={onDismiss} aria-label="Later"><X size={15} /></button>}
    </div>
  </motion.aside>
}
