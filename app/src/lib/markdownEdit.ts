// Pure Markdown text transforms for the artifact editor: text + selection in, text + selection out.

export type EditState = { text: string; start: number; end: number }
export type ListKind = 'ul' | 'ol' | 'task' | 'quote'

const FRONT = /^---\r?\n[\s\S]*?\r?\n---[ \t]*(?:\r?\n|$)/
const LIST = /^(\s*)(?:([-*+])|(\d+)([.)]))[ \t]+(\[[ xX]\][ \t]+)?/
const PREFIX = /^(\s*)(?:(?:[-*+]|\d+[.)])[ \t]+(?:\[[ xX]\][ \t]+)?|>[ \t]?)/

/** Splits a leading `---` front matter block off so the editor never touches it. */
export function splitFrontMatter(text: string): [front: string, body: string] {
  const match = text.match(FRONT)
  return match ? [match[0], text.slice(match[0].length)] : ['', text]
}

function runOf(text: string, char: string, from: number, step: 1 | -1) {
  let count = 0
  for (let index = from; index >= 0 && index < text.length && text[index] === char; index += step) count++
  return count
}

/** Wraps the selection in `marker` (e.g. `**`), or unwraps it when it is already wrapped. */
export function toggleWrap(state: EditState, marker: string, placeholder = 'text'): EditState {
  const { text, start, end } = state
  const selected = text.slice(start, end)
  const m = marker.length
  const single = marker.split('').every(char => char === marker[0])
  // Bold `**` and italic `*` share a character: only treat a run as ours when its length fits.
  const fits = (run: number) => !single ? true : m === 2 ? run >= 2 : run === 1 || run >= 3
  if (selected.length >= 2 * m && selected.startsWith(marker) && selected.endsWith(marker)
    && (!single || fits(runOf(selected, marker[0], 0, 1)))) {
    return { text: text.slice(0, start) + selected.slice(m, -m) + text.slice(end), start, end: end - 2 * m }
  }
  const before = text.slice(start - m, start), after = text.slice(end, end + m)
  if (start >= m && before === marker && after === marker
    && (!single || fits(Math.min(runOf(text, marker[0], start - 1, -1), runOf(text, marker[0], end, 1))))) {
    return { text: text.slice(0, start - m) + selected + text.slice(end + m), start: start - m, end: end - m }
  }
  const inner = selected || placeholder
  return { text: text.slice(0, start) + marker + inner + marker + text.slice(end), start: start + m, end: start + m + inner.length }
}

function lineRange(text: string, start: number, end: number) {
  const from = text.lastIndexOf('\n', start - 1) + 1
  // A selection ending right after a newline does not include the next line.
  const stop = end > start && text[end - 1] === '\n' ? end - 1 : end
  const next = text.indexOf('\n', stop)
  return { from, to: next === -1 ? text.length : next }
}

function hasKind(line: string, kind: ListKind) {
  if (kind === 'quote') return /^\s*>/.test(line)
  const match = line.match(LIST)
  if (!match) return false
  if (kind === 'task') return Boolean(match[5])
  return !match[5] && (kind === 'ol' ? Boolean(match[3]) : Boolean(match[2]))
}

/** Toggles a list/quote prefix on every selected line; replaces a different list prefix. */
export function toggleLinePrefix(state: EditState, kind: ListKind): EditState {
  const { text } = state
  const { from, to } = lineRange(text, state.start, state.end)
  const lines = text.slice(from, to).split('\n')
  const content = lines.filter(line => line.trim())
  const remove = content.length > 0 && content.every(line => hasKind(line, kind))
  let number = 0
  const next = lines.map(line => {
    if (!line.trim() && lines.length > 1) return line
    const match = line.match(PREFIX)
    const indent = match ? match[1] : line.match(/^\s*/)![0]
    const rest = line.slice(match ? match[0].length : indent.length)
    if (remove) return indent + rest
    const prefix = kind === 'ul' ? '- ' : kind === 'task' ? '- [ ] ' : kind === 'quote' ? '> ' : `${++number}. `
    return indent + prefix + rest
  }).join('\n')
  if (state.start === state.end && lines.length === 1) {
    const caret = Math.max(from, state.start + next.length - (to - from))
    return { text: text.slice(0, from) + next + text.slice(to), start: caret, end: caret }
  }
  return { text: text.slice(0, from) + next + text.slice(to), start: from, end: from + next.length }
}

