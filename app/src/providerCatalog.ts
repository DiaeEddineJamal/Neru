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
  /** What the provider's published terms say it does with prompts. See `privacyLabels`. */
  privacy: Privacy
  /** Where Settings lists it: free plans, models on this computer, or paid keys. */
  group: ProviderGroup
  /** One plain sentence for the provider card. */
  tagline: string
  /** How to get a key, in plain steps, shown next to the key field. */
  keySteps?: string[]
  /** Works without a key (at lower limits). */
  keyOptional?: boolean
  /** Shown first in its group, with a "Recommended" mark. */
  recommended?: boolean
}

export type ProviderGroup = 'free' | 'local' | 'paid'
export const providerGroups: { id: ProviderGroup; title: string; hint: string }[] = [
  { id: 'free', title: 'Free', hint: 'Sign up, copy a key, paste it here. No card needed.' },
  { id: 'local', title: 'On this computer', hint: 'Private and unlimited, but needs a capable computer.' },
  { id: 'paid', title: 'Paid or your own key', hint: 'Billed by the provider for what you use.' },
]

/**
 * What happens to what you send, per the provider's published terms (free tiers included):
 * `local` never leaves your devices; `no-training` is kept a while for abuse checks but not trained on;
 * `may-train` can be used to improve their models; `unknown` is not stated clearly. Terms change, so this is a guide.
 */
export type Privacy = 'local' | 'no-training' | 'may-train' | 'unknown'
export const privacyLabels: Record<Privacy, { label: string; detail: string }> = {
  local: { label: 'Stays on your devices', detail: 'Runs on your own hardware. Nothing is sent to a company.' },
  'no-training': { label: 'Not used for training', detail: 'Their terms say prompts are not used to train models. They may keep logs for a while to check for abuse.' },
  'may-train': { label: 'May train on your data', detail: 'Their terms allow using what you send to improve their models, and people may review it. Avoid private code here.' },
  unknown: { label: 'Not clearly stated', detail: 'Their terms do not say clearly what happens to prompts. Treat what you send as possibly kept.' },
}

/** How much a privacy rule lets through: 'any' everything, 'no-train' blocks `may-train`, 'local' only your own hardware. */
export type PrivacyRule = 'any' | 'no-train' | 'local'
export function privacyAllows(rule: PrivacyRule, privacy: Privacy): boolean {
  if (rule === 'local') return privacy === 'local'
  if (rule === 'no-train') return privacy !== 'may-train'
  return true
}
/** A provider's privacy; on-device models and unknown ids count as local and unknown. */
export const privacyOf = (providerId: string): Privacy => (providerId === 'on-device' ? 'local' : providerPresets.find(p => p.id === providerId)?.privacy ?? 'unknown')

