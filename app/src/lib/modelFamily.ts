import type { ModelInfo } from '../types'
import { providerPresets } from '../providerCatalog'

// Logos from LobeHub Icons (MIT); see assets/model-logos/NOTICE.txt.
const LOGOS = import.meta.glob<string>('../assets/model-logos/*.svg', { eager: true, query: '?url', import: 'default' })
const logoUrl = (key: string) => LOGOS[`../assets/model-logos/${key}.svg`]

/** Logos drawn in a single colour, or too dark to read on a dark surface; painted through a mask so they follow the theme. */
const MONO = new Set(['ai21', 'gemma', 'grok', 'groq', 'ibm', 'liquid', 'moonshot', 'ollama', 'openai', 'opencode', 'zai'])

export type Logo = { url: string; mono: boolean; label: string }

export type Family = {
  key: string
  label: string
  maker: string
  /** Rough capability for agentic coding, 0-100. A family baseline, adjusted by size and variant below. */
  base: number
}

/** First match wins, so specific names (codestral, gemma) come before broader ones (mistral, gemini). */
const FAMILIES: [RegExp, Family][] = [
  [/claude|anthropic/, { key: 'claude', label: 'Claude', maker: 'Anthropic', base: 92 }],
  [/gpt-oss/, { key: 'openai', label: 'gpt-oss', maker: 'OpenAI', base: 70 }],
  [/gpt|\bo[1-9]\b|o[1-9]-|codex|openai/, { key: 'openai', label: 'GPT', maker: 'OpenAI', base: 88 }],
  [/gemma/, { key: 'gemma', label: 'Gemma', maker: 'Google', base: 55 }],
  [/gemini/, { key: 'gemini', label: 'Gemini', maker: 'Google', base: 86 }],
  [/deepseek/, { key: 'deepseek', label: 'DeepSeek', maker: 'DeepSeek', base: 82 }],
  [/qwen|qwq/, { key: 'qwen', label: 'Qwen', maker: 'Alibaba', base: 78 }],
  [/kimi|moonshot/, { key: 'moonshot', label: 'Kimi', maker: 'Moonshot AI', base: 82 }],
  [/glm|zhipu|z-ai|zai-org/, { key: 'zhipu', label: 'GLM', maker: 'Z.ai', base: 80 }],
  [/minimax|abab/, { key: 'minimax', label: 'MiniMax', maker: 'MiniMax', base: 76 }],
  [/codestral|devstral|mistral|mixtral|magistral|ministral|pixtral/, { key: 'mistral', label: 'Mistral', maker: 'Mistral AI', base: 70 }],
  [/grok|x-ai|xai/, { key: 'grok', label: 'Grok', maker: 'xAI', base: 84 }],
  [/nemotron|nvidia/, { key: 'nvidia', label: 'Nemotron', maker: 'NVIDIA', base: 68 }],
  [/llama|meta-/, { key: 'meta', label: 'Llama', maker: 'Meta', base: 62 }],
  [/phi-|microsoft/, { key: 'microsoft', label: 'Phi', maker: 'Microsoft', base: 52 }],
  [/command|cohere|aya/, { key: 'cohere', label: 'Command', maker: 'Cohere', base: 58 }],
  [/granite|ibm/, { key: 'ibm', label: 'Granite', maker: 'IBM', base: 50 }],
  [/hunyuan|tencent/, { key: 'hunyuan', label: 'Hunyuan', maker: 'Tencent', base: 66 }],
  [/doubao|seed-|bytedance/, { key: 'doubao', label: 'Doubao', maker: 'ByteDance', base: 74 }],
  [/step-|stepfun/, { key: 'stepfun', label: 'Step', maker: 'StepFun', base: 66 }],
  [/ernie|baidu|wenxin/, { key: 'wenxin', label: 'ERNIE', maker: 'Baidu', base: 64 }],
  [/internlm|intern-/, { key: 'internlm', label: 'InternLM', maker: 'Shanghai AI Lab', base: 56 }],
  [/yi-|01-ai/, { key: 'yi', label: 'Yi', maker: '01.AI', base: 54 }],
  [/baichuan/, { key: 'baichuan', label: 'Baichuan', maker: 'Baichuan', base: 54 }],
  [/jamba|ai21/, { key: 'ai21', label: 'Jamba', maker: 'AI21', base: 52 }],
  [/sonar|perplexity/, { key: 'perplexity', label: 'Sonar', maker: 'Perplexity', base: 66 }],
  [/nova-|amazon/, { key: 'nova', label: 'Nova', maker: 'Amazon', base: 64 }],
  [/lfm|liquid/, { key: 'liquid', label: 'LFM', maker: 'Liquid AI', base: 48 }],
  [/solar|upstage/, { key: 'upstage', label: 'Solar', maker: 'Upstage', base: 52 }],
]

/** Where a model is served from, shown under its name: the provider's own logo when there is one. */
const HOST_LOGO: Record<string, string> = {
  nvidia: 'nvidia', modelscope: 'modelscope', gemini: 'gemini', cerebras: 'cerebras', mistral: 'mistral', openrouter: 'openrouter',
  groq: 'groq', huggingface: 'huggingface', ollama: 'ollama', opencode: 'opencode', xai: 'grok', openai: 'openai', anthropic: 'claude',
  deepseek: 'deepseek', qwen: 'qwen', kimi: 'moonshot', zai: 'zai', minimax: 'minimax', cloudflare: 'cloudflare', sambanova: 'sambanova',
}

