import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type { EmbedRequest, EmbedResponse } from './embed.worker'
import { EMBEDDING_MODEL } from './model'

/**
 * The window's half of semantic search (semantic.rs): it embeds the agent's queries and the
 * project's code chunks with a local model in a worker, and hands the vectors back to Rust.
 * Indexing starts in a project the first time the agent searches it, then follows its edits.
 */

interface ChunkJob { path: string; mtime: number; chunk: number; start: number; end: number; text: string }
interface Batch { files: number; embedded: number; chunks: ChunkJob[] }

let worker: Worker | null = null
let nextId = 1
const waiting = new Map<number, { resolve: (vectors: number[][]) => void; reject: (error: Error) => void }>()

function getWorker() {
  if (!worker) {
    worker = new Worker(new URL('./embed.worker.ts', import.meta.url), { type: 'module' })
    worker.onmessage = (event: MessageEvent<EmbedResponse>) => {
      const message = event.data
      const entry = waiting.get(message.id)
      if (!entry) return
      waiting.delete(message.id)
      if (message.type === 'result') entry.resolve(message.vectors)
      else entry.reject(new Error(message.message))
    }
    worker.onerror = event => {
      for (const entry of waiting.values()) entry.reject(new Error(event.message || 'The embedding engine failed to start'))
      waiting.clear()
      worker = null
    }
  }
  return worker
}

function embed(texts: string[]): Promise<number[][]> {
  const id = nextId++
  return new Promise((resolve, reject) => {
    waiting.set(id, { resolve, reject })
    getWorker().postMessage({ type: 'embed', id, texts } satisfies EmbedRequest)
  })
}

/** Projects with an indexing pass running or due, so bursts of edits start one pass each. */
const running = new Set<string>()
const again = new Set<string>()
const timers = new Map<string, ReturnType<typeof setTimeout>>()

async function indexProject(root: string) {
  if (running.has(root)) { again.add(root); return }
  running.add(root)
  try {
    for (;;) {
      const batch = await invoke<Batch>('semantic_chunks', { root, model: EMBEDDING_MODEL, limit: 16 })
      if (batch.chunks.length === 0) break
      const vectors = await embed(batch.chunks.map(chunk => chunk.text))
      const items = batch.chunks.map((chunk, index) => ({ path: chunk.path, mtime: chunk.mtime, chunk: chunk.chunk, start: chunk.start, end: chunk.end, vector: vectors[index] }))
      await invoke('semantic_store', { root, model: EMBEDDING_MODEL, items })
      // Give the interface room between batches.
      await new Promise(resolve => setTimeout(resolve, 50))
    }
  } catch (cause) {
    console.warn('Semantic indexing stopped:', cause)
  } finally {
    running.delete(root)
    if (again.delete(root)) schedule(root)
  }
}

function schedule(root: string, delay = 1_500) {
  clearTimeout(timers.get(root))
  timers.set(root, setTimeout(() => { timers.delete(root); void indexProject(root) }, delay))
}

let started = false

/** Listens for the agent's embedding requests. Call once at startup. */
export function startSemanticBridge() {
  if (started) return
  started = true
  void listen<{ id: number; root: string; text: string }>('semantic://embed', async ({ payload }) => {
    try {
      const [vector] = await embed([payload.text])
      await invoke('semantic_reply', { id: payload.id, model: EMBEDDING_MODEL, vector })
    } catch (cause) {
      await invoke('semantic_reply', { id: payload.id, error: cause instanceof Error ? cause.message : String(cause) })
    }
    schedule(payload.root, 0)
  }).catch(() => undefined)
  // Outside the desktop app (a plain browser) there are no events; nothing to do.
  void listen<{ root: string }>('semantic://index', ({ payload }) => schedule(payload.root)).catch(() => undefined)
}
