import { fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { MemoryStore } from '../../types'
import { api } from '../../api'
import { MemorySettings } from './MemorySettings'

const store = (): MemoryStore => ({
  settings: { referenceMemories: true, learnFromChats: true, referenceChatHistory: false },
  memories: [
    { id: 'a1', text: 'Prefers metric units', category: 'preferences', createdAt: 1, updatedAt: Date.UTC(2026, 9, 3), source: 'user' },
    { id: 'b2', text: 'Studies law in Lyon', category: 'about', createdAt: 1, updatedAt: Date.UTC(2026, 9, 5), source: 'noticed', sessionId: 's1' },
  ],
})

vi.mock('../../api', () => ({
  api: {
    memoryStore: vi.fn(),
    setMemoryPrefs: vi.fn(),
    addMemory: vi.fn(),
    editMemory: vi.fn(),
    deleteMemory: vi.fn(),
    clearMemories: vi.fn(),
    openMemoryFile: vi.fn(() => Promise.resolve('')),
  },
}))

const mocked = vi.mocked(api)

describe('MemorySettings', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mocked.memoryStore.mockResolvedValue(store())
  })

  it('lists memories newest first with category, source and date, and filters them', async () => {
    render(<MemorySettings onError={vi.fn()} onNotice={vi.fn()} />)
    await waitFor(() => expect(screen.getByText('Studies law in Lyon')).toBeTruthy())
    const items = screen.getAllByRole('listitem')
    expect(items[0].textContent).toContain('Studies law in Lyon')
    expect(items[0].textContent).toContain('Neru noticed')
    expect(items[0].textContent).toContain('About you')
    expect(items[1].textContent).toContain('You said')
    expect(items[1].textContent).toContain('2026')
    fireEvent.change(screen.getByRole('searchbox', { name: 'Search memories' }), { target: { value: 'metric' } })
    expect(screen.queryByText('Studies law in Lyon')).toBeNull()
    expect(screen.getByText('Prefers metric units')).toBeTruthy()
  })

  it('turns settings on and off', async () => {
    mocked.setMemoryPrefs.mockImplementation(settings => Promise.resolve({ ...store(), settings }))
    render(<MemorySettings onError={vi.fn()} onNotice={vi.fn()} />)
    const history = await screen.findByRole('switch', { name: 'Reference chat history' })
    expect(history.getAttribute('aria-checked')).toBe('false')
    fireEvent.click(history)
    expect(mocked.setMemoryPrefs).toHaveBeenCalledWith({ referenceMemories: true, learnFromChats: true, referenceChatHistory: true })
    await waitFor(() => expect(history.getAttribute('aria-checked')).toBe('true'))
  })

  it('adds, edits and deletes a memory', async () => {
    const next = store()
    mocked.addMemory.mockResolvedValue({ ...next, memories: [...next.memories, { id: 'c3', text: 'Uses Linux', category: 'about', createdAt: 9, updatedAt: Date.UTC(2026, 9, 9), source: 'user' }] })
    mocked.editMemory.mockResolvedValue({ ...next, memories: [{ ...next.memories[0], text: 'Prefers imperial units' }, next.memories[1]] })
    mocked.deleteMemory.mockResolvedValue({ ...next, memories: [next.memories[0]] })
    render(<MemorySettings onError={vi.fn()} onNotice={vi.fn()} />)
    await screen.findByText('Prefers metric units')

    fireEvent.click(screen.getByRole('button', { name: /Add memory/ }))
    fireEvent.change(screen.getByRole('textbox', { name: 'Memory' }), { target: { value: 'Uses Linux' } })
    fireEvent.click(screen.getByRole('button', { name: 'Add' }))
    expect(mocked.addMemory).toHaveBeenCalledWith('Uses Linux', 'about')
    await screen.findByText('Uses Linux')

    fireEvent.click(screen.getByRole('button', { name: 'Edit: Prefers metric units' }))
    const input = screen.getByRole('textbox', { name: 'Memory' })
    fireEvent.change(input, { target: { value: 'Prefers imperial units' } })
    fireEvent.submit(input.closest('form')!)
    expect(mocked.editMemory).toHaveBeenCalledWith('a1', 'Prefers imperial units', 'preferences')
    await screen.findByText('Prefers imperial units')

    fireEvent.click(screen.getByRole('button', { name: 'Delete: Studies law in Lyon' }))
    expect(mocked.deleteMemory).toHaveBeenCalledWith('b2')
    await waitFor(() => expect(screen.queryByText('Studies law in Lyon')).toBeNull())
  })

  it('asks before deleting everything', async () => {
    const onNotice = vi.fn()
    mocked.clearMemories.mockResolvedValue({ ...store(), memories: [] })
    render(<MemorySettings onError={vi.fn()} onNotice={onNotice} />)
    await screen.findByText('Prefers metric units')
    fireEvent.click(screen.getByRole('button', { name: /Delete all/ }))
    expect(mocked.clearMemories).not.toHaveBeenCalled()
    const dialog = screen.getByRole('alertdialog')
    expect(dialog.textContent).toContain('Delete all 2 saved memories?')
    fireEvent.click(within(dialog).getByRole('button', { name: 'Delete all' }))
    expect(mocked.clearMemories).toHaveBeenCalled()
    await waitFor(() => expect(screen.getByText(/Nothing saved yet/)).toBeTruthy())
    expect(onNotice).toHaveBeenCalledWith('Deleted all saved memories.')
  })
})
