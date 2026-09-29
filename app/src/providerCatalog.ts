export type ApiFormat = 'openai-chat' | 'openai-responses' | 'anthropic'

export type ProviderPreset = {
  id: string
  name: string
  description: string
  baseUrl: string
  model: string
  format: ApiFormat
  /** Page where the user creates an API key for this provider. */
  keyUrl?: string
  /** Free-plan allowance, shown next to the provider. */
  freeLimit?: string
}

// Free coding providers first, most generous daily allowance first. Limits change; each provider's
// console shows the live numbers, and Neru's context meter shows what the key reports.
export const providerPresets: ProviderPreset[] = [
  { id: 'nvidia', name: 'NVIDIA NIM', description: 'Recommended free pick. Large open coding models (Qwen Coder, Kimi, DeepSeek, GLM). No card needed.', baseUrl: 'https://integrate.api.nvidia.com/v1', model: '', format: 'openai-chat', keyUrl: 'https://build.nvidia.com/settings/api-keys', freeLimit: '~10,000 requests/day · 40/min' },
  { id: 'modelscope', name: 'ModelScope', description: 'Alibaba’s free inference API. Qwen Coder, DeepSeek, GLM, and MiniMax models. Sign up with a phone number or Alibaba account.', baseUrl: 'https://api-inference.modelscope.cn/v1', model: '', format: 'openai-chat', keyUrl: 'https://modelscope.cn/my/myaccesstoken', freeLimit: '2,000 requests/day · 500 per model' },
  { id: 'gemini', name: 'Google Gemini', description: 'Free AI Studio key. Flash models with a 1M-token context for big projects.', baseUrl: 'https://generativelanguage.googleapis.com/v1beta/openai', model: 'gemini-2.5-flash', format: 'openai-chat', keyUrl: 'https://aistudio.google.com/apikey', freeLimit: 'Daily quota per model, shown in AI Studio' },
  { id: 'cerebras', name: 'Cerebras', description: 'Very fast open models. Few requests per minute, so long agent runs are slower.', baseUrl: 'https://api.cerebras.ai/v1', model: 'gpt-oss-120b', format: 'openai-chat', keyUrl: 'https://cloud.cerebras.ai/platform/', freeLimit: '1M tokens/day · 5 requests/min' },
  { id: 'mistral', name: 'Mistral', description: 'Free plan. Codestral and Devstral for writing and editing code.', baseUrl: 'https://api.mistral.ai/v1', model: 'codestral-latest', format: 'openai-chat', keyUrl: 'https://console.mistral.ai/api-keys', freeLimit: 'Monthly free credit' },
  { id: 'openrouter', name: 'OpenRouter', description: 'One key for many models. Neru stays on the free catalog and switches free models when one is busy.', baseUrl: 'https://openrouter.ai/api/v1', model: 'openrouter/free', format: 'openai-chat', keyUrl: 'https://openrouter.ai/settings/keys', freeLimit: '50 free requests/day (1,000 after $10 credit)' },
  { id: 'groq', name: 'Groq Cloud', description: 'Very fast, but the free plan allows about 8K tokens a minute: fine for Chat, too small for Code on real projects. This is Groq, not xAI Grok.', baseUrl: 'https://api.groq.com/openai/v1', model: 'llama-3.3-70b-versatile', format: 'openai-chat', keyUrl: 'https://console.groq.com/keys', freeLimit: '1,000 requests/day · 8K tokens/min' },
  { id: 'huggingface', name: 'Hugging Face', description: 'Free token. Open coder models, with a small monthly credit.', baseUrl: 'https://router.huggingface.co/v1', model: 'Qwen/Qwen2.5-Coder-32B-Instruct', format: 'openai-chat', keyUrl: 'https://huggingface.co/settings/tokens', freeLimit: 'Small monthly credit' },
  { id: 'local', name: 'Local gateway', description: 'FreeLLMAPI or another local OpenAI-compatible server', baseUrl: 'http://localhost:3001/v1', model: 'auto', format: 'openai-chat' },
  { id: 'ollama', name: 'Ollama / local models', description: 'Run open coding models on your computer. No limits, uses your hardware.', baseUrl: 'http://localhost:11434/v1', model: '', format: 'openai-chat', keyUrl: 'https://ollama.com/search?c=tools' },
  { id: 'opencode', name: 'OpenCode Zen', description: 'Curated coding models with your Zen key', baseUrl: 'https://opencode.ai/zen/v1', model: '', format: 'openai-chat', keyUrl: 'https://opencode.ai/auth' },
  { id: 'xai', name: 'Grok', description: 'xAI Grok key. Neru lists the models that key can use.', baseUrl: 'https://api.x.ai/v1', model: '', format: 'openai-chat', keyUrl: 'https://console.x.ai' },
  { id: 'openai', name: 'OpenAI', description: 'Direct OpenAI API key', baseUrl: 'https://api.openai.com/v1', model: '', format: 'openai-responses', keyUrl: 'https://platform.openai.com/api-keys' },
  { id: 'anthropic', name: 'Anthropic', description: 'Direct Claude API key', baseUrl: 'https://api.anthropic.com/v1', model: '', format: 'anthropic', keyUrl: 'https://console.anthropic.com/settings/keys' },
  { id: 'deepseek', name: 'DeepSeek', description: 'Direct DeepSeek API key', baseUrl: 'https://api.deepseek.com', model: '', format: 'openai-chat', keyUrl: 'https://platform.deepseek.com/api_keys' },
  { id: 'qwen', name: 'Alibaba Qwen', description: 'Model Studio, Singapore region by default', baseUrl: 'https://dashscope-intl.aliyuncs.com/compatible-mode/v1', model: '', format: 'openai-chat', keyUrl: 'https://modelstudio.console.alibabacloud.com/?tab=playground#/api-key' },
  { id: 'kimi', name: 'Moonshot Kimi', description: 'Direct Moonshot AI API key', baseUrl: 'https://api.moonshot.ai/v1', model: '', format: 'openai-chat', keyUrl: 'https://platform.moonshot.ai/console/api-keys' },
  { id: 'zai', name: 'Z.AI GLM', description: 'Direct Z.AI API key', baseUrl: 'https://api.z.ai/api/paas/v4', model: '', format: 'openai-chat', keyUrl: 'https://z.ai/manage-apikey/apikey-list' },
  { id: 'minimax', name: 'MiniMax', description: 'Direct MiniMax API key', baseUrl: 'https://api.minimax.io/v1', model: '', format: 'openai-chat', keyUrl: 'https://platform.minimax.io/user-center/basic-information/interface-key' },
  { id: 'custom', name: 'Custom endpoint', description: 'Any compatible Chat, Responses, or Messages API', baseUrl: '', model: '', format: 'openai-chat' },
]

