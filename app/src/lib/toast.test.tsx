import { act, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { Toaster } from '../components/neru/Toaster'
import { actionLabels } from './friendlyError'
import { dismissAll, readingTime, toast, useToasts } from './toast'

function Probe({ onRender }: { onRender: (count: number) => void }) {
  onRender(useToasts().length)
  return null
}

afterEach(() => { act(() => dismissAll()); vi.useRealTimers() })

describe('toast store', () => {
  it('merges repeats into one toast with a count', () => {
    let count = 0
    render(<Probe onRender={n => { count = n }} />)
    act(() => { toast.info('Saved'); toast.info('Saved'); toast.info('Saved') })
    expect(count).toBe(1)
    render(<Toaster />)
    expect(screen.getByText('×3')).toBeTruthy()
  })

  it('keeps at most three, dropping the oldest non-error first', () => {
    render(<Toaster />)
    act(() => { toast.error('HTTP 401 invalid api key'); toast.info('one'); toast.info('two'); toast.info('three') })
    expect(screen.queryByText('one')).toBeNull()
    expect(screen.getByText('two')).toBeTruthy()
    expect(screen.getByText('The provider rejected your API key.')).toBeTruthy()
  })

  it('errors stay until dismissed; other toasts leave after their reading time', () => {
    vi.useFakeTimers()
    render(<Toaster />)
    act(() => { toast.error('HTTP 429 Too Many Requests'); toast.success('Paper saved') })
    act(() => { vi.advanceTimersByTime(readingTime('Paper saved') + 400) })
    expect(screen.queryByText('Paper saved')).toBeNull()
    expect(screen.getByRole('alert').textContent).toContain('usage limit')
  })

  it('reading time grows with the text and stays between 4 and 12 seconds', () => {
    expect(readingTime('Saved')).toBe(4_000)
    expect(readingTime('word '.repeat(20))).toBeGreaterThan(readingTime('a few words here'))
    expect(readingTime('word '.repeat(200))).toBe(12_000)
  })

  it('a key replaces the earlier toast instead of stacking', () => {
    render(<Toaster />)
    act(() => { toast.info('Downloading 10%', { key: 'update' }); toast.info('Downloading 60%', { key: 'update' }) })
    expect(screen.queryByText('Downloading 10%')).toBeNull()
    expect(screen.getByText('Downloading 60%')).toBeTruthy()
  })
})

describe('Toaster', () => {
  it('announces errors assertively and other toasts politely', () => {
    render(<Toaster />)
    act(() => { toast.info('Attached notes.md'); toast.error('connection refused') })
    expect(screen.getByRole('status').getAttribute('aria-live')).toBe('polite')
    expect(screen.getByRole('alert').getAttribute('aria-live')).toBe('assertive')
  })

  it('runs the one-click fix and closes', () => {
    const onAction = vi.fn()
    vi.useFakeTimers()
    render(<Toaster />)
    act(() => { toast.error('HTTP 429 Too Many Requests', onAction, actionLabels) })
    fireEvent.click(screen.getByRole('button', { name: actionLabels.retry }))
    expect(onAction).toHaveBeenCalledWith('retry')
    act(() => { vi.advanceTimersByTime(200) })
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('dismisses with the close button or Escape, and shows details on request', () => {
    vi.useFakeTimers()
    render(<Toaster />)
    act(() => { toast.error('Provider said: HTTP 500 upstream exploded') })
    fireEvent.click(screen.getByRole('button', { name: 'Details' }))
    expect(screen.getByText(/upstream exploded/)).toBeTruthy()
    fireEvent.keyDown(screen.getByRole('alert'), { key: 'Escape' })
    act(() => { vi.advanceTimersByTime(200) })
    expect(screen.queryByRole('alert')).toBeNull()
  })

  it('renders nothing when there is nothing to say', () => {
    const { container } = render(<Toaster />)
    act(() => { toast.info('   ') })
    expect(container.querySelector('.toaster')).toBeNull()
  })
})
