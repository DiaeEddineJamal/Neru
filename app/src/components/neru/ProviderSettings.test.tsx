import { fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import type { ModelInfo, ProviderView } from '../../types'
import { ProviderSettings } from './ProviderSettings'

vi.mock('../../api', () => ({ api: { providerAllowance: vi.fn(() => Promise.resolve(null)), onModelCheck: vi.fn(() => Promise.resolve(() => undefined)) } }))

const provider: ProviderView = { providerId: 'nvidia', apiFormat: 'openai-chat', baseUrl: 'https://integrate.api.nvidia.com/v1', model: 'a/ok-model', configured: true, hasKey: true }
const model = (id: string, verified: ModelInfo['verified']): ModelInfo => ({ id, kind: 'code', modalities: { input: ['text'], output: ['text'] }, vision: false, tools: true, reasoning: false, free: true, verified, source: 'metadata' })

function setup(overrides: Partial<Parameters<typeof ProviderSettings>[0]> = {}) {
  const props = {
    provider, savedKeys: ['nvidia'], providerId: 'nvidia', onProvider: vi.fn(), format: 'openai-chat' as const, onFormat: vi.fn(), baseUrl: provider.baseUrl, onBaseUrl: vi.fn(),
    apiKey: '', onApiKey: vi.fn(), model: 'a/ok-model', onModel: vi.fn(), models: [] as ModelInfo[], onModels: vi.fn(), loading: false, error: '',
    engine: null, onEngine: vi.fn(), onSave: vi.fn(), onForgetKeys: vi.fn(), ...overrides,
  }
  render(<ProviderSettings {...props} />)
  return props
}

describe('ProviderSettings', () => {
  it('groups providers by cost, keeps paid ones folded, and picks a card on click', () => {
    const props = setup()
    expect(screen.getByRole('radiogroup', { name: 'Free' })).toBeTruthy()
    expect(screen.queryByRole('radio', { name: /^OpenAI GPT/ })).toBeNull()
    fireEvent.click(screen.getByRole('button', { name: /paid providers/ }))
    fireEvent.click(screen.getByRole('radio', { name: /^OpenAI GPT/ }))
    expect(props.onProvider).toHaveBeenCalledWith('openai')
    expect(screen.getByRole('radio', { name: /NVIDIA NIM/ }).getAttribute('aria-checked')).toBe('true')
  })

  it('shows how to get a key and says plainly when the key works', () => {
    setup({ models: [model('a/ok-model', 'ok'), model('b/new-model', 'unknown')] })
    expect(screen.getByText(/Generate API Key/)).toBeTruthy()
    expect(screen.getByRole('status').textContent).toContain('The key works. 2 models found.')
    expect(screen.getByText('Ready to use')).toBeTruthy()
    expect(screen.getByText('Not checked yet')).toBeTruthy()
  })

  it('marks the key optional for providers that work without one, and hides the technical fields', () => {
    setup({ providerId: 'ovhcloud', savedKeys: [] })
    expect(screen.getByText('API key (optional)')).toBeTruthy()
    expect(screen.getByText('Advanced').closest('details')?.open).toBe(false)
  })

  it('turns errors into an alert and keeps the save button off until a usable model is picked', () => {
    setup({ error: 'The provider rejected your API key.', model: '' })
    expect(screen.getByRole('alert').textContent).toContain('rejected your API key')
    expect((screen.getByRole('button', { name: /^Use / }) as HTMLButtonElement).disabled).toBe(true)
  })
})
