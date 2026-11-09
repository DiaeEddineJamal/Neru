import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { MemorySummary as Summary } from '@/lib/memoryApi'

const hoisted = vi.hoisted(() => ({
  summary: vi.fn(),
  setSummary: vi.fn(),
  regenerateSummary: vi.fn(),
  reviseSummary: vi.fn(),
}))
vi.mock('@/lib/memoryApi', () => ({
  memoryApi: hoisted,
  onMemoryChanged: () => () => undefined,
  memoryTime: (millis: number) => new Date(millis).toISOString().slice(0, 10),
}))

import { MemorySummary } from './MemorySummary'

const summary = (extra: Partial<Summary> = {}): Summary => ({
  text: '## Work context\nThe user is a nurse in Lyon.\n\n## Top of mind\nPlanning a move.',
  updatedAt: Date.UTC(2026, 10, 8), generatedAt: Date.UTC(2026, 10, 8), chatsThrough: 5, edited: false, generating: false, enabled: true, ...extra,
})

describe('MemorySummary', () => {
  beforeEach(() => { Object.values(hoisted).forEach(fn => fn.mockReset()) })

  it('renders the summary as markdown with when it was updated', async () => {
    hoisted.summary.mockResolvedValue(summary())
    render(<MemorySummary onError={vi.fn()} />)
    expect(await screen.findByRole('heading', { name: 'Top of mind' })).toBeTruthy()
    expect(screen.getByText('The user is a nurse in Lyon.')).toBeTruthy()
    expect(screen.getByText('Updated 2026-11-08')).toBeTruthy()
    expect(hoisted.summary).toHaveBeenCalledWith(undefined)
  })

  it('edits the summary and saves the user\'s text', async () => {
    hoisted.summary.mockResolvedValue(summary())
    hoisted.setSummary.mockResolvedValue(summary({ text: '## Work context\nThe user is a doctor.', edited: true, updatedAt: Date.UTC(2026, 10, 9) }))
    render(<MemorySummary projectId="p1" onError={vi.fn()} />)
    await screen.findByText('The user is a nurse in Lyon.')
    fireEvent.click(screen.getByRole('button', { name: /Edit/ }))
    const box = screen.getByRole('textbox', { name: 'Memory summary' })
    fireEvent.change(box, { target: { value: '## Work context\nThe user is a doctor.' } })
    fireEvent.click(screen.getByRole('button', { name: 'Save summary' }))
    expect(hoisted.setSummary).toHaveBeenCalledWith('## Work context\nThe user is a doctor.', 'p1')
    expect(await screen.findByText('The user is a doctor.')).toBeTruthy()
    expect(screen.getByText('Edited by you 2026-11-09')).toBeTruthy()
  })

  it('regenerates now and applies a described change', async () => {
    hoisted.summary.mockResolvedValue(summary())
    hoisted.regenerateSummary.mockResolvedValue(summary({ text: '## Top of mind\nStarting a new job.' }))
    hoisted.reviseSummary.mockResolvedValue(summary({ text: '## Top of mind\nNothing in particular.', edited: true }))
    render(<MemorySummary onError={vi.fn()} />)
    await screen.findByText('The user is a nurse in Lyon.')
    fireEvent.click(screen.getByRole('button', { name: /Regenerate/ }))
    expect(hoisted.regenerateSummary).toHaveBeenCalledWith(undefined)
    expect(await screen.findByText('Starting a new job.')).toBeTruthy()
    fireEvent.change(screen.getByRole('textbox', { name: 'Tell Neru what to change in the summary' }), { target: { value: 'forget the new job' } })
    fireEvent.click(screen.getByRole('button', { name: 'Apply' }))
    expect(hoisted.reviseSummary).toHaveBeenCalledWith('forget the new job', undefined)
    expect(await screen.findByText('Nothing in particular.')).toBeTruthy()
  })

  it('explains an empty summary and keeps regenerate off when generation is off', async () => {
    hoisted.summary.mockResolvedValue(summary({ text: '', updatedAt: 0, generatedAt: 0, enabled: false }))
    render(<MemorySummary onError={vi.fn()} />)
    expect(await screen.findByText(/Memory from chat history is off/)).toBeTruthy()
    expect(screen.getByText('Not written yet')).toBeTruthy()
    expect((screen.getByRole('button', { name: /Regenerate/ }) as HTMLButtonElement).disabled).toBe(true)
  })

  it('reports a failed regeneration', async () => {
    const onError = vi.fn()
    hoisted.summary.mockResolvedValue(summary())
    hoisted.regenerateSummary.mockRejectedValue('There are no past chats to learn from yet.')
    render(<MemorySummary onError={onError} />)
    await screen.findByText('The user is a nurse in Lyon.')
    fireEvent.click(screen.getByRole('button', { name: /Regenerate/ }))
    await waitFor(() => expect(onError).toHaveBeenCalledWith('There are no past chats to learn from yet.'))
    expect(screen.getByText('The user is a nurse in Lyon.')).toBeTruthy()
  })
})
