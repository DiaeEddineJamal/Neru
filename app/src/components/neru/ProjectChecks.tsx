import { useEffect, useState } from 'react'
import { isTauri } from '@tauri-apps/api/core'
import { CircleCheck, CircleMinus, CircleX, ListChecks, LoaderCircle, TriangleAlert } from 'lucide-react'
import { api } from '../../api'
import type { CheckResult } from '../../types'
import { t } from '../../i18n'

const icon = { pass: CircleCheck, fail: CircleX, skipped: CircleMinus, error: TriangleAlert, idle: CircleMinus }

/**
 * The project's `.neru/checks/*.md`: run them as read-only agents and see pass/fail with findings. Also
 * the two automations of the Git section: run the checks before opening a pull request, and queue a fix
 * run whenever the pull request's CI fails (works while Neru is open, from the task queue).
 */
export function ProjectChecks({ projectPath, results, running, checksOnPr, onRun, onChecksOnPr, onError }: {
  projectPath: string
  results: CheckResult[] | null
  running: boolean
  checksOnPr: boolean
  onRun: () => void
  onChecksOnPr: (value: boolean) => void
  onError: (message: string) => void
}) {
  const [watching, setWatching] = useState(false)
  useEffect(() => {
    if (!isTauri()) return
    void api.ciWatching(projectPath).then(setWatching).catch(() => setWatching(false))
  }, [projectPath])
  const watch = (value: boolean) => void api.ciWatch(projectPath, value).then(setWatching).catch(cause => onError(String(cause)))
  return <div className="pr-card">
    <div className="pr-head">
      <span className="pr-title"><ListChecks size={14} /> {t('Project checks')}</span>
      <button className="button subtle small" onClick={onRun} disabled={running}>{running ? <LoaderCircle size={14} className="animate-spin" /> : null} {running ? t('Running…') : t('Run checks')}</button>
    </div>
    {results && results.length === 0 && <p className="task-meta">{t('No checks yet. Add markdown files to .neru/checks with a name, a description and an optional glob.')}</p>}
    {results && results.length > 0 && <ul className="pr-checks">{results.map(result => {
      const Icon = icon[result.status]
      const state = result.status === 'pass' ? 'success' : result.status === 'fail' || result.status === 'error' ? 'failure' : 'skipped'
      return <li key={result.name} className={state} title={result.findings.join('\n')}>
        <Icon size={14} /><span className="truncate">{result.name}{result.findings.length > 0 && result.status !== 'pass' ? ` — ${result.findings[0]}` : ''}</span>
      </li>
    })}</ul>}
    <div className="pr-toggles">
      <label><input type="checkbox" checked={checksOnPr} onChange={event => onChecksOnPr(event.target.checked)} /> {t('Run checks before opening a pull request')}</label>
      <label><input type="checkbox" checked={watching} onChange={event => watch(event.target.checked)} /> {t('Queue a fix run when CI fails')}</label>
    </div>
  </div>
}
