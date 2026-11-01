// Reviewing the documents the agent writes (plans, walkthroughs, Spec mode): splitting them into
// blocks to comment on, and the message that sends a review back to the agent.

import type { SessionDocument } from '../types'

/** Spec mode's documents, in the order the agent writes them. */
export const SPEC_STEPS = ['requirements', 'design', 'tasks'] as const

/** Markdown split into blocks (paragraphs, lists, headings, fenced code) that comments attach to. */
export function markdownBlocks(markdown: string): string[] {
  const blocks: string[] = []
  let current: string[] = []
  let fence: string | null = null
  const flush = () => { if (current.some(line => line.trim())) blocks.push(current.join('\n')); current = [] }
  for (const line of markdown.replace(/\r\n/g, '\n').split('\n')) {
    const marker = /^\s*(```|~~~)/.exec(line)?.[1]
    if (fence) { current.push(line); if (marker === fence) { fence = null; flush() } continue }
    if (marker) { flush(); fence = marker; current.push(line); continue }
    if (!line.trim()) { flush(); continue }
    // A heading stands alone, so it can take its own comment.
    if (/^#{1,6}\s/.test(line)) { flush(); current.push(line); flush(); continue }
    current.push(line)
  }
  flush()
  return blocks
}

export interface BlockComment { id: string; block: number; text: string }

/** The message that sends the user's review of a document to the agent. */
export function reviewMessage(doc: Pick<SessionDocument, 'name' | 'title'>, blocks: string[], comments: BlockComment[], general: string, edited: string | null) {
  const parts = [`Review of the document "${doc.title}" (${doc.name}).`]
  if (edited !== null) parts.push(`I edited it. Its current text:\n\n\`\`\`markdown\n${edited}\n\`\`\``)
  for (const comment of [...comments].sort((a, b) => a.block - b.block)) {
    const quote = (blocks[comment.block] ?? '').split('\n').find(line => line.trim())?.trim().slice(0, 160) ?? ''
    parts.push(`> ${quote}\n\n${comment.text.trim()}`)
  }
  if (general.trim()) parts.push(general.trim())
  parts.push('Revise the document with write_document to address these comments, then stop for my review.')
  return parts.join('\n\n')
}
