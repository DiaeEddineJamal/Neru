import { render } from '@testing-library/react'
import { describe, expect, it } from 'vitest'
import { SoundBars } from './SoundBars'

describe('SoundBars', () => {
  it('draws a row of bars, hidden from screen readers', () => {
    const { container } = render(<SoundBars level={() => 0.5} />)
    const host = container.querySelector('.sound-bars')!
    expect(host.getAttribute('aria-hidden')).toBe('true')
    expect(host.querySelectorAll('i')).toHaveLength(28)
  })
  it('marks itself paused', () => {
    const { container } = render(<SoundBars level={() => 0} paused bars={10} />)
    expect(container.querySelector('.sound-bars.paused')?.children).toHaveLength(10)
  })
})
