import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import type { LocalAdvice, OllamaStatus } from '../../types'
import { LocalModelsView } from './LocalModels'

const advice: LocalAdvice = {
  hardware: { ramGb: 31.7, cpu: 'Intel i7', threads: 8, gpus: [{ name: 'Intel(R) Iris(R) Xe Graphics', vramGb: 0, dedicated: false }], unified: false, modelsDir: 'C:/Users/me/.ollama/models', diskFreeGb: 120 },
  picks: [
    { role: 'agent', tag: 'qwen3-coder:30b', sizeGb: 19, runsOn: 'cpu', tokensPerSecond: 14 },
    { role: 'helper', tag: 'gemma3:4b', sizeGb: 3.3, runsOn: 'cpu', tokensPerSecond: 8 },
    { role: 'complete', tag: 'qwen2.5-coder:1.5b', sizeGb: 1, runsOn: 'cpu', tokensPerSecond: 28 },
  ],
  agent: true,
  verdict: 'This computer can run qwen3-coder:30b for the agent itself, slowly.',
}
const running: OllamaStatus = { installed: true, running: true, version: '0.12.9', baseUrl: 'http://127.0.0.1:11434/v1', models: [{ name: 'gemma3:4b', sizeGb: 3.1, parameters: '4.3B', quantization: 'Q4_K_M' }], pulling: [] }
const handlers = () => ({ onPull: vi.fn(), onCancel: vi.fn(), onDelete: vi.fn(), onStart: vi.fn(), onRefresh: vi.fn() })

describe('LocalModelsView', () => {
  it('recommends what fits, marks what is installed and downloads the rest', () => {
    const on = handlers()
    render(<LocalModelsView advice={advice} status={running} pulls={{}} error={null} busy={false} {...on} />)
    expect(screen.getByText('32 GB memory · no graphics card with its own memory · 120 GB free for models')).toBeTruthy()
    expect(screen.getByText('Ollama 0.12.9 is running.')).toBeTruthy()
    expect(screen.queryByText('qwen2.5-coder:1.5b'), 'roles Neru does not use yet stay hidden').toBeNull()
    fireEvent.click(screen.getByRole('button', { name: 'Download qwen3-coder:30b' }))
    expect(on.onPull).toHaveBeenCalledWith('qwen3-coder:30b')
    expect(screen.getByText('Installed')).toBeTruthy()
  })

  it('shows download progress with a cancel button', () => {
    const on = handlers()
    render(<LocalModelsView advice={advice} status={running} pulls={{ 'qwen3-coder:30b': { tag: 'qwen3-coder:30b', status: 'downloading', completed: 5 * 1_073_741_824, total: 19 * 1_073_741_824, done: false, error: null } }} error={null} busy={false} {...on} />)
    expect(screen.getByText('5.0 GB of 19 GB')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Cancel download' }))
    expect(on.onCancel).toHaveBeenCalledWith('qwen3-coder:30b')
  })

  it('asks before removing a model', () => {
    const on = handlers()
    render(<LocalModelsView advice={advice} status={running} pulls={{}} error={null} busy={false} {...on} />)
    fireEvent.click(screen.getByRole('button', { name: 'Remove gemma3:4b' }))
    expect(on.onDelete).not.toHaveBeenCalled()
    fireEvent.click(screen.getByRole('button', { name: 'Remove' }))
    expect(on.onDelete).toHaveBeenCalledWith('gemma3:4b')
  })

  it('points to the installer, or offers to start Ollama', () => {
    const on = handlers()
    const { rerender } = render(<LocalModelsView advice={advice} status={{ ...running, installed: false, running: false, models: [] }} pulls={{}} error={null} busy={false} {...on} />)
    expect(screen.getByRole('link', { name: /Get Ollama/ }).getAttribute('href')).toBe('https://ollama.com/download')
    rerender(<LocalModelsView advice={advice} status={{ ...running, running: false, models: [] }} pulls={{}} error={null} busy={false} {...on} />)
    fireEvent.click(screen.getByRole('button', { name: 'Start Ollama' }))
    expect(on.onStart).toHaveBeenCalled()
  })
})
