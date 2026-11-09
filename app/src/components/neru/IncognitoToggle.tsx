import { useCallback, useEffect, useState } from 'react'
import { isTauri } from '@tauri-apps/api/core'
import { Incognito } from '@/components/icons'
import { cn } from '@/lib/utils'
import { t } from '../../i18n'
import { memoryApi, type IncognitoState } from '@/lib/memoryApi'
import './MemorySummary.css'

/** How often the shown chat's state is checked again, in case the window moved to another session. */
const RECHECK_MS = 2500

/**
 * Incognito in Chat. On the empty chat page it is a toggle: the chat about to start is not saved to
 * history, never learned from, summarized or searched, and gets no memory. In an incognito chat it is a
 * filled mark saying so, like a browser's incognito icon. `refreshKey` changes when the composer's situation does, so the state is read again.
 */
export function IncognitoToggle({ home, refreshKey, disabled }: { home: boolean; refreshKey?: string; disabled?: boolean }) {
  const [state, setState] = useState<IncognitoState | null>(null)
  const [saving, setSaving] = useState(false)
  const read = useCallback(() => { if (isTauri()) void memoryApi.incognito().then(setState).catch(() => undefined) }, [])
  useEffect(() => { read() }, [read, home, refreshKey])
  useEffect(() => {
    if (!isTauri()) return
    const timer = window.setInterval(read, RECHECK_MS)
    window.addEventListener('focus', read)
    return () => { window.clearInterval(timer); window.removeEventListener('focus', read) }
  }, [read])
  if (!state) return null

  const startingIncognito = state.next || (state.active && state.activeEmpty)
  if (home) {
    const flip = async () => {
      setSaving(true)
      try { setState(await memoryApi.setIncognito(!startingIncognito)) } catch { read() } finally { setSaving(false) }
    }
    return <button type="button" className={cn('prompt-toggle', startingIncognito && 'on')} aria-pressed={startingIncognito} aria-label={t('Incognito')} disabled={disabled || saving} onClick={() => void flip()}
      title={startingIncognito ? t('Incognito on: this chat is not saved and does not use or add to memory') : t('Start an incognito chat: not saved to history, no memory')}>
      <Incognito size={15} weight={startingIncognito ? 'fill' : 'regular'} aria-hidden />
    </button>
  }
  if (!state.active) return null
  return <span className="prompt-toggle on incognito-mark" role="img" aria-label={t('Incognito chat')} title={t('This chat is not saved to history and does not use or add to memory. It is gone when you leave it.')}>
    <Incognito size={15} weight="fill" aria-hidden />
  </span>
}
