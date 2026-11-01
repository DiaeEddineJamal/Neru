import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { ChatProjectDetail, ChatProjectListItem, SessionSummary } from '../../types'
import { capacity, sortProjects } from '@/lib/chatProjects'

const hoisted = vi.hoisted(() => ({
  listChatProjects: vi.fn(),
  getChatProject: vi.fn(),
  createChatProject: vi.fn(),
  updateChatProject: vi.fn(),
}))
vi.mock('../../api', () => ({ api: hoisted }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }))

import { ChatProjects, type ChatProjectsProps } from './ChatProjects'

const DAY = 86_400_000
const now = Date.now()
const project = (id: string, name: string, extra: Partial<ChatProjectListItem> = {}): ChatProjectListItem => ({
  id, name, description: `${name} description`, instructions: '', starred: false, archived: false, created: now - 10 * DAY, updated: now - 5 * DAY, knowledgeItems: 0, knowledgeTokens: 0, ...extra,
})
const chat = (id: string, chatProjectId: string | undefined, updatedAt: number, title = `Chat ${id}`): SessionSummary => ({ id, title, projectPath: '', updatedAt, chatProjectId })

function setup(view: ChatProjectsProps['view'], sessions: SessionSummary[] = []) {
  const props: ChatProjectsProps = {
    view, sessions, busy: false,
    onView: vi.fn(), onStartChat: vi.fn(), onOpenChat: vi.fn(), onRenameChat: vi.fn(), onDeleteChat: vi.fn(), onMoveChat: vi.fn(), onChanged: vi.fn(),
  }
  render(<ChatProjects {...props} />)
  return props
}

describe('ChatProjects', () => {
  beforeEach(() => { Object.values(hoisted).forEach(fn => fn.mockReset()) })

  it('lists projects with chat counts, searches and opens one', async () => {
    hoisted.listChatProjects.mockResolvedValue([project('a', 'Thesis', { starred: true }), project('b', 'Cooking')])
    const props = setup({ kind: 'list' }, [chat('1', 'a', now), chat('2', 'a', now - DAY), chat('3', undefined, now)])
    expect(await screen.findByText('Thesis')).toBeTruthy()
    expect(screen.getByText(/2 chats/)).toBeTruthy()
    expect(screen.getByLabelText('Starred')).toBeTruthy()
    fireEvent.change(screen.getByLabelText('Search projects'), { target: { value: 'cook' } })
    expect(screen.queryByText('Thesis')).toBeNull()
    fireEvent.click(screen.getByText('Cooking'))
    expect(props.onView).toHaveBeenCalledWith({ kind: 'project', id: 'b' })
  })

  it('creates a project from the new-project form', async () => {
    hoisted.createChatProject.mockResolvedValue(project('n', 'Garden'))
    const props = setup({ kind: 'new' })
    const create = screen.getByRole('button', { name: 'Create project' }) as HTMLButtonElement
    expect(create.disabled).toBe(true)
    fireEvent.change(screen.getByLabelText('What are you working on?'), { target: { value: 'Garden' } })
    fireEvent.change(screen.getByLabelText('What are you trying to achieve?'), { target: { value: 'Plan the beds' } })
    fireEvent.click(create)
    await waitFor(() => expect(props.onView).toHaveBeenCalledWith({ kind: 'project', id: 'n' }))
    expect(hoisted.createChatProject).toHaveBeenCalledWith('Garden', 'Plan the beds')
    expect(props.onChanged).toHaveBeenCalled()
  })

  it('shows a project with its chats, knowledge meter, and starts a chat from the composer', async () => {
    const detail: ChatProjectDetail = {
      project: project('a', 'Thesis', { instructions: 'Use British English.' }),
      knowledge: [{ id: 'k1', title: 'notes.pdf', kind: 'file', source: 'notes.pdf', chars: 4000, tokens: 1000, added: now }],
      knowledgeTokens: 1000, budgetTokens: 100_000, inlineLimitTokens: 40_000,
    }
    hoisted.getChatProject.mockResolvedValue(detail)
    const props = setup({ kind: 'project', id: 'a' }, [chat('1', 'a', now, 'Outline'), chat('2', 'b', now, 'Elsewhere')])
    expect(await screen.findByRole('heading', { name: 'Thesis' })).toBeTruthy()
    expect(screen.getByText('Outline')).toBeTruthy()
    expect(screen.queryByText('Elsewhere')).toBeNull()
    expect(screen.getByText('notes.pdf')).toBeTruthy()
    expect(screen.getByText('Use British English.')).toBeTruthy()
    expect(screen.getByRole('meter', { name: 'Project knowledge capacity' }).getAttribute('aria-valuenow')).toBe('1')
    const box = screen.getByLabelText('Start a chat in Thesis')
    fireEvent.change(box, { target: { value: 'Draft chapter one' } })
    fireEvent.keyDown(box, { key: 'Enter' })
    expect(props.onStartChat).toHaveBeenCalledWith('a', 'Draft chapter one')
    fireEvent.click(screen.getByText('Outline'))
    expect(props.onOpenChat).toHaveBeenCalledWith('1')
  })

  it('toggles the star', async () => {
    const base = project('a', 'Thesis')
    hoisted.getChatProject.mockResolvedValue({ project: base, knowledge: [], knowledgeTokens: 0, budgetTokens: 100_000, inlineLimitTokens: 40_000 })
    hoisted.updateChatProject.mockResolvedValue({ ...base, starred: true })
    const props = setup({ kind: 'project', id: 'a' })
    fireEvent.click(await screen.findByRole('button', { name: 'Star project' }))
    await waitFor(() => expect(screen.getByRole('button', { name: 'Unstar project' })).toBeTruthy())
    expect(hoisted.updateChatProject).toHaveBeenCalledWith('a', { starred: true })
    expect(props.onChanged).toHaveBeenCalled()
  })
})

describe('chat project helpers', () => {
  it('sorts by activity, name and creation date', () => {
    const list = [project('a', 'beta', { created: 1, updated: 1 }), project('b', 'Alpha', { created: 2, updated: 2 })]
    const sessions = [chat('1', 'a', 10)]
    expect(sortProjects(list, 'recent', sessions).map(item => item.id)).toEqual(['a', 'b'])
    expect(sortProjects(list, 'name', sessions).map(item => item.id)).toEqual(['b', 'a'])
    expect(sortProjects(list, 'created', sessions).map(item => item.id)).toEqual(['b', 'a'])
  })
  it('reports capacity and the retrieval threshold', () => {
    expect(capacity({ knowledgeTokens: 50_000, budgetTokens: 100_000, inlineLimitTokens: 40_000 })).toEqual({ percent: 50, threshold: 40, retrieval: true })
    expect(capacity({ knowledgeTokens: 0, budgetTokens: 0, inlineLimitTokens: 0 }).percent).toBe(0)
  })
})