/** Cycles the current line through H1, H2, H3 and plain text. */
export function cycleHeading(state: EditState): EditState {
  const { text } = state
  const { from, to } = lineRange(text, state.start, state.start)
  const line = text.slice(from, to)
  const match = line.match(/^(#{1,6})[ \t]+/)
  const level = match ? match[1].length : 0
  const rest = match ? line.slice(match[0].length) : line
  const next = (level >= 3 ? '' : `${'#'.repeat(level + 1)} `) + rest
  const shift = next.length - line.length
  const clamp = (at: number) => Math.min(Math.max(at + shift, from), from + next.length)
  return { text: text.slice(0, from) + next + text.slice(to), start: clamp(state.start), end: clamp(state.end) }
}

/** Enter inside a list item: continues the list, or ends it on an empty item. Null means "not in a list". */
export function continueList(state: EditState): EditState | null {
  const { text, start, end } = state
  if (start !== end) return null
  const from = text.lastIndexOf('\n', start - 1) + 1
  const before = text.slice(from, start)
  const quote = before.match(/^(\s*>[ \t]?)/)
  const match = before.match(LIST)
  if (!match && !quote) return null
  const prefix = match ? match[0] : quote![0]
  const lineEnd = text.indexOf('\n', start) === -1 ? text.length : text.indexOf('\n', start)
  if (!text.slice(from + prefix.length, lineEnd).trim()) {
    // Empty item: drop the marker and leave the list.
    return { text: text.slice(0, from) + text.slice(lineEnd), start: from, end: from }
  }
  const marker = !match ? prefix
    : match[1] + (match[3] ? `${Number(match[3]) + 1}${match[4]}` : match[2]) + ' ' + (match[5] ? '[ ] ' : '')
  const insert = '\n' + marker
  return { text: text.slice(0, start) + insert + text.slice(end), start: start + insert.length, end: start + insert.length }
}

/** Tab / Shift+Tab on list lines. Null means "no list lines here", so the key keeps its default. */
export function indentList(state: EditState, outdent: boolean): EditState | null {
  const { text } = state
  const { from, to } = lineRange(text, state.start, state.end)
  const lines = text.slice(from, to).split('\n')
  if (!lines.some(line => LIST.test(line))) return null
  let first = 0, total = 0
  const next = lines.map((line, index) => {
    if (!LIST.test(line)) return line
    const change = outdent ? -Math.min(2, line.match(/^ */)![0].length) : 2
    if (index === 0) first = change
    total += change
    return change >= 0 ? ' '.repeat(change) + line : line.slice(-change)
  }).join('\n')
  const start = Math.max(from, state.start + first)
  return { text: text.slice(0, from) + next + text.slice(to), start, end: Math.max(start, state.end + total) }
}

/** Wraps the selection as `[text](url)` with "url" selected; unwraps an already-linked selection. */
export function toggleLink(state: EditState): EditState {
  const { text, start, end } = state
  const selected = text.slice(start, end)
  const linked = selected.match(/^\[([^\]]*)\]\([^)]*\)$/)
  if (linked) return { text: text.slice(0, start) + linked[1] + text.slice(end), start, end: start + linked[1].length }
  const label = selected || 'link'
  const head = `[${label}](`
  return { text: text.slice(0, start) + head + 'url)' + text.slice(end), start: start + head.length, end: start + head.length + 3 }
}

/** Inserts a block on its own lines, with blank lines around it. `select` is a [from, to] range inside the block. */
export function insertBlock(state: EditState, block: string, select?: [number, number]): EditState {
  const { text, start, end } = state
  const head = text.slice(0, start), tail = text.slice(end)
  const lead = !head ? '' : head.endsWith('\n\n') ? '' : head.endsWith('\n') ? '\n' : '\n\n'
  const trail = tail.startsWith('\n\n') ? '' : tail.startsWith('\n') ? '\n' : '\n\n'
  const at = start + lead.length
  const [a, b] = select ?? [block.length, block.length]
  return { text: head + lead + block + trail + tail, start: at + a, end: at + b }
}

/** Fences the selection as a code block, or unfences a selected fenced block. */
export function toggleCodeBlock(state: EditState): EditState {
  const { text, start, end } = state
  const selected = text.slice(start, end)
  const fenced = selected.match(/^```[^\n]*\n([\s\S]*?)\n?```$/)
  if (fenced) return { text: text.slice(0, start) + fenced[1] + text.slice(end), start, end: start + fenced[1].length }
  const body = selected || 'code'
  return insertBlock(state, '```\n' + body + '\n```', [4, 4 + body.length])
}

export const TABLE = '| Column | Column | Column |\n| --- | --- | --- |\n|  |  |  |\n|  |  |  |'

export function insertTable(state: EditState): EditState {
  return insertBlock(state, TABLE, [2, 8])
}

export function insertRule(state: EditState): EditState {
  return insertBlock(state, '---')
}
