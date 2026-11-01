// The small Markdown subset research papers use, parsed once for every export (Word, PDF, LaTeX, HTML):
// paragraphs, ### subheadings, bullet and numbered lists, block quotes and pipe tables, with bold,
// italic, code and links inline.

export type Inline = { text: string; bold?: boolean; italic?: boolean; code?: boolean }
export type Block =
  | { kind: 'paragraph'; inlines: Inline[] }
  | { kind: 'heading'; level: 3 | 4; inlines: Inline[] }
  | { kind: 'list'; ordered: boolean; items: Inline[][] }
  | { kind: 'quote'; inlines: Inline[] }
  | { kind: 'table'; header: Inline[][]; rows: Inline[][][] }

/** Splits **bold**, *italic* / _italic_, `code` and [text](url) (kept as text) into runs. */
export function inlines(text: string): Inline[] {
  const out: Inline[] = []
  const pattern = /(\*\*([^*]+)\*\*|__([^_]+)__|\*([^*\s][^*]*)\*|(?<![\w])_([^_\s][^_]*)_(?![\w])|`([^`]+)`|\[([^\]]+)\]\((?:[^)]+)\))/g
  let last = 0
  for (const match of text.matchAll(pattern)) {
    if (match.index > last) out.push({ text: text.slice(last, match.index) })
    if (match[2] ?? match[3]) out.push({ text: match[2] ?? match[3], bold: true })
    else if (match[4] ?? match[5]) out.push({ text: match[4] ?? match[5], italic: true })
    else if (match[6]) out.push({ text: match[6], code: true })
    else if (match[7]) out.push({ text: match[7] })
    last = match.index + match[0].length
  }
  if (last < text.length) out.push({ text: text.slice(last) })
  return out.filter(run => run.text.length > 0)
}

const cells = (line: string) => line.trim().replace(/^\||\|$/g, '').split('|').map(cell => inlines(cell.trim()))
const isRule = (line: string) => /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/.test(line)

export function blocks(markdown: string): Block[] {
  const lines = markdown.replace(/\r\n/g, '\n').split('\n')
  const out: Block[] = []
  let paragraph: string[] = []
  const flush = () => { if (paragraph.length) { out.push({ kind: 'paragraph', inlines: inlines(paragraph.join(' ')) }); paragraph = [] } }
  for (let index = 0; index < lines.length; index++) {
    const line = lines[index]
    const trimmed = line.trim()
    if (!trimmed) { flush(); continue }
    const heading = /^(#{2,6})\s+(.*)$/.exec(trimmed)
    if (heading) { flush(); out.push({ kind: 'heading', level: heading[1].length <= 3 ? 3 : 4, inlines: inlines(heading[2].replace(/\s+#+$/, '')) }); continue }
    if (trimmed.startsWith('|') && index + 1 < lines.length && isRule(lines[index + 1])) {
      flush()
      const header = cells(trimmed)
      const rows: Inline[][][] = []
      index += 2
      while (index < lines.length && lines[index].trim().startsWith('|')) { rows.push(cells(lines[index])); index++ }
      index--
      out.push({ kind: 'table', header, rows })
      continue
    }
    const bullet = /^[-*+]\s+(.*)$/.exec(trimmed)
    const numbered = /^\d+[.)]\s+(.*)$/.exec(trimmed)
    if (bullet || numbered) {
      flush()
      const ordered = Boolean(numbered)
      const items: Inline[][] = []
      while (index < lines.length) {
        const item = (ordered ? /^\s*\d+[.)]\s+(.*)$/ : /^\s*[-*+]\s+(.*)$/).exec(lines[index])
        if (!item) {
          // A wrapped line continues the item before it.
          if (lines[index].trim() && /^\s{2,}/.test(lines[index]) && items.length) { items[items.length - 1].push(...inlines(` ${lines[index].trim()}`)); index++; continue }
          break
        }
        items.push(inlines(item[1]))
        index++
      }
      index--
      out.push({ kind: 'list', ordered, items })
      continue
    }
    if (trimmed.startsWith('>')) { flush(); out.push({ kind: 'quote', inlines: inlines(trimmed.replace(/^>\s?/, '')) }); continue }
    paragraph.push(trimmed)
  }
  flush()
  return out
}

/** Plain text of a run list, for alt text, file names and the like. */
export const plain = (runs: Inline[]) => runs.map(run => run.text).join('')
