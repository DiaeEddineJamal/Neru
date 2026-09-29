import { CircleCheck, CircleDashed, CircleMinus, CircleX, ExternalLink, LoaderCircle, RefreshCw, Wrench } from 'lucide-react'
import type { PrStatus } from '../../types'

const icon = { success: CircleCheck, failure: CircleX, pending: CircleDashed, skipped: CircleMinus }

/** The branch's pull request with its CI checks, and a way to hand failures to Neru. */
export function PullRequestChecks({ pr, loading, error, fixing, autoFix, autoMerge, onAutoFix, onAutoMerge, onRefresh, onOpen, onFix }: {
  pr: PrStatus | null
  loading: boolean
  error: string
  fixing: boolean
  autoFix: boolean
  autoMerge: boolean
  onAutoFix: (value: boolean) => void
  onAutoMerge: (value: boolean) => void
  onRefresh: () => void
  onOpen: (url: string) => void
  onFix: () => void
}) {
  if (error) return <div className="pr-card muted-card"><p>{error}</p></div>
  if (!pr) return loading ? <div className="pr-card muted-card"><LoaderCircle size={14} className="animate-spin" /> Checking for a pull request…</div> : null
  const failed = pr.checks.filter(check => check.state === 'failure').length
  const pending = pr.checks.filter(check => check.state === 'pending').length
  const summary = pr.checks.length === 0 ? 'No checks reported' : failed ? `${failed} failing` : pending ? `${pending} running` : 'All checks passed'
  return <div className="pr-card">
    <div className="pr-head">
      <button className="pr-title" onClick={() => onOpen(pr.url)} title="Open on GitHub"><span>#{pr.number}</span> {pr.title}<ExternalLink size={12} /></button>
      <button className="icon-button small" onClick={onRefresh} aria-label="Refresh checks" title="Refresh">{loading ? <LoaderCircle size={14} className="animate-spin" /> : <RefreshCw size={14} />}</button>
    </div>
    <div className={`pr-summary ${failed ? 'failure' : pending ? 'pending' : 'success'}`}>{pr.state.toLowerCase()} · {summary}</div>
    {pr.checks.length > 0 && <ul className="pr-checks">{pr.checks.map(check => {
      const Icon = icon[check.state]
      return <li key={`${check.name}-${check.url}`} className={check.state}><Icon size={14} className={check.state === 'pending' ? 'animate-spin-slow' : undefined} /><span className="truncate">{check.name}</span>{check.url && <button className="text-action" onClick={() => onOpen(check.url!)}>Details</button>}</li>
    })}</ul>}
    <div className="pr-toggles">
      <label><input type="checkbox" checked={autoFix} onChange={event => onAutoFix(event.target.checked)} /> Auto-fix failures</label>
      <label><input type="checkbox" checked={autoMerge} onChange={event => onAutoMerge(event.target.checked)} /> Auto-merge when green</label>
    </div>
    {failed > 0 && <button className="button primary" onClick={onFix} disabled={fixing}>{fixing ? <LoaderCircle size={14} className="animate-spin" /> : <Wrench size={14} />} Ask Neru to fix the failing checks</button>}
  </div>
}
