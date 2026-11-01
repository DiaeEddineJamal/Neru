import { useCallback, useEffect, useRef, useState } from 'react'
import { CircleAlert, CircleDashed, ExternalLink, GitBranch, GitCommitHorizontal, GitMerge, GitPullRequest, LoaderCircle, RefreshCw } from '@/components/icons'
import { cn } from '@/lib/utils'
import { api } from '../../api'
import { t } from '../../i18n'
import type { BoardCard, SessionStatus } from '../../types'
import { Pill } from './team/TeamPanels'
import './SessionBoard.css'

const COLUMNS: { id: SessionStatus; title: string; hint: string }[] = [
  { id: 'running', title: 'Running', hint: 'Working now' },
  { id: 'needs-you', title: 'Needs you', hint: 'Waiting for an approval or an answer' },
  { id: 'done', title: 'Done', hint: 'Finished; review, commit or open a pull request' },
  { id: 'merged', title: 'Merged', hint: 'Its work is in the base branch' },
]

type Action = 'commit' | 'pr' | 'merge'

/** Every session of the project as a card, in Running, Needs you, Done and Merged. */
export function SessionBoard({ projectName, activeSessionId, refreshKey, onOpen, onError, onNotice }: {
  projectName: string
  activeSessionId: string | null
  /** Changes when sessions start, stop or change, so the board reloads. */
  refreshKey: unknown
  onOpen: (id: string) => void
  onError: (message: string) => void
  onNotice: (message: string) => void
}) {
  const [cards, setCards] = useState<BoardCard[] | null>(null)
  const [loadingPrs, setLoadingPrs] = useState(false)
  const [acting, setActing] = useState<string | null>(null)

  const load = useCallback(async (withPr: boolean) => {
    if (withPr) setLoadingPrs(true)
    try { setCards(await api.sessionBoard(withPr)) } catch (cause) { onError(String(cause)) } finally { if (withPr) setLoadingPrs(false) }
  }, [onError])

  // Git state first (fast); the first time, pull requests from the GitHub CLI after it.
  const prsLoaded = useRef(false)
  useEffect(() => {
    void load(false).then(() => { if (!prsLoaded.current) { prsLoaded.current = true; void load(true) } })
  }, [refreshKey, load])

  const act = async (card: BoardCard, action: Action) => {
    setActing(`${card.session.id}:${action}`)
    try {
      const result = await api.boardAction(card.session.id, action)
      onNotice(action === 'pr' && result.startsWith('https://') ? t('Opened pull request {url}', { url: result }) : result.split('\n')[0] || t('Done'))
      await load(true)
    } catch (cause) {
      onError(String(cause))
    } finally {
      setActing(null)
    }
  }

  const button = (card: BoardCard, action: Action, label: string, icon: React.ReactNode, enabled: boolean, title: string) =>
    <button type="button" className="button subtle small" disabled={!enabled || acting !== null} title={title} onClick={event => { event.stopPropagation(); void act(card, action) }}>
      {acting === `${card.session.id}:${action}` ? <LoaderCircle size={13} className="animate-spin" /> : icon}{label}
    </button>

  return <div className="session-board">
    <div className="session-board-head">
      <div><h2>{t('Sessions')}</h2><p>{t('Every session in {name}, by where it stands.', { name: projectName })}</p></div>
      <button type="button" className="button subtle small" onClick={() => void load(true)} disabled={loadingPrs}>{loadingPrs ? <LoaderCircle size={13} className="animate-spin" /> : <RefreshCw size={13} />}{t('Refresh')}</button>
    </div>
    {cards === null ? <p className="settings-note"><LoaderCircle size={13} className="animate-spin" /> {t('Reading sessions…')}</p>
      : <div className="session-board-columns">
        {COLUMNS.map(column => {
          const items = cards.filter(card => (card.session.status ?? 'done') === column.id)
          return <section key={column.id} className={cn('session-board-column', `is-${column.id}`)} aria-label={t(column.title)}>
            <header title={t(column.hint)}><strong>{t(column.title)}</strong><Pill>{items.length}</Pill></header>
            {items.length === 0 && <p className="empty-small">{t('No sessions')}</p>}
            {items.map(card => {
              const { session } = card
              const tree = Boolean(session.worktree)
              const prOpen = session.pr?.state === 'OPEN'
              return <article key={session.id} className={cn('session-card', session.id === activeSessionId && 'active')} onClick={() => onOpen(session.id)} onKeyDown={event => { if (event.key === 'Enter') onOpen(session.id) }} tabIndex={0} role="button" aria-label={t('Open {title}', { title: session.title })}>
                <div className="session-card-title">
                  {column.id === 'running' ? <LoaderCircle size={13} className="animate-spin" /> : column.id === 'needs-you' ? <CircleAlert size={13} /> : <CircleDashed size={13} />}
                  <span className="truncate">{session.title}</span>
                </div>
                <div className="session-card-meta">
                  {card.branch && <span className="session-card-branch" title={tree ? t('Worktree branch, based on {base}', { base: card.base }) : t('Main checkout')}><GitBranch size={12} />{card.branch}</span>}
                  {card.dirty > 0 && <Pill tone="amber">{t('{n} uncommitted', { n: card.dirty })}</Pill>}
                  {tree && card.ahead > 0 && <Pill>{t('{n} ahead', { n: card.ahead })}</Pill>}
                  {session.pr && <a className={cn('session-card-pr', session.pr.checks && `is-${session.pr.checks}`)} href={session.pr.url} target="_blank" rel="noreferrer" onClick={event => event.stopPropagation()} title={session.pr.checks ? t('Checks {state}', { state: t(session.pr.checks) }) : undefined}>
                    <GitPullRequest size={12} />#{session.pr.number} · {session.pr.state.toLowerCase()}{session.pr.checks ? ` · ${t(session.pr.checks)}` : ''}<ExternalLink size={11} />
                  </a>}
                </div>
                {column.id !== 'running' && column.id !== 'merged' && <div className="session-card-actions">
                  {button(card, 'commit', t('Commit'), <GitCommitHorizontal size={13} />, card.dirty > 0, t('Commit every change in this session’s checkout'))}
                  {tree && !session.pr && button(card, 'pr', t('Pull request'), <GitPullRequest size={13} />, card.dirty === 0 && card.ahead > 0, t('Push the branch and open a pull request'))}
                  {tree && button(card, 'merge', t('Merge'), <GitMerge size={13} />, card.dirty === 0 && (card.ahead > 0 || prOpen), prOpen ? t('Squash-merge its pull request') : t('Merge the branch into {base} in the main checkout', { base: card.base }))}
                </div>}
              </article>
            })}
          </section>
        })}
      </div>}
  </div>
}
