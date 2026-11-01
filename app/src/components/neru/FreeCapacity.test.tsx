import { render, screen } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import type { ProviderCapacity } from '../../types'
import { FreeCapacityList } from './FreeCapacity'

const plan = (over: Partial<ProviderCapacity>): ProviderCapacity => ({ providerId: 'nvidia', current: false, usedToday: 0, limit: null, left: null, restingModels: 0, backIn: null, resetsIn: '5h0m', ...over })

describe('FreeCapacityList', () => {
  it('adds up what is left today, marking plans whose allowance is unknown', () => {
    render(<FreeCapacityList plans={[
      plan({ providerId: 'modelscope', usedToday: 40, limit: 2000, left: 1960, current: true }),
      plan({ providerId: 'gemini', usedToday: 12, restingModels: 2, backIn: '3h10m' }),
    ]} />)
    expect(screen.getByText('1,960+ requests left')).toBeTruthy()
    expect(screen.getByText('40 used · 1,960 left')).toBeTruthy()
    expect(screen.getByText('12 used')).toBeTruthy()
    expect(screen.getByText('2 models resting after a limit, first back in 3h10m')).toBeTruthy()
    expect(screen.getByText('Its day starts over in 5h0m')).toBeTruthy()
  })

  it('shows nothing without a free plan', () => {
    const { container } = render(<FreeCapacityList plans={[]} />)
    expect(container.textContent).toBe('')
  })
})