const chatProviders = new Set(['modelscope', 'local', 'ollama', 'groq', 'gemini', 'cerebras', 'mistral', 'nvidia', 'huggingface', 'openrouter', 'deepseek', 'qwen', 'kimi', 'zai', 'minimax', 'xai'])

function leaf(model: string) {
  return (model.split('/').pop() ?? model).trim().toLowerCase()
}

function hostOf(baseUrl: string) {
  try { return new URL(baseUrl).host.toLowerCase() } catch { return '' }
}

/** OpenAI's current GPT and reasoning models use the Responses API. Older chat models stay on Chat Completions. */
function openAiFormat(model: string): ApiFormat {
  const name = leaf(model)
  if (!name || name === 'auto') return 'openai-responses'
  if (/^(gpt-4o|gpt-4\.1|gpt-5|o1|o3|o4|codex)/.test(name)) return 'openai-responses'
  if (/^(gpt-3\.5|gpt-4$|gpt-4-|text-|davinci|babbage|ada|curie|whisper|tts|dall-e)/.test(name)) return 'openai-chat'
  return 'openai-responses'
}

/** OpenCode Zen serves each model family on a different protocol. */
function openCodeFormat(model: string): ApiFormat {
  const name = leaf(model)
  if (/^(gpt-|grok-|muse-spark)/.test(name)) return 'openai-responses'
  if (/^(claude-|qwen3\.[567]-|qwen3\.8-flash)/.test(name)) return 'anthropic'
  return 'openai-chat'
}

function customFormat(baseUrl: string, model: string, current: ApiFormat): ApiFormat {
  const host = hostOf(baseUrl)
  if (host === 'api.openai.com') return openAiFormat(model)
  if (host === 'api.anthropic.com') return 'anthropic'
  if (host === 'api.x.ai' || host === 'localhost' || host === '127.0.0.1' || host === '[::1]') return 'openai-chat'
  if (host.endsWith('opencode.ai')) return openCodeFormat(model)
  const name = leaf(model)
  if (/^claude-/.test(name)) return 'anthropic'
  if (/^(gpt-4o|gpt-4\.1|gpt-5|o1|o3|o4)/.test(name)) return 'openai-responses'
  if (name) return 'openai-chat'
  return current
}

/** Protocol for this key's selected model: Chat Completions, Responses, or Anthropic Messages. */
export function formatForModel(providerId: string, model: string, current: ApiFormat = 'openai-chat', baseUrl = ''): ApiFormat {
  if (providerId === 'anthropic') return 'anthropic'
  if (providerId === 'openai') return openAiFormat(model)
  if (providerId === 'opencode') return openCodeFormat(model)
  if (providerId === 'custom') return customFormat(baseUrl, model, current)
  if (chatProviders.has(providerId)) return 'openai-chat'
  return providerPresets.find(item => item.id === providerId)?.format ?? customFormat(baseUrl, model, current)
}

export function formatLabel(format: ApiFormat): string {
  switch (format) {
    case 'openai-chat': return 'Chat Completions'
    case 'openai-responses': return 'Responses'
    case 'anthropic': return 'Anthropic Messages'
    default: {
      const neverFormat: never = format
      return neverFormat
    }
  }
}
