import type { ReactNode } from 'react'

/** Just enough of an mdast node to walk the tree. */
interface MdNode { type: string; value?: string; children?: MdNode[]; data?: Record<string, unknown> }

const MENTION = /(^|[^\w.@/])@([a-z][\w-]*)/gi

function pieces(text: string, handles: Set<string>): ({ mention: string } | { text: string })[] {
  const out: ({ mention: string } | { text: string })[] = []
  let last = 0
  for (const match of text.matchAll(MENTION)) {
    const handle = match[2].toLowerCase()
    if (!handles.has(handle)) continue
    const at = match.index + match[1].length
    if (at > last) out.push({ text: text.slice(last, at) })
    out.push({ mention: `@${match[2]}` })
    last = at + 1 + match[2].length
  }
  if (last < text.length) out.push({ text: text.slice(last) })
  return out
}

/** Remark plugin: `@claude`, `@codex`, `@all` in prose become `<span class="mention">`, outside code and links. */
export function remarkMentions(options: { handles: string[] }) {
  const handles = new Set([...options.handles.map(handle => handle.toLowerCase()), 'all'])
  const walk = (node: MdNode) => {
    if (!node.children || node.type === 'link' || node.type === 'linkReference') return
    node.children = node.children.flatMap(child => {
      if (child.type !== 'text' || !child.value?.includes('@')) { walk(child); return [child] }
      return pieces(child.value, handles).map(piece => 'mention' in piece
        ? { type: 'mention', data: { hName: 'span', hProperties: { className: ['mention'] } }, children: [{ type: 'text', value: piece.mention }] }
        : { type: 'text', value: piece.text })
    })
  }
  return (tree: MdNode) => walk(tree)
}

/** The same highlighting for plain text, such as the user's own messages. */
export function withMentions(text: string, handles: string[]): ReactNode[] {
  return pieces(text, new Set([...handles.map(handle => handle.toLowerCase()), 'all'])).map((piece, index) => 'mention' in piece
    ? <span key={index} className="mention">{piece.mention}</span>
    : piece.text)
}
