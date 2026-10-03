/**
 * Turns raw errors (provider responses, OS errors, Rust messages) into plain words: what went
 * wrong, what to do about it, and a one-click action when there is an obvious one.
 */

export type ErrorAction = 'model-settings' | 'retry' | 'open-project' | 'doctor' | 'compact' | 'connectors'

export interface FriendlyError {
  title: string
  hint?: string
  action?: ErrorAction
  /** The original message, for "Details". Empty when it adds nothing to the title. */
  detail: string
}

const has = (text: string, ...parts: string[]) => parts.some(part => text.includes(part))

export function describeError(value: unknown): FriendlyError {
  const raw = (value instanceof Error ? value.message : typeof value === 'string' ? value : JSON.stringify(value) ?? String(value)).trim()
  const text = raw.toLowerCase()
  const withDetail = (error: Omit<FriendlyError, 'detail'>): FriendlyError => ({ ...error, detail: raw === error.title ? '' : raw })

  // OpenCode Zen: "Free tier can only be used from within OpenCode" (HTTP 403). The key is fine.
  const locked = /http 403|forbidden/.test(text) && /only (be used|available|works) (from within|within|in|with|through) ([^.,;"\n]{1,40})/.exec(raw)
  if (locked)
    return withDetail({ title: `This free model only works inside ${locked[3].trim()}.`, hint: 'The provider keeps its free tier to its own app. Pick a paid model this key has credits for, or add a key for another provider so Neru can switch on its own.', action: 'model-settings' })
  if (has(text, 'http 402', 'payment required', 'insufficient credit', 'insufficient balance', 'insufficient funds', 'insufficient_quota', 'credit balance is too low'))
    return withDetail({ title: 'This account has no credits for the model.', hint: 'Add credits with the provider, pick a free model, or add a key for another provider so Neru can switch on its own.', action: 'model-settings' })
  if (has(text, 'http 403') && !has(text, 'api key', 'api_key', 'token', 'authentication', 'unauthenticated', 'expired'))
    return withDetail({ title: 'This key may not use that model.', hint: 'Pick another model in Settings → Model provider, or check the plan the key belongs to.', action: 'model-settings' })
  if (has(text, 'http 401', 'http 403', 'unauthorized', 'invalid api key', 'incorrect api key', 'invalid_api_key', 'authentication', 'no auth credentials', 'forbidden'))
    return withDetail({ title: 'The provider rejected your API key.', hint: 'Check or replace the key in Settings → Model provider. Keys sometimes expire or lose access to a model.', action: 'model-settings' })
  if (has(text, 'configure an api key', 'configure a provider', 'no model is set up', 'configure a provider in settings'))
    return withDetail({ title: 'No model is set up yet.', hint: 'Add an API key and pick a model. NVIDIA, Gemini, Cerebras, Mistral and OpenRouter all have free plans.', action: 'model-settings' })
  if (has(text, 'no other model is available', 'no other free coding model'))
    return withDetail({ title: 'Every available model is busy or down right now.', hint: 'Try again in a minute, or add a key for another provider so Neru has more models to switch to.', action: 'model-settings' })
  if (has(text, 'http 429', 'rate limit', 'rate-limit', 'too many requests', 'quota', 'resource_exhausted', 'free-models-per-day'))
    return withDetail({ title: 'The model’s usage limit was reached.', hint: 'Wait a minute and retry, or pick another model. Adding keys for more providers lets Neru switch on its own.', action: 'retry' })
  if (has(text, 'context length', 'context_length', 'context window', 'maximum context', 'too many tokens', 'request is still larger', 'http 413'))
    return withDetail({ title: 'This conversation is too long for the model.', hint: 'Compact it (/compact), start a new session, attach fewer files, or choose a model with a larger context window.', action: 'compact' })
  if (has(text, 'did not start answering', 'did not respond', 'stalled', 'timed out', 'timeout', 'deadline'))
    return withDetail({ title: 'The model took too long to answer.', hint: 'The provider may be overloaded. Retry, or choose a faster model.', action: 'retry' })
  if (has(text, 'error sending request', 'dns', 'connection refused', 'connection reset', 'failed to fetch', 'network', 'could not reach', 'no such host', 'tcp connect', 'offline'))
    return withDetail({ title: 'Neru could not reach the provider.', hint: 'Check your internet connection (and VPN or proxy), then try again.', action: 'retry' })
  if (has(text, 'http 500', 'http 502', 'http 503', 'http 504', 'overloaded', 'unavailable', 'bad gateway'))
    return withDetail({ title: 'The model’s service is having problems.', hint: 'This is on the provider’s side. Retry shortly or choose another model.', action: 'retry' })
  if (has(text, 'model not found', 'model_not_found', 'does not exist', 'unknown model', 'http 404'))
    return withDetail({ title: 'That model is not available with this key.', hint: 'Pick another model in Settings → Model provider.', action: 'model-settings' })
  if (has(text, 'open a project', 'no project open', 'no project is open'))
    return withDetail({ title: 'Open a project folder first.', hint: 'Code works on a folder on your computer. Chat works without one.', action: 'open-project' })
  if (has(text, 'os error 112', 'not enough space', 'disk full', 'no space left'))
    return withDetail({ title: 'Your disk is full.', hint: 'Free some space on the drive and try again.' })
  if (has(text, 'os error 5', 'access is denied', 'permission denied', 'eacces', 'eperm'))
    return withDetail({ title: 'Neru does not have permission to do that.', hint: 'The file may be open in another program, read-only, or in a protected folder.' })
  if (has(text, 'os error 2', 'os error 3', 'cannot find the path', 'no such file', 'not found on path', 'enoent'))
    return withDetail({ title: 'A file or program could not be found.', hint: 'It may have been moved or deleted, or a tool (Git, Node.js) is not installed. /doctor checks your setup.', action: 'doctor' })
  if (has(text, 'mcp', 'connector'))
    return withDetail({ title: 'A connector ran into a problem.', hint: 'Open Connectors to restart it or sign in again.', action: 'connectors' })
  if (has(text, 'review the pending action', 'already responding'))
    return withDetail({ title: raw.split('\n')[0], hint: 'Finish or dismiss the current step first.' })
  // Unknown: keep the message, but only its first line, and point at /doctor.
  const first = raw.split('\n')[0].replace(/^Error:\s*/i, '')
  const long = raw.includes('\n') || first.length > 160
  return { title: long ? `${first.slice(0, 157)}${first.length > 157 ? '…' : ''}` : first || 'Something went wrong.', hint: 'If this keeps happening, /doctor checks your setup.', action: 'doctor', detail: long ? raw : '' }
}

export const actionLabels: Record<ErrorAction, string> = {
  'model-settings': 'Open model settings',
  retry: 'Try again',
  'open-project': 'Open a folder',
  doctor: 'Run /doctor',
  compact: 'Compact conversation',
  connectors: 'Open connectors',
}