const UNKNOWN: Family = { key: '', label: 'Model', maker: '', base: 55 }

export function familyOf(id: string): Family {
  const text = id.toLowerCase()
  return FAMILIES.find(([pattern]) => pattern.test(text))?.[1] ?? UNKNOWN
}

export function logoFor(key: string, label: string): Logo | null {
  const url = logoUrl(key)
  return url ? { url, mono: MONO.has(key), label } : null
}

export const modelLogo = (id: string) => { const family = familyOf(id); return logoFor(family.key, family.maker || family.label) }

export function hostOf(providerId: string) {
  const preset = providerPresets.find(item => item.id === providerId)
  const key = HOST_LOGO[providerId]
  return { name: preset?.name ?? providerId, logo: key ? logoFor(key, preset?.name ?? providerId) : null }
}

/** Parameter count in billions when the id names one ("70b", "8x7b", "a12b" active counts are ignored). */
function sizeOf(id: string) {
  const text = id.toLowerCase()
  const moe = text.match(/(\d+)x(\d+(?:\.\d+)?)b\b/)
  if (moe) return Number(moe[1]) * Number(moe[2])
  const plain = [...text.matchAll(/(?<![a-z])(\d+(?:\.\d+)?)b\b/g)].map(match => Number(match[1]))
  return plain.length ? Math.max(...plain) : 0
}

const FAST = /flash|mini|lite|nano|small|haiku|turbo|instant|fast|tiny|8b|7b|3b|1b/
const SLOW = /reason|thinking|r1\b|qwq|opus|pro\b|ultra|large|max\b|heavy/

export type Metrics = { intelligence: number; speed: number; context: number; cost: number }
export type MetricNotes = Record<keyof Metrics, string>

const clamp = (value: number) => Math.max(1, Math.min(10, Math.round(value)))

/** Raw estimates before they are spread over 10 segments against the rest of the list. */
function raw(info: ModelInfo, providerId: string) {
  const id = info.id.toLowerCase()
  const family = familyOf(id)
  const size = sizeOf(id)
  let intelligence = family.base
  if (size) intelligence += size >= 200 ? 6 : size >= 60 ? 2 : size >= 25 ? -4 : size >= 10 ? -12 : -22
  if (/coder|codestral|devstral|codex/.test(id)) intelligence += 4
  if (info.reasoning) intelligence += 4
  if (FAST.test(id)) intelligence -= 8
  let speed = 50
  if (FAST.test(id)) speed += 25
  if (SLOW.test(id) || info.reasoning) speed -= 18
  if (size) speed += size <= 10 ? 15 : size >= 200 ? -12 : 0
  if (providerId === 'cerebras' || providerId === 'groq' || providerId === 'sambanova') speed += 35
  if (providerId === 'ollama') speed -= 10
  return { intelligence, speed }
}

/** Ten-segment bars for every model in the list. Intelligence and speed are estimates from the model's family, size,
 *  variant and host, normalised against the best in the list; context is the provider's reported window; cost is what
 *  this key pays (free, local, or paid). The notes say which, so the tooltips never pass a guess off as a benchmark. */
export function metricsFor(models: ModelInfo[], providerId: string) {
  const raws = new Map(models.map(info => [info.id, raw(info, providerId)]))
  const top = Math.max(1, ...[...raws.values()].map(item => item.intelligence))
  const fastest = Math.max(1, ...[...raws.values()].map(item => item.speed))
  const widest = Math.max(1, ...models.map(info => info.contextWindow ?? 0))
  const local = providerId === 'ollama' || providerId === 'local'
  return (info: ModelInfo): { metrics: Metrics; notes: MetricNotes } => {
    const item = raws.get(info.id) ?? raw(info, providerId)
    const window = info.contextWindow ?? 0
    return {
      metrics: {
        intelligence: clamp(item.intelligence / top * 10),
        speed: clamp(item.speed / fastest * 10),
        context: window ? clamp(Math.log2(window) / Math.log2(widest) * 10) : 0,
        cost: local || info.free ? 10 : 4,
      },
      notes: {
        intelligence: `Estimated from the ${familyOf(info.id).label} family, its size and variant, relative to the strongest model in this list`,
        speed: `Estimated from the model's size and variant and how fast ${hostOf(providerId).name} usually serves it`,
        context: window ? `${window.toLocaleString()} tokens context window, as the provider reports it` : 'The provider did not report a context window',
        cost: local ? 'Runs on your computer: no per-token cost' : info.free ? 'Free with this key, within the provider’s daily limits' : 'Paid per token on this key; see the provider’s price list',
      },
    }
  }
}

/** One plain sentence about the model, built from what the provider says it can do. */
export function describe(info: ModelInfo, providerId: string) {
  const family = familyOf(info.id)
  const traits = [info.reasoning && 'reasons step by step', info.kind === 'code' && 'is tuned for code', info.vision && 'reads images', info.tools ? 'calls tools, so it can edit files and run commands' : 'cannot call tools, so it suits Chat more than Code']
    .filter(Boolean) as string[]
  const who = family.maker ? `${family.label} model from ${family.maker}` : 'Model'
  const served = info.free ? `, free on ${hostOf(providerId).name}` : ` on ${hostOf(providerId).name}`
  const list = traits.length > 1 ? `${traits.slice(0, -1).join(', ')} and ${traits[traits.length - 1]}` : traits[0]
  return `${who}${served}. It ${list}.`
}
