import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import type { BuiltinAdvice, BuiltinPick, BuiltinStatus } from '../../types'
import { BuiltinModelsView, type BuiltinViewProps } from './BuiltinModels'

const pick = (over: Partial<BuiltinPick>): BuiltinPick => ({
  id: 'qwen3-4b-instruct-2507', name: 'Qwen3 4B Instruct', repo: 'unsloth/Qwen3-4B-Instruct-2507-GGUF', roles: ['helper', 'chat'], note: 'Small and capable.',
  quant: 'Q8_0', file: 'Qwen3-4B-Instruct-2507-Q8_0.gguf', bytes: 4_280_000_000, runsOn: 'cpu', gpuPercent: 0, tokensPerSecond: 7,
  fit: 'good', fitLabel: 'Runs on the processor, about 7 tokens a second', quality: 'Near the original', context: 65_536, recommended: true, alternatives: ['Q6_K', 'Q4_K_M'], ...over,
})
const advice: BuiltinAdvice = {
  hardware: { ramGb: 31.7, cpu: 'Intel i7', threads: 8, gpus: [{ name: 'Intel(R) Iris(R) Xe Graphics', vramGb: 0, dedicated: false }], unified: false, modelsDir: '', diskFreeGb: null },
  backend: 'cpu', verdict: 'Qwen3 Coder can drive Code on this computer.', diskFreeGb: 120,
  models: [pick({}), pick({ id: 'devstral', name: 'Devstral Small 24B', fit: 'no', recommended: false, fitLabel: 'Too large for this computer', alternatives: [] })],
}
const status: BuiltinStatus = {
  engine: { tag: 'b12230', backend: 'cpu', path: 'llama-server.exe' }, backend: 'cpu', baseUrl: 'http://127.0.0.1:39281/v1',
  models: [{ id: 'qwen3-0.6b:Q4_K_M', name: 'Qwen3 0.6B', repo: 'unsloth/Qwen3-0.6B-GGUF', file: 'Qwen3-0.6B-Q4_K_M.gguf', quant: 'Q4_K_M', bytes: 396_705_472, path: 'x', catalogId: 'qwen3-0.6b', roles: ['helper'] }],
  downloading: [], loaded: [{ id: 'qwen3-0.6b:Q4_K_M', port: 5000, context: 40_960, backend: 'cpu', gpuLayers: null, idleSeconds: 3, busy: false }],
  settings: { keepWarmMinutes: 15, context: 0, backend: 'auto', maxLoaded: 2 }, speeds: {}, modelsDir: 'D:/models',
}
const props = (over: Partial<BuiltinViewProps> = {}): BuiltinViewProps => ({
  advice, status, downloads: {}, engine: null, busy: {}, error: null,
  onDownload: vi.fn(), onCancel: vi.fn(), onDelete: vi.fn(), onLoad: vi.fn(), onUnload: vi.fn(), onSpeed: vi.fn(), onInstallEngine: vi.fn(), onSettings: vi.fn(), onRefresh: vi.fn(), ...over,
})

describe('BuiltinModelsView', () => {
  it('shows the machine, the engine and what fits, and downloads the chosen quantization', () => {
    const on = props()
    render(<BuiltinModelsView {...on} />)
    expect(screen.getByText('32 GB memory · no graphics card with its own memory · 120 GB free for models')).toBeTruthy()
    expect(screen.getByText('llama.cpp b12230 on the processor is ready.')).toBeTruthy()
    expect(screen.getByText('Recommended')).toBeTruthy()
    expect(screen.getByText('Too large here: Devstral Small 24B.')).toBeTruthy()
    fireEvent.change(screen.getByLabelText('Quantization for Qwen3 4B Instruct'), { target: { value: 'Q4_K_M' } })
    fireEvent.click(screen.getByRole('button', { name: 'Download Qwen3 4B Instruct' }))
    expect(on.onDownload).toHaveBeenCalledWith('unsloth/Qwen3-4B-Instruct-2507-GGUF', 'Qwen3-4B-Instruct-2507-Q4_K_M.gguf')
  })

  it('lists downloaded models with their loaded state and asks before removing', () => {
    const on = props()
    render(<BuiltinModelsView {...on} />)
    expect(screen.getByText(/loaded · 40K context · the processor/)).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Unload' }))
    expect(on.onUnload).toHaveBeenCalledWith('qwen3-0.6b:Q4_K_M')
    fireEvent.click(screen.getByRole('button', { name: 'Remove qwen3-0.6b:Q4_K_M' }))
    fireEvent.click(screen.getByRole('button', { name: 'Remove' }))
    expect(on.onDelete).toHaveBeenCalledWith('qwen3-0.6b:Q4_K_M')
  })

  it('shows download progress with cancel, and offers the engine when it is missing', () => {
    const key = 'unsloth/Qwen3-4B-Instruct-2507-GGUF/Qwen3-4B-Instruct-2507-Q8_0.gguf'
    const on = props({ status: { ...status, engine: null }, downloads: { [key]: { key, id: '', status: 'downloading', completed: 1_073_741_824, total: 4_294_967_296, speed: 5_242_880, done: false, error: null } } })
    render(<BuiltinModelsView {...on} />)
    expect(screen.getByText('1.0 GB of 4.0 GB · 5.0 MB/s')).toBeTruthy()
    fireEvent.click(screen.getByRole('button', { name: 'Cancel download' }))
    expect(on.onCancel).toHaveBeenCalledWith(key)
    fireEvent.click(screen.getByRole('button', { name: 'Get the engine' }))
    expect(on.onInstallEngine).toHaveBeenCalled()
  })
})
