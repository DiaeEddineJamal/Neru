import { useState } from 'react'
import { ChevronDown, CircleAlert, X } from 'lucide-react'
import { actionLabels, describeError, type ErrorAction } from '@/lib/friendlyError'
import { cn } from '@/lib/utils'

/** An error in plain words: what happened, what to do, a one-click fix, and the raw text under Details. */
export function ErrorNotice({ error, onAction, onDismiss, compact = false }: { error: unknown; onAction?: (action: ErrorAction) => void; onDismiss?: () => void; compact?: boolean }) {
  const [open, setOpen] = useState(false)
  const friendly = describeError(error)
  return <div className={cn('error-notice', compact && 'compact')} role="alert">
    <CircleAlert size={16} className="error-notice-icon" aria-hidden />
    <div className="error-notice-body">
      <strong>{friendly.title}</strong>
      {friendly.hint && <span>{friendly.hint}</span>}
      {(friendly.action && onAction) || friendly.detail ? <div className="error-notice-actions">
        {friendly.action && onAction && <button type="button" className="button subtle" onClick={() => onAction(friendly.action!)}>{actionLabels[friendly.action]}</button>}
        {friendly.detail && <button type="button" className="error-notice-details" aria-expanded={open} onClick={() => setOpen(value => !value)}>Details <ChevronDown size={12} className={cn(open && 'open')} /></button>}
      </div> : null}
      {open && friendly.detail && <pre className="error-notice-raw">{friendly.detail}</pre>}
    </div>
    {onDismiss && <button type="button" className="icon-button" onClick={onDismiss} aria-label="Dismiss"><X size={14} /></button>}
  </div>
}
