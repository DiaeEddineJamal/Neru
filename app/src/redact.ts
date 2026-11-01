/**
 * Finds secrets in text before it leaves the device: private keys, provider API keys, tokens, and the values of
 * KEY/SECRET/TOKEN/PASSWORD settings in .env style lines. The desktop's src-tauri/src/redact.rs uses the same rules.
 * Plain TypeScript with no imports, so the phone, the desktop window and Node checks can all use it.
 */

export type Secret = { kind: string; start: number; end: number }

const RULES: [string, RegExp][] = [
  ['private key', /-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----/g],
  ['Anthropic key', /\bsk-ant-[A-Za-z0-9_-]{20,}/g],
  ['OpenAI key', /\bsk-(?:proj-|svcacct-)?[A-Za-z0-9_-]{20,}/g],
  ['GitHub token', /\b(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,})/g],
  ['AWS key', /\b(?:AKIA|ASIA)[0-9A-Z]{16}\b/g],
  ['Google key', /\bAIza[0-9A-Za-z_-]{35}/g],
  ['Slack token', /\bxox[abprs]-[A-Za-z0-9-]{10,}/g],
  ['Stripe key', /\b(?:sk|rk)_live_[0-9A-Za-z]{20,}/g],
  ['NVIDIA key', /\bnvapi-[A-Za-z0-9_-]{30,}/g],
  ['Hugging Face token', /\bhf_[A-Za-z0-9]{30,}/g],
  ['Groq key', /\bgsk_[A-Za-z0-9]{40,}/g],
  ['OpenRouter key', /\bsk-or-v1-[a-f0-9]{40,}/g],
  ['JSON web token', /\beyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}/g],
]

// NAME=value or NAME: value where NAME says it is secret; the value is what gets hidden.
const ASSIGNMENT = /\b([A-Za-z0-9_.-]*(?:secret|token|passw(?:or)?d|pwd|api[_-]?key|apikey|private[_-]?key|access[_-]?key|client[_-]?secret|auth)[A-Za-z0-9_.-]*)(["']?\s*[:=]\s*["']?)([^\s"'`,;#]{8,})/gi
// user:password@host in connection strings.
const URL_PASSWORD = /\b[a-z][a-z0-9+.-]*:\/\/[^:\s/@]+:([^@\s/]{3,})@/gi

/** Values that are clearly examples or code, not secrets. Real secrets have a digit or are long. */
const placeholder = (value: string) =>
  (!/\d/.test(value) && value.length < 16) ||
  /[()[\]{}]/.test(value) ||
  /^(?:your|my|example|sample|dummy|test|xxx|changeme|placeholder|redacted|\*+|<|\$\{|\{\{|process\.env|os\.environ|env\(|true|false|null|none|undefined)/i.test(value) ||
  /^[x*.]+$/i.test(value) ||
  /\[secret removed|«secret/i.test(value)

/** Every secret in `text`, in order, without overlaps. */
export function findSecrets(text: string): Secret[] {
  const found: Secret[] = []
  for (const [kind, rule] of RULES) for (const m of text.matchAll(rule)) found.push({ kind, start: m.index, end: m.index + m[0].length })
  for (const m of text.matchAll(ASSIGNMENT)) {
    if (placeholder(m[3])) continue
    const start = m.index + m[1].length + m[2].length
    found.push({ kind: m[1], start, end: start + m[3].length })
  }
  for (const m of text.matchAll(URL_PASSWORD)) {
    const start = m.index + m[0].length - 1 - m[1].length
    if (!placeholder(m[1])) found.push({ kind: 'password', start, end: start + m[1].length })
  }
  found.sort((a, b) => a.start - b.start || b.end - a.end)
  const out: Secret[] = []
  for (const s of found) if (!out.length || s.start >= out[out.length - 1].end) out.push(s)
  return out
}

/** `text` with each secret replaced by `mark(secret, value)`; also returns what was found. */
export function redact(text: string, mark: (s: Secret, value: string) => string = s => `[secret removed: ${s.kind}]`) {
  const secrets = findSecrets(text)
  if (!secrets.length) return { text, secrets }
  let out = ''
  let at = 0
  for (const s of secrets) {
    out += text.slice(at, s.start) + mark(s, text.slice(s.start, s.end))
    at = s.end
  }
  return { text: out + text.slice(at), secrets }
}
