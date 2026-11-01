import { useSyncExternalStore } from 'react'

// Which paper the document panel shows, and the section being written into it right now. A chat card
// opens a paper; paper.rs streams `paper://draft` while it writes and `paper://saved` after each step.

export type LiveDraft = { sessionId: string; id: string; index: number; heading: string; text: string; before?: string; after?: string }
type Saved = { sessionId: string; id: string; status: 'writing' | 'paused' | 'done' }

let open: string | null = null
let live: LiveDraft | null = null
/** Bumped on every save, so the panel reloads the paper it shows. */
let saves = 0
const listeners = new Set<() => void>()
const notify = () => listeners.forEach(listener => listener())
const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener) } }

export function openPaper(id: string) { open = id; notify() }
export function closePaper() { open = null; notify() }
export const useOpenPaper = () => useSyncExternalStore(subscribe, () => open)
export const useLiveDraft = () => useSyncExternalStore(subscribe, () => live)
export const useSaves = () => useSyncExternalStore(subscribe, () => saves)

/** The paper card in a reply, when Research mode wrote one. */
export function paperIdIn(content: string): string | null {
  const block = /```neru-document\n([\s\S]*?)\n```/.exec(content)
  if (!block) return null
  try { return (JSON.parse(block[1]) as { id?: string }).id ?? null } catch { return null }
}

let started = false
/** Listens for the writer's events once. `activeSession` says which chat is on screen. */
export function listenForPapers(activeSession: () => string | null) {
  if (started) return
  started = true
  void import('@tauri-apps/api/event').then(({ listen }) => {
    void listen<LiveDraft>('paper://draft', event => {
      if (event.payload.sessionId !== activeSession()) return
      live = event.payload
      // The panel opens on the paper being written, as Claude opens an artifact it is writing.
      if (open !== event.payload.id) open = event.payload.id
      notify()
    })
    void listen<Saved>('paper://saved', event => {
      if (event.payload.sessionId !== activeSession()) return
      saves += 1
      if (event.payload.status === 'writing' && open !== event.payload.id) open = event.payload.id
      if (live?.id === event.payload.id || event.payload.status !== 'writing') live = null
      notify()
    })
  }).catch(() => undefined)
}
