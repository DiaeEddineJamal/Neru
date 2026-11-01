import { describe, expect, it } from 'vitest'
import type { ModelInfo } from '../types'
import { describe as describeModel, familyOf, hostOf, metricsFor, modelLogo } from './modelFamily'

const model = (id: string, extra: Partial<ModelInfo> = {}): ModelInfo => ({ id, kind: 'chat', modalities: { input: ['text'], output: ['text'] }, vision: false, tools: true, reasoning: false, free: true, verified: 'unknown', source: 'metadata', ...extra })

describe('model families', () => {
  it('recognises makers from model ids', () => {
    expect(familyOf('qwen/qwen3-coder-480b').maker).toBe('Alibaba')
    expect(familyOf('openai/gpt-oss-120b').label).toBe('gpt-oss')
    expect(familyOf('google/gemma-4-31b-it').label).toBe('Gemma')
    expect(familyOf('gemini-3.8-flash').label).toBe('Gemini')
    expect(familyOf('mistralai/devstral-2').maker).toBe('Mistral AI')
    expect(familyOf('moonshotai/kimi-k2.6').label).toBe('Kimi')
    expect(familyOf('something-unknown').maker).toBe('')
  })
  it('gives known families a logo, with theme-following mono logos for dark ones', () => {
    expect(modelLogo('deepseek-ai/deepseek-v4')?.url).toBeTruthy()
    expect(modelLogo('openai/gpt-oss-20b')?.mono).toBe(true)
    expect(modelLogo('moonshotai/kimi-k2.6')?.mono).toBe(true)
    expect(modelLogo('mystery-model')).toBeNull()
  })
  it('names the host provider', () => {
    expect(hostOf('nvidia').name).toBe('NVIDIA NIM')
    expect(hostOf('nvidia').logo).not.toBeNull()
  })
})

describe('metrics', () => {
  const list = [model('qwen/qwen3-coder-480b', { contextWindow: 262_144 }), model('meta/llama-3.1-8b-instruct', { contextWindow: 131_072 }), model('x/no-context', { free: false })]
  const measure = metricsFor(list, 'nvidia')
  it('keeps every bar on the ten-segment scale', () => {
    for (const info of list) {
      for (const value of Object.values(measure(info).metrics)) {
        expect(value).toBeGreaterThanOrEqual(0)
        expect(value).toBeLessThanOrEqual(10)
      }
    }
  })
  it('ranks a large coder above a small model on intelligence and below it on speed', () => {
    const big = measure(list[0]).metrics
    const small = measure(list[1]).metrics
    expect(big.intelligence).toBeGreaterThan(small.intelligence)
    expect(small.speed).toBeGreaterThan(big.speed)
  })
  it('says when a bar is an estimate and when context is unknown', () => {
    expect(measure(list[0]).notes.intelligence).toMatch(/Estimated/)
    expect(measure(list[2]).metrics.context).toBe(0)
    expect(measure(list[2]).metrics.cost).toBeLessThan(10)
  })
  it('describes tool support honestly', () => {
    expect(describeModel(model('a/b', { tools: false }), 'groq')).toMatch(/cannot call tools/)
  })
})
