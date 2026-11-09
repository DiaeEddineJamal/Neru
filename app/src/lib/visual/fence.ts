/** Fence tags drawn as visuals rather than shown as code. */
export const CHART_TAGS = ['chart'] as const
export const WIDGET_TAGS = ['html-widget', 'visual', 'widget'] as const
export const DIAGRAM_TAGS = ['mermaid'] as const

export type VisualKind = 'chart' | 'widget' | 'mermaid'

export function visualKind(tag: string | undefined): VisualKind | null {
  if (!tag) return null
  const lower = tag.toLowerCase()
  if ((CHART_TAGS as readonly string[]).includes(lower)) return 'chart'
  if ((WIDGET_TAGS as readonly string[]).includes(lower)) return 'widget'
  if ((DIAGRAM_TAGS as readonly string[]).includes(lower)) return 'mermaid'
  return null
}

export const isVisualTag = (tag: string | undefined) => visualKind(tag) !== null

type Node = { type: string; lang?: string | null; position?: { start: { offset?: number }; end: { offset?: number } }; data?: { hProperties?: Record<string, unknown> }; children?: Node[] }

/** True when a fenced block's source ends with its closing fence. */
export function fenceIsClosed(block: string): boolean {
  const lines = block.replace(/\r\n?/g, '\n').replace(/\n+$/, '').split('\n')
  if (lines.length < 2) return false
  // Blockquote and list prefixes are dropped before matching.
  const strip = (line: string) => line.replace(/^[\s>]*/, '')
  const open = /^(`{3,}|~{3,})/.exec(strip(lines[0]))
  if (!open) return true // not a fence (indented code): always complete
  const close = /^(`{3,}|~{3,})\s*$/.exec(strip(lines[lines.length - 1]))
  return !!close && close[1][0] === open[1][0] && close[1].length >= open[1].length
}

/**
 * A remark plugin that marks fenced code still being written (no closing fence yet) with
 * `data-open="true"` on its `<code>` element, so a streaming chart shows a placeholder instead of
 * a half-parsed render.
 */
export function remarkVisualFences() {
  return (tree: Node, file: { value?: unknown; toString(): string }) => {
    const source = typeof file.value === 'string' ? file.value : String(file)
    const walk = (node: Node) => {
      if (node.type === 'code' && node.lang && isVisualTag(node.lang)) {
        const start = node.position?.start.offset, end = node.position?.end.offset
        if (start !== undefined && end !== undefined && !fenceIsClosed(source.slice(start, end))) {
          node.data = { ...node.data, hProperties: { ...node.data?.hProperties, dataOpen: 'true' } }
        }
      }
      node.children?.forEach(walk)
    }
    walk(tree)
  }
}
