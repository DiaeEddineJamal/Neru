import { CornerDownLeft, Pencil, X } from 'lucide-react'
import { canSteerQueued, queueLabel, type QueuedMessage } from '../../lib/queue'

function Preview({ item }: { item: QueuedMessage }) {
  const images = item.documents.filter(doc => doc.kind === 'image')
  const files = item.documents.filter(doc => doc.kind !== 'image')
  return <span className="queue-preview">
    {images.map(doc => <em key={doc.path}>[Image]</em>)}
    {files.map(doc => <em key={doc.path}>[{doc.name}]</em>)}
    {item.contextPaths.map(path => <em key={path}>[{path.split(/[\\/]/).pop()}]</em>)}
    {item.text && <span className="queue-text">{item.text.replace(/\s+/g, ' ')}</span>}
  </span>
}

/** The bar above the message box listing messages waiting for the running reply to finish. */
export function QueueBar({ items, onSteer, onEdit, onRemove }: {
  items: QueuedMessage[]
  onSteer: (id: string) => void
  onEdit: (id: string) => void
  onRemove: (id: string) => void
}) {
  if (items.length === 0) return null
  const actions = (item: QueuedMessage) => <span className="queue-actions">
    <button type="button" onClick={() => onSteer(item.id)} disabled={!canSteerQueued(item)} aria-label="Send now" title={canSteerQueued(item) ? 'Send now: Neru reads it before its next step' : 'Messages with attachments send when the reply finishes'}><CornerDownLeft size={14} /></button>
    <button type="button" onClick={() => onEdit(item.id)} aria-label="Edit queued message" title="Edit: move it back into the message box"><Pencil size={14} /></button>
    <button type="button" onClick={() => onRemove(item.id)} aria-label="Remove queued message" title="Remove"><X size={15} /></button>
  </span>
  return <div className="queue-bar" role="status" aria-label={queueLabel(items.length)}>
    {items.length === 1
      ? <div className="queue-row"><strong>{queueLabel(1)}</strong><Preview item={items[0]} />{actions(items[0])}</div>
      : <>
        <div className="queue-head"><strong>{queueLabel(items.length)}</strong></div>
        {items.map(item => <div className="queue-row" key={item.id}><Preview item={item} />{actions(item)}</div>)}
      </>}
  </div>
}
