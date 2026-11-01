import { describe, expect, it } from 'vitest'
import { providerForKey, providerPresets, untouchedDefault } from './providerCatalog'

describe('providerForKey', () => {
  it('recognises a pasted key by its prefix', () => {
    expect(providerForKey('nvapi-abc123')).toBe('nvidia')
    expect(providerForKey('  AIzaSyExample  ')).toBe('gemini')
    expect(providerForKey('sk-or-v1-abc')).toBe('openrouter')
    expect(providerForKey('gsk_abc')).toBe('groq')
  })

  it('names only providers that exist, and guesses nothing for plain keys', () => {
    for (const key of ['nvapi-', 'AIza', 'sk-or-', 'gsk_', 'sk-ant-', 'hf_', 'xai-']) {
      expect(providerPresets.some(preset => preset.id === providerForKey(`${key}x`))).toBe(true)
    }
    expect(providerForKey('3f2a9c0e1b')).toBeUndefined()
    expect(providerForKey('sk-proj-abc')).toBeUndefined()
  })
})

describe('untouchedDefault', () => {
  const fresh = { providerId: 'local', baseUrl: 'http://localhost:3001/v1', model: 'auto', hasKey: false }
  it('is the gateway a fresh install points at, and nothing the user set', () => {
    expect(untouchedDefault(fresh)).toBe(true)
    expect(untouchedDefault({ ...fresh, baseUrl: 'http://localhost:3001/v1/' })).toBe(true)
    expect(untouchedDefault({ ...fresh, model: 'qwen3-coder' })).toBe(false)
    expect(untouchedDefault({ ...fresh, baseUrl: 'http://localhost:8080/v1' })).toBe(false)
    expect(untouchedDefault({ ...fresh, providerId: 'ollama' })).toBe(false)
  })
})
