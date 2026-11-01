import { t } from '../i18n'
import type { ChatProject, ChatProjectDetail, ChatProjectListItem, SessionSummary } from '../types'

export type ProjectsView = { kind: 'list' } | { kind: 'new' } | { kind: 'project'; id: string }
export type ProjectSort = 'recent' | 'name' | 'created'

export function relativeTime(ms: number) {
  const minutes = Math.floor((Date.now() - ms) / 60000)
  if (minutes < 1) return t('just now')
  if (minutes < 60) return t('{n} min ago', { n: minutes })
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return hours === 1 ? t('1 hour ago') : t('{n} hours ago', { n: hours })
  const days = Math.floor(hours / 24)
  if (days < 30) return days === 1 ? t('yesterday') : t('{n} days ago', { n: days })
  return new Date(ms).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' })
}

export const projectChats = (sessions: SessionSummary[], id: string) => sessions.filter(session => !session.projectPath && session.chatProjectId === id).sort((a, b) => b.updatedAt - a.updatedAt)

/** Latest activity in a project: its own edits or its newest chat. */
export const lastActivity = (project: ChatProject, sessions: SessionSummary[]) => Math.max(project.updated, ...projectChats(sessions, project.id).map(session => session.updatedAt))

export function sortProjects(list: ChatProjectListItem[], sort: ProjectSort, sessions: SessionSummary[]) {
  const copy = [...list]
  if (sort === 'name') copy.sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }))
  else if (sort === 'created') copy.sort((a, b) => b.created - a.created)
  else copy.sort((a, b) => lastActivity(b, sessions) - lastActivity(a, sessions))
  return copy
}

export function capacity(detail: Pick<ChatProjectDetail, 'knowledgeTokens' | 'budgetTokens' | 'inlineLimitTokens'>) {
  const percent = detail.budgetTokens > 0 ? (detail.knowledgeTokens / detail.budgetTokens) * 100 : 0
  return { percent, threshold: detail.budgetTokens > 0 ? (detail.inlineLimitTokens / detail.budgetTokens) * 100 : 40, retrieval: detail.knowledgeTokens > detail.inlineLimitTokens }
}

