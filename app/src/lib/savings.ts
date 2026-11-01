import type { UsageTotals } from '../types'

/** The paid model savings compare against, in US dollars per million tokens (as the phone app's usage.ts). */
export const REFERENCE = { name: 'Claude Sonnet', input: 3, output: 15 }

/** What `tokens` would cost on the reference model. */
export const referenceCost = (b: UsageTotals['free']) => (b.input * REFERENCE.input + b.output * REFERENCE.output) / 1e6

export const dollars = (n: number) => `$${n < 10 ? n.toFixed(2) : Math.round(n).toLocaleString('en-US')}`
export const tokens = (n: number) => (n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${Math.round(n / 1e3)}K` : String(n))
