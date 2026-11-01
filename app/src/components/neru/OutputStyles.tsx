import { useEffect, useState } from 'react'
import { Check, LoaderCircle, PenLine } from 'lucide-react'
import { api } from '../../api'
import { t } from '../../i18n'
import { cn } from '@/lib/utils'
import type { OutputStyle } from '../../types'
import './SidePanels.css'

/** Settings → Output style: how Neru writes its replies in this project, as `/output-style` sets it. */
export function OutputStyles({ hasProject, onError }: { hasProject: boolean; onError: (message: string) => void }) {
  const [styles, setStyles] = useState<OutputStyle[] | null>(null)
  const [saving, setSaving] = useState<string | null>(null)

  useEffect(() => {
    if (!hasProject) return
    let alive = true
    api.outputStyles().then(list => { if (alive) setStyles(list) }).catch(cause => onError(String(cause)))
    return () => { alive = false }
  }, [hasProject, onError])

  const choose = async (name: string) => {
    setSaving(name)
    try { setStyles(await api.setOutputStyle(name)) } catch (cause) { onError(String(cause)) } finally { setSaving(null) }
  }

  return <section className="settings-section output-styles"><h2>{t('Output style')}</h2>
    <p className="settings-lede">{t('How Neru writes while it works in this project. The choice is saved in the project’s .claude/settings.local.json, where Claude Code keeps it too.')}</p>
    {!hasProject && <p className="settings-note">{t('Open a project to choose its output style.')}</p>}
    {hasProject && styles === null && <p className="settings-note"><LoaderCircle size={14} className="animate-spin" /> {t('Loading styles…')}</p>}
    {styles && <div className="style-list" role="radiogroup" aria-label={t('Output style')}>{styles.map(style => <button key={style.name} type="button" role="radio" aria-checked={style.current}
      className={cn('style-option', style.current && 'active')} disabled={saving !== null} onClick={() => void choose(style.name)}>
      <span className="style-check" aria-hidden>{saving === style.name ? <LoaderCircle size={14} className="animate-spin" /> : style.current ? <Check size={14} /> : null}</span>
      <span className="style-text"><strong>{style.name}</strong><small>{style.description || t('No description')}</small></span>
      {style.source !== 'built-in' && <small className="style-source">{t(style.source)}</small>}
    </button>)}</div>}
    <p className="settings-note"><PenLine size={14} /> {t('Add your own as a Markdown file in .neru/output-styles/ or ~/.claude/output-styles/: front matter with name and description, then the instructions.')}</p>
  </section>
}
