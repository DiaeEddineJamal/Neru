import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { Select } from './Select'

const setup = (onChange = vi.fn()) => {
  render(<Select value="b" onChange={onChange} aria-label="Letter">
    <option value="a">Alpha</option>
    {[<option key="b" value="b">Bravo</option>]}
    <optgroup label="More"><option value="c">Charlie</option><option value="d" disabled>Delta</option></optgroup>
  </Select>)
  return onChange
}

describe('Select', () => {
  it('shows the chosen option and opens a styled list of every option', () => {
    setup()
    const trigger = screen.getByRole('button', { name: 'Letter' })
    expect(trigger.textContent).toContain('Bravo')
    fireEvent.click(trigger)
    expect(screen.getAllByRole('option').map(option => option.textContent)).toEqual(['Alpha', 'Bravo', 'Charlie', 'Delta'])
    expect(screen.getByText('More')).toBeTruthy()
    expect(screen.getByRole('option', { name: 'Bravo' }).getAttribute('aria-selected')).toBe('true')
  })

  it('calls onChange like a native select when an option is clicked', () => {
    const onChange = setup()
    fireEvent.click(screen.getByRole('button', { name: 'Letter' }))
    fireEvent.click(screen.getByRole('option', { name: 'Charlie' }))
    expect(onChange).toHaveBeenCalledWith({ target: { value: 'c' } })
    expect(screen.queryByRole('listbox')).toBeNull()
  })

  it('works from the keyboard and skips disabled options', () => {
    const onChange = setup()
    const trigger = screen.getByRole('button', { name: 'Letter' })
    fireEvent.keyDown(trigger, { key: 'ArrowDown' })
    fireEvent.keyDown(trigger, { key: 'ArrowDown' })
    fireEvent.keyDown(trigger, { key: 'ArrowDown' })
    fireEvent.keyDown(trigger, { key: 'Enter' })
    expect(onChange).toHaveBeenCalledWith({ target: { value: 'c' } })
    fireEvent.keyDown(trigger, { key: 'ArrowDown' })
    fireEvent.keyDown(trigger, { key: 'a' })
    fireEvent.keyDown(trigger, { key: 'Enter' })
    expect(onChange).toHaveBeenLastCalledWith({ target: { value: 'a' } })
  })

  it('closes on Escape without changing the value', () => {
    const onChange = setup()
    const trigger = screen.getByRole('button', { name: 'Letter' })
    fireEvent.click(trigger)
    fireEvent.keyDown(trigger, { key: 'Escape' })
    expect(screen.queryByRole('listbox')).toBeNull()
    expect(onChange).not.toHaveBeenCalled()
  })
})
