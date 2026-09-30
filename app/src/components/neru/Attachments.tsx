import { useEffect, useState } from 'react'
import { AnimatePresence, motion, useReducedMotion } from 'motion/react'
import { FileText, LoaderCircle, MousePointer2, Paperclip, X } from 'lucide-react'
import type { AttachedDocument } from '../../types'

/** Attachments inside the message box: image thumbnails (click to enlarge) and compact chips for other files, like Claude. */
export function Attachments({ paths, documents, reading, onRemovePath, onRemoveDocument }: {
  paths: string[]
  documents: AttachedDocument[]
  reading: number
  onRemovePath: (path: string) => void
  onRemoveDocument: (path: string) => void
}) {
  const reduce = useReducedMotion() ?? false
  const images = documents.filter(doc => doc.kind === 'image' && doc.dataUrl)
  const others = documents.filter(doc => !(doc.kind === 'image' && doc.dataUrl))
  const [open, setOpen] = useState<number | null>(null)
  useEffect(() => {
    if (open === null) return
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') { event.stopPropagation(); setOpen(null) }
      if (event.key === 'ArrowRight') setOpen(index => index === null ? null : (index + 1) % images.length)
      if (event.key === 'ArrowLeft') setOpen(index => index === null ? null : (index - 1 + images.length) % images.length)
    }
    window.addEventListener('keydown', onKey, true)
    return () => window.removeEventListener('keydown', onKey, true)
  }, [open, images.length])
  const base = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path
  return <>
    <div className="composer-attachments" role="list" aria-label="Attachments">
      {images.map((doc, index) => <div key={doc.path} role="listitem" className="attach-thumb">
        <button type="button" className="attach-open" onClick={() => setOpen(index)} aria-label={`Preview ${doc.name}`} title={doc.name}>
          <img src={doc.dataUrl} alt="" decoding="async" draggable={false} />
        </button>
        <button type="button" className="attach-remove" onClick={() => onRemoveDocument(doc.path)} aria-label={`Remove ${doc.name}`}><X size={11} strokeWidth={2.4} /></button>
      </div>)}
      {others.map(doc => doc.kind === 'element'
        // An element picked in the in-app browser: a compact tag chip, like Claude's. The full
        // inspector detail travels with the message but stays out of the box.
        ? <div key={doc.path} role="listitem" className="attach-chip element" title={doc.text}>
          <MousePointer2 size={13} />
          <span className="attach-name">{doc.name}</span>
          <button type="button" className="attach-chip-remove" onClick={() => onRemoveDocument(doc.path)} aria-label={`Remove ${doc.name}`}><X size={12} /></button>
        </div>
        : <div key={doc.path} role="listitem" className="attach-chip" title={doc.path}>
        <FileText size={14} />
        <span className="attach-name">{doc.name}</span>
        <button type="button" className="attach-chip-remove" onClick={() => onRemoveDocument(doc.path)} aria-label={`Remove ${doc.name}`}><X size={12} /></button>
      </div>)}
      {paths.map(path => <div key={path} role="listitem" className="attach-chip" title={path}>
        <Paperclip size={14} />
        <span className="attach-name">{base(path)}</span>
        <button type="button" className="attach-chip-remove" onClick={() => onRemovePath(path)} aria-label={`Remove ${path}`}><X size={12} /></button>
      </div>)}
      {reading > 0 && <div className="attach-chip reading" role="status"><LoaderCircle size={14} className="animate-spin" /><span className="attach-name">Reading {reading} file{reading === 1 ? '' : 's'}…</span></div>}
    </div>
    <AnimatePresence>{open !== null && images[open] && <motion.div className="image-lightbox" role="dialog" aria-label="Image" onClick={() => setOpen(null)}
      initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} transition={{ duration: reduce ? 0 : 0.18 }}>
      <motion.img src={images[open].dataUrl} alt="" onClick={event => event.stopPropagation()}
        initial={reduce ? false : { scale: 0.94, opacity: 0 }} animate={{ scale: 1, opacity: 1 }} exit={reduce ? undefined : { scale: 0.96, opacity: 0 }} transition={{ type: 'spring', duration: 0.35, bounce: 0.15 }} />
      <button type="button" className="image-lightbox-close" aria-label="Close" onClick={() => setOpen(null)}>×</button>
    </motion.div>}</AnimatePresence>
  </>
}
