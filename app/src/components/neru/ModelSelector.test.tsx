import { act, fireEvent, render, screen } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Effort, ModelInfo } from '../../types'
import { ModelSelector } from './ModelSelector'

const model = (id: string, extra: Partial<ModelInfo> = {}): ModelInfo => ({ id, kind: 'chat', modalities: { input: ['text'], output: ['text'] }, vision: false, tools: true, reasoning: false, free: true, verified: 'unknown', source: 'metadata', contextWindow: 131_072, ...extra })
const models = [
  model('qwen/qwen3-coder-480b', { kind: 'code' }),
  model('deepseek-ai/deepseek-v4.1-flash', { reasoning: true }),
  model('z-ai/glm-5.3', { reasoning: true }),
  model('nvidia/nemotron-ultra', { verified: 'unavailable' }),
]

function setup(props: Partial<{ value: string; effort: Effort; effortSupported: boolean }> = {}) {
  const onChange = vi.fn()
  const onEffortChange = vi.fn()
  render(<ModelSelector models={models} value={props.value ?? models[0].id} onChange={onChange} providerId="nvidia" effort={props.effort ?? 'auto'} effortSupported={props.effortSupported ?? false} onEffortChange={onEffortChange} />)
  return { onChange, onEffortChange, trigger: screen.getByRole('button', { name: /^Model:/ }) }
}

/** Where the trigger sits on screen; jsdom has no layout. */
function place(top: number) {
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({ top, bottom: top + 32, left: 600, right: 760, width: 160, height: 32, x: 600, y: top, toJSON: () => ({}) })
}

describe('ModelSelector', () => {
  beforeEach(() => { localStorage.clear(); vi.restoreAllMocks(); Object.defineProperty(window, 'innerHeight', { value: 900, configurable: true }) })

  it('shows the current model and opens a searchable list', () => {
    place(820)
    const { trigger } = setup()
    expect(trigger.textContent).toContain('qwen3-coder-480b')
    fireEvent.click(trigger)
    expect(screen.getByRole('listbox', { name: 'Models' })).toBeTruthy()
    expect(screen.getAllByRole('option')).toHaveLength(3)
  })

  it('filters by search and picks with Enter', () => {
    place(820)
    const { trigger, onChange, onEffortChange } = setup()
    fireEvent.click(trigger)
    fireEvent.change(screen.getByRole('combobox', { name: 'Search models' }), { target: { value: 'glm' } })
    expect(screen.getAllByRole('option').map(option => option.textContent)).toEqual([expect.stringContaining('glm-5.3')])
    fireEvent.keyDown(screen.getByRole('combobox', { name: 'Search models' }), { key: 'Enter' })
    expect(onChange).toHaveBeenCalledWith('z-ai/glm-5.3')
    expect(onEffortChange).toHaveBeenCalledWith('auto')
    expect(screen.queryByRole('listbox')).toBeNull()
  })

  it('moves with the arrow keys and closes with Escape', () => {
    place(820)
    const { trigger, onChange } = setup()
    fireEvent.click(trigger)
    const search = screen.getByRole('combobox', { name: 'Search models' })
    fireEvent.keyDown(search, { key: 'ArrowDown' })
    fireEvent.keyDown(search, { key: 'Enter' })
    expect(onChange).toHaveBeenCalledWith('deepseek-ai/deepseek-v4.1-flash')
    fireEvent.click(trigger)
    fireEvent.keyDown(search.isConnected ? search : screen.getByRole('combobox'), { key: 'Escape' })
    expect(screen.queryByRole('listbox')).toBeNull()
  })

  it('opens below the trigger when there is no room above, and never past the window', () => {
    place(120)
    const { trigger } = setup()
    fireEvent.click(trigger)
    const popup = document.querySelector('.ms-popup') as HTMLElement
    expect(popup.classList.contains('down')).toBe(true)
    expect(popup.style.top).toBe('160px')
    const panel = document.querySelector('.ms-panel') as HTMLElement
    expect(parseInt(panel.style.maxHeight)).toBeLessThanOrEqual(900 - 152 - 16)
  })

  it('opens above when the composer is low, sized to the room under the title bar', () => {
    place(820)
    const { trigger } = setup()
    fireEvent.click(trigger)
    const popup = document.querySelector('.ms-popup') as HTMLElement
    expect(popup.classList.contains('down')).toBe(false)
    expect(popup.style.bottom).toBe(`${900 - 820 + 8}px`)
    // The menu lives in document.body, so no panel can clip it.
    expect(popup.parentElement).toBe(document.body)
  })

  it('remembers reasoning per model and applies it when the model is picked', () => {
    place(820)
    vi.useFakeTimers()
    const { trigger, onChange, onEffortChange } = setup()
    fireEvent.click(trigger)
    fireEvent.mouseEnter(screen.getAllByRole('option')[1])
    act(() => { vi.advanceTimersByTime(200) })
    fireEvent.click(screen.getByRole('radio', { name: 'High' }))
    fireEvent.click(screen.getAllByRole('option')[1])
    vi.useRealTimers()
    expect(onChange).toHaveBeenCalledWith('deepseek-ai/deepseek-v4.1-flash')
    expect(onEffortChange).toHaveBeenLastCalledWith('high')
  })

  it('hides unavailable models until asked', () => {
    place(820)
    const { trigger } = setup()
    fireEvent.click(trigger)
    expect(screen.queryByText('nemotron-ultra')).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: /Show unavailable|Show hidden/ }))
    expect(screen.getByText('nemotron-ultra')).toBeTruthy()
  })
})
