import { useEffect, useState } from 'react'
import { X } from 'lucide-react'
import { cn } from '@/lib/utils'
import { t } from '../../i18n'

/**
 * Deny with a reason: the agent hears why, and with "Remember" the reason becomes a rule in the project's
 * .neru/memory.md, which every later session reads, so the same mistake does not come back.
 */
export function DenyDialog({ open, project, onCancel, onDeny }: { open: boolean; project: boolean; onCancel: () => void; onDeny: (reason: string, remember: boolean) => void }) {
  const [reason, setReason] = useState('')
  const [remember, setRemember] = useState(true)
  useEffect(() => { if (open) setReason('') }, [open])
  useEffect(() => {
    if (!open) return
    const onKey = (event: KeyboardEvent) => { if (event.key === 'Escape') onCancel() }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [open, onCancel])
  if (!open) return null
  const submit = () => onDeny(reason.trim(), remember && project && Boolean(reason.trim()))
  return <div className="modal-backdrop" onMouseDown={onCancel}>
    <form className="deny-dialog" role="dialog" aria-modal="true" aria-labelledby="deny-title" onMouseDown={event => event.stopPropagation()} onSubmit={event => { event.preventDefault(); submit() }}>
      <header><h2 id="deny-title">{t('Deny this change')}</h2><button type="button" className="icon-button" onClick={onCancel} aria-label={t('Close')}><X size={16} /></button></header>
      <label>{t('Why?')} <span>{t('(optional)')}</span>
        <textarea autoFocus rows={3} value={reason} onChange={event => setReason(event.target.value)} placeholder={t("Don't touch generated files. Use the existing date helper instead.")}
          onKeyDown={event => { if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) { event.preventDefault(); submit() } }} />
      </label>
      {project && <div className="settings-row deny-remember"><div><strong>{t('Remember for this project')}</strong><p>{t('Saved to .neru/memory.md, which Neru reads in every later session.')}</p></div>
        <button type="button" className={cn('switch', remember && 'on')} role="switch" aria-checked={remember} aria-label={t('Remember for this project')} onClick={() => setRemember(!remember)} /></div>}
      <footer><button type="button" className="button subtle" onClick={onCancel}>{t('Cancel')}</button><button type="submit" className="button danger">{reason.trim() ? t('Deny and tell Neru') : t('Deny')}</button></footer>
    </form>
  </div>
}