// The `model` of each preset is only the first pick before the key's own list loads: Neru replaces it
// with a model from that list, so an id that a provider retires never sticks.
// Free coding providers first, most generous daily allowance first. Limits change; each provider's
// console shows the live numbers, and Neru's context meter shows what the key reports.
// A provider stays in 'free' only while an official page says a new account can use some models at no
// cost and without a card; the page and the date it was checked are noted above each one. The same list
// is `FREE_PROVIDERS` in src-tauri/src/fallback.rs.
export const providerPresets: ProviderPreset[] = [
  // Free, checked 2026-11-01: https://docs.api.nvidia.com/nim/docs/product ("free access to NIM API endpoints for prototyping").
  { id: 'nvidia', name: 'NVIDIA NIM', group: 'free', recommended: true, tagline: 'Big open coding models with a generous free allowance.', description: 'Large open coding models (Qwen Coder, DeepSeek, GLM, Nemotron). Free for prototyping with an NVIDIA Developer Program account; no card needed. NVIDIA lists more models than a free key can call; Neru tests each one and only offers those that answer.', baseUrl: 'https://integrate.api.nvidia.com/v1', model: '', format: 'openai-chat', keyUrl: 'https://build.nvidia.com/settings/api-keys', freeLimit: 'Free for prototyping, rate limited (about 40/min)', privacy: 'unknown',
    keySteps: ['Sign in at build.nvidia.com (free account).', 'Open API Keys and choose Generate API Key.', 'Copy the key that starts with nvapi- and paste it below.'] },
  // Free, checked 2026-11-01: https://ai.google.dev/gemini-api/docs/pricing (Flash models "Free of charge" on the free tier).
  { id: 'gemini', name: 'Google Gemini', group: 'free', recommended: true, tagline: 'Google’s Flash models, able to read very long projects.', description: 'Free AI Studio key. Flash models read up to 1M tokens, enough for big projects. Not every Gemini model has free quota; Neru tests each one. On the free plan Google may use prompts to improve its products, and in the EU, UK and Switzerland Google’s terms require the paid plan.', baseUrl: 'https://generativelanguage.googleapis.com/v1beta/openai', model: 'gemini-3.8-flash', format: 'openai-chat', keyUrl: 'https://aistudio.google.com/apikey', freeLimit: 'Daily quota per model, per Google Cloud project (keys in one project share it)', privacy: 'may-train',
    keySteps: ['Sign in at aistudio.google.com with a Google account.', 'Choose Create API key.', 'Copy the key and paste it below.'] },
  // Free, checked 2026-11-01: https://modelscope.cn/docs/model-service/API-Inference/limits (free daily calls after binding a real-name-verified Alibaba Cloud account; the page renders in the browser only, so this was read from its search summary).
  { id: 'modelscope', name: 'ModelScope', group: 'free', tagline: 'Alibaba’s free API with Qwen, DeepSeek and GLM.', description: 'Alibaba’s free inference API. Qwen Coder, DeepSeek, GLM and MiniMax models. ModelScope asks you to bind an Alibaba Cloud account with real-name verification before the API works.', baseUrl: 'https://api-inference.modelscope.cn/v1', model: '', format: 'openai-chat', keyUrl: 'https://modelscope.cn/my/myaccesstoken', freeLimit: '2,000 requests/day · per-model caps vary', privacy: 'unknown',
    keySteps: ['Create a ModelScope account (phone number or Alibaba account).', 'Open Access Tokens and create one.', 'Copy the token and paste it below.'] },
  // Free, checked 2026-11-01: https://openrouter.ai/docs/api-reference/limits (50 free-model requests/day under $10 of credit, 1,000 above).
  { id: 'openrouter', name: 'OpenRouter', group: 'free', tagline: 'One key for many free models; Neru switches when one is busy.', description: 'One key for many models. Neru stays on the free catalog and switches free models when one is busy.', baseUrl: 'https://openrouter.ai/api/v1', model: 'openrouter/free', format: 'openai-chat', keyUrl: 'https://openrouter.ai/settings/keys', freeLimit: '50 free requests/day (1,000 after $10 credit)', privacy: 'may-train',
    keySteps: ['Sign in at openrouter.ai.', 'Open Keys and choose Create Key.', 'Copy the key that starts with sk-or- and paste it below.'] },
  // Free, checked 2026-11-01: https://docs.mistral.ai/getting-started/quickstarts/studio/activate-and-generate-api-key ("no credit card required").
  { id: 'mistral', name: 'Mistral', group: 'free', tagline: 'Codestral and Devstral, made for writing code.', description: 'Free plan, no card needed. Codestral and Devstral for writing and editing code. Mistral may use free-plan prompts to train its models.', baseUrl: 'https://api.mistral.ai/v1', model: 'codestral-latest', format: 'openai-chat', keyUrl: 'https://console.mistral.ai/api-keys', freeLimit: 'Free plan, rate limited', privacy: 'may-train',
    keySteps: ['Sign in at console.mistral.ai and choose the free plan.', 'Open API Keys and create a key.', 'Copy the key and paste it below.'] },
  // Free, checked 2026-11-01: https://developers.cloudflare.com/workers-ai/platform/pricing/ (10,000 neurons/day at no charge).
  { id: 'cloudflare', name: 'Cloudflare Workers AI', group: 'free', tagline: 'A free daily allowance on a Cloudflare account.', description: 'A free daily allowance on your Cloudflare account, with open models such as gpt-oss and Llama. Replace ACCOUNT_ID in the address (under Advanced) with your account ID from the Workers AI page, and use an API token with Workers AI access.', baseUrl: 'https://api.cloudflare.com/client/v4/accounts/ACCOUNT_ID/ai/v1', model: '@cf/openai/gpt-oss-120b', format: 'openai-chat', keyUrl: 'https://dash.cloudflare.com/profile/api-tokens', freeLimit: '10,000 neurons/day', privacy: 'no-training',
    keySteps: ['Sign in at dash.cloudflare.com and open Workers AI to find your account ID.', 'Create an API token with the Workers AI template.', 'Paste the token below, then put your account ID in the address under Advanced.'] },
  // Free, checked 2026-11-01: https://console.groq.com/docs/billing-faqs (a card is needed only to leave the Free tier).
  { id: 'groq', name: 'Groq Cloud', group: 'free', tagline: 'Very fast answers; best for Chat.', description: 'Very fast, but the free plan allows about 8K tokens a minute: fine for Chat, too small for Code on real projects. This is Groq, not xAI Grok.', baseUrl: 'https://api.groq.com/openai/v1', model: 'openai/gpt-oss-120b', format: 'openai-chat', keyUrl: 'https://console.groq.com/keys', freeLimit: '1,000 requests/day · 8K tokens/min', privacy: 'no-training',
    keySteps: ['Sign in at console.groq.com.', 'Open API Keys and choose Create API Key.', 'Copy the key that starts with gsk_ and paste it below.'] },
  // Free, checked 2026-11-01: https://docs.ovhcloud.com/en/guides/public-cloud/ai-machine-learning/ai-endpoints-capabilities (anonymous use at 2 requests/min per IP and model).
  { id: 'ovhcloud', name: 'OVHcloud AI Endpoints', group: 'free', keyOptional: true, tagline: 'Hosted in Europe, does not train on your data. Works without a key.', description: 'Open models hosted in Europe by OVHcloud, which does not use your prompts for training. Works without a key at 2 requests a minute per model; a key from the OVHcloud Manager raises that to 400.', baseUrl: 'https://oai.endpoints.kepler.ai.cloud.ovh.net/v1', model: '', format: 'openai-chat', keyUrl: 'https://www.ovh.com/manager/#/public-cloud/', freeLimit: 'Free without a key, 2 requests/min per model', privacy: 'no-training',
    keySteps: ['Optional: you can use it without a key.', 'For higher limits, open the OVHcloud Manager → Public Cloud → AI Endpoints → API keys.', 'Create a key and paste it below.'] },
  // Free, checked 2026-11-01: https://kilo.ai/docs/gateway/authentication (models ending in ":free" need no payment; base URL moved to ai-gateway.kilo.ai).
  { id: 'kilo', name: 'Kilo Gateway', group: 'free', tagline: 'Kilo Code’s gateway, with a rotating set of free models.', description: 'Kilo Code’s OpenAI-compatible gateway. Models ending in :free cost nothing, and the free list changes; Neru tests each one. Free models may log prompts for the model maker.', baseUrl: 'https://ai-gateway.kilo.ai/api/v1', model: '', format: 'openai-chat', keyUrl: 'https://app.kilo.ai', freeLimit: 'Free :free models, about 200 requests/hour', privacy: 'unknown',
    keySteps: ['Sign in at app.kilo.ai.', 'Open your profile and copy the API key.', 'Paste it below.'] },
  // Free, checked 2026-11-01: https://docs.llm7.io/limits ("Free-token quotas are provided at no charge"; a free token is now required).
  { id: 'llm7', name: 'LLM7', group: 'free', tagline: 'Small free gateway. Use only for things that are not private.', description: 'A small free gateway. It now needs a free token, its terms are thin and its limits may drop without notice, so Neru only falls back to it and you should not send private code.', baseUrl: 'https://api.llm7.io/v1', model: '', format: 'openai-chat', keyUrl: 'https://dash.llm7.io', freeLimit: '60 requests/min · 100K tokens/day · fallback only', privacy: 'unknown',
    keySteps: ['Get a free token at dash.llm7.io.', 'Copy the token.', 'Paste it below.'] },
  // Free, checked 2026-11-01: https://docs.sambanova.ai/docs/en/models/rate-limits (free tier while no payment method is linked).
  { id: 'sambanova', name: 'SambaNova', group: 'free', tagline: 'Fast, but a small daily allowance. Used only as a backup.', description: 'Very fast open models on a small free plan (about 20 requests a day per model, no card). Too few to work on, so Neru only falls back to it when the other free keys are busy.', baseUrl: 'https://api.sambanova.ai/v1', model: 'Meta-Llama-3.3-70B-Instruct', format: 'openai-chat', keyUrl: 'https://cloud.sambanova.ai/apis', freeLimit: '20 requests/day per model · fallback only', privacy: 'unknown',
    keySteps: ['Sign in at cloud.sambanova.ai.', 'Open APIs and create a key.', 'Copy the key and paste it below.'] },
  { id: 'ollama', name: 'Ollama', group: 'local', recommended: true, tagline: 'Run open models on this computer. Private, no limits.', description: 'Run open coding models on your computer. Nothing leaves your PC and there are no limits, but it needs a strong machine: qwen3-coder:30b handles Code best on a 24 GB graphics card or a 32 GB Mac; gpt-oss:20b fits 16 GB.', baseUrl: 'http://localhost:11434/v1', model: '', format: 'openai-chat', keyUrl: 'https://ollama.com/download', privacy: 'local',
    keySteps: ['Install Ollama from ollama.com/download.', 'Download a model, for example: ollama pull gpt-oss:20b', 'Leave Ollama running; Neru finds its models.'] },
  { id: 'local', name: 'Local gateway', group: 'local', tagline: 'Another OpenAI-compatible server on this computer.', description: 'FreeLLMAPI or another local OpenAI-compatible server.', baseUrl: 'http://localhost:3001/v1', model: 'auto', format: 'openai-chat', privacy: 'local' },
  { id: 'ollamacloud', name: 'Ollama Cloud', group: 'paid', tagline: 'Big open models run by Ollama, billed for what you use.', description: 'Large open models served by Ollama’s cloud. Usage is billed; check your plan on ollama.com.', baseUrl: 'https://ollama.com/v1', model: '', format: 'openai-chat', keyUrl: 'https://ollama.com/settings/keys', privacy: 'no-training' },
  { id: 'cerebras', name: 'Cerebras', group: 'paid', tagline: 'Very fast open models. Trial only, needs a card.', description: 'Very fast open models. The free plan ended: new accounts get a $5 trial for 30 days and need a card.', baseUrl: 'https://api.cerebras.ai/v1', model: 'gpt-oss-120b', format: 'openai-chat', keyUrl: 'https://cloud.cerebras.ai/platform/', privacy: 'unknown' },
  // Paid, checked 2026-11-01: https://docs.z.ai/guides/overview/pricing lists GLM-4.7-Flash and GLM-4.5-Flash at no
  // charge, but keys without a balance get "Insufficient balance or no resource package" (code 1113), so it is not 'free'.
  { id: 'zai', name: 'Z.AI GLM', group: 'paid', tagline: 'GLM models from Z.AI, billed to your balance.', description: 'Z.AI key, billed by Z.AI. Its price list shows GLM-4.7-Flash and GLM-4.5-Flash at no charge, but a key without a balance can still be refused ("Insufficient balance", code 1113); Neru’s model check says so plainly. On a Flash model Neru only switches to other free Flash models on its own.', baseUrl: 'https://api.z.ai/api/paas/v4', model: 'glm-4.7-flash', format: 'openai-chat', keyUrl: 'https://z.ai/manage-apikey/apikey-list', privacy: 'unknown',
    keySteps: ['Sign in at z.ai and add a balance under Billing.', 'Open API Keys and create a key.', 'Copy the key and paste it below.'] },
  { id: 'opencode', name: 'OpenCode Zen', group: 'paid', tagline: 'Curated coding models with your Zen key.', description: 'Curated coding models with your Zen key.', baseUrl: 'https://opencode.ai/zen/v1', model: '', format: 'openai-chat', keyUrl: 'https://opencode.ai/auth', privacy: 'unknown' },
  { id: 'openai', name: 'OpenAI', group: 'paid', tagline: 'GPT models with your OpenAI key.', description: 'Direct OpenAI API key.', baseUrl: 'https://api.openai.com/v1', model: '', format: 'openai-responses', keyUrl: 'https://platform.openai.com/api-keys', privacy: 'no-training' },
  { id: 'anthropic', name: 'Anthropic', group: 'paid', tagline: 'Claude models with your Anthropic key.', description: 'Direct Claude API key.', baseUrl: 'https://api.anthropic.com/v1', model: '', format: 'anthropic', keyUrl: 'https://console.anthropic.com/settings/keys', privacy: 'no-training' },
  { id: 'xai', name: 'Grok', group: 'paid', tagline: 'xAI’s Grok models.', description: 'xAI Grok key. Neru lists the models that key can use.', baseUrl: 'https://api.x.ai/v1', model: '', format: 'openai-chat', keyUrl: 'https://console.x.ai', privacy: 'unknown' },
  { id: 'deepseek', name: 'DeepSeek', group: 'paid', tagline: 'DeepSeek models at low prices.', description: 'Direct DeepSeek API key.', baseUrl: 'https://api.deepseek.com', model: '', format: 'openai-chat', keyUrl: 'https://platform.deepseek.com/api_keys', privacy: 'may-train' },
  { id: 'qwen', name: 'Alibaba Qwen', group: 'paid', tagline: 'Qwen models from Alibaba Model Studio.', description: 'Model Studio, Singapore region by default.', baseUrl: 'https://dashscope-intl.aliyuncs.com/compatible-mode/v1', model: '', format: 'openai-chat', keyUrl: 'https://modelstudio.console.alibabacloud.com/?tab=playground#/api-key', privacy: 'unknown' },
  { id: 'kimi', name: 'Moonshot Kimi', group: 'paid', tagline: 'Kimi models from Moonshot AI.', description: 'Direct Moonshot AI API key.', baseUrl: 'https://api.moonshot.ai/v1', model: '', format: 'openai-chat', keyUrl: 'https://platform.moonshot.ai/console/api-keys', privacy: 'unknown' },
  { id: 'minimax', name: 'MiniMax', group: 'paid', tagline: 'MiniMax models with your own key.', description: 'Direct MiniMax API key.', baseUrl: 'https://api.minimax.io/v1', model: '', format: 'openai-chat', keyUrl: 'https://platform.minimax.io/user-center/basic-information/interface-key', privacy: 'unknown' },
  { id: 'huggingface', name: 'Hugging Face', group: 'paid', tagline: 'Open models through Hugging Face’s partners. Billed.', description: 'Hugging Face token. Open models served by several inference partners. Free accounts get no usage credit, so Neru treats it as a paid key and does not fall back to it.', baseUrl: 'https://router.huggingface.co/v1', model: 'openai/gpt-oss-120b', format: 'openai-chat', keyUrl: 'https://huggingface.co/settings/tokens', privacy: 'unknown' },
  { id: 'custom', name: 'Custom endpoint', group: 'paid', tagline: 'Any compatible API: enter its address under Advanced.', description: 'Any compatible Chat, Responses, or Messages API.', baseUrl: '', model: '', format: 'openai-chat', privacy: 'unknown' },
]

const chatProviders = new Set(['kilo', 'ovhcloud', 'llm7', 'ollamacloud', 'cloudflare', 'sambanova', 'modelscope', 'local', 'ollama', 'groq', 'gemini', 'cerebras', 'mistral', 'nvidia', 'huggingface', 'openrouter', 'deepseek', 'qwen', 'kimi', 'zai', 'minimax', 'xai'])

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
