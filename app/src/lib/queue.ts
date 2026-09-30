import type { AttachedDocument } from '../types'

/** A message the user chose to send after the running reply finishes. */
export interface QueuedMessage { id: string; text: string; contextPaths: string[]; documents: AttachedDocument[] }

export type QueueMap = Record<string, QueuedMessage[]>

export const queueLabel = (count: number) => `${count} ${count === 1 ? 'message' : 'messages'} queued`

export const enqueue = (queues: QueueMap, sessionId: string, item: QueuedMessage): QueueMap => ({ ...queues, [sessionId]: [...(queues[sessionId] ?? []), item] })

export const dequeue = (queues: QueueMap, sessionId: string, id: string): QueueMap => {
  const rest = (queues[sessionId] ?? []).filter(item => item.id !== id)
  if (rest.length === (queues[sessionId] ?? []).length) return queues
  const next = { ...queues }
  if (rest.length) next[sessionId] = rest
  else delete next[sessionId]
  return next
}

/** Steering only carries text, so a queued message with attachments waits for the reply to finish. */
export const canSteerQueued = (item: QueuedMessage) => Boolean(item.text.trim()) && item.documents.length === 0 && item.contextPaths.length === 0
