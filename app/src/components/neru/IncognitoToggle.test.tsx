import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { IncognitoState } from '@/lib/memoryApi'

const hoisted = vi.hoisted(() => ({ incognito: vi.fn(), setIncognito: vi.fn() }))
vi.mock('@/lib/memoryApi', () => ({ memoryApi: hoisted }))
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true }))

import { IncognitoToggle } from './IncognitoToggle'

const state = (extra: Partial<IncognitoState> = {}): IncognitoState => ({ next: false, active: false, activeId: 's1', activeEmpty: true, ...extra })

describe('IncognitoToggle', () => {
  beforeEach(() => { Object.values(hoisted).forEach(fn => fn.mockReset()) })

  it('turns incognito on for the chat about to start', async () => {
    hoisted.incognito.mockResolvedValue(state())
    hoisted.setIncognito.mockResolvedValue(state({ active: true }))
    render(<IncognitoToggle home />)
    const toggle = await screen.findByRole('button', { name: /Incognito/ })
    expect(toggle.getAttribute('aria-pressed')).toBe('false')
    fireEvent.click(toggle)
    expect(hoisted.setIncognito).toHaveBeenCalledWith(true)
    await waitFor(() => expect(toggle.getAttribute('aria-pressed')).toBe('true'))
  })

  it('turns it off again', async () => {
    hoisted.incognito.mockResolvedValue(state({ next: true, activeEmpty: false }))
    hoisted.setIncognito.mockResolvedValue(state())
    render(<IncognitoToggle home />)
    const toggle = await screen.findByRole('button', { name: /Incognito/ })
    expect(toggle.getAttribute('aria-pressed')).toBe('true')
    fireEvent.click(toggle)
    expect(hoisted.setIncognito).toHaveBeenCalledWith(false)
    await waitFor(() => expect(toggle.getAttribute('aria-pressed')).toBe('false'))
  })

  it('shows a mark in an incognito chat and nothing in a saved one', async () => {
    hoisted.incognito.mockResolvedValue(state({ active: true, activeEmpty: false }))
    const { unmount } = render(<IncognitoToggle home={false} />)
    expect(await screen.findByRole('img', { name: 'Incognito chat' })).toBeTruthy()
    expect(screen.queryByRole('button')).toBeNull()
    unmount()
    hoisted.incognito.mockResolvedValue(state({ activeEmpty: false }))
    const { container } = render(<IncognitoToggle home={false} />)
    await waitFor(() => expect(hoisted.incognito).toHaveBeenCalledTimes(2))
    expect(container.textContent).toBe('')
  })
})
