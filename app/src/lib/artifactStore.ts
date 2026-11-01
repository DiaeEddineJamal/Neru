import { useSyncExternalStore } from 'react'

// Which paper the document panel shows. A chat card opens one; the panel closes itself.
let open: string | null = null
const listeners = new Set<() => void>()
const notify = () => listeners.forEach(listener => listener())

export function openPaper(id: string) { open = id; notify() }
export function closePaper() { open = null; notify() }
export const useOpenPaper = () => useSyncExternalStore(listener => { listeners.add(listener); return () => listeners.delete(listener) }, () => open)

/** The paper card in a reply, when Research mode wrote one. */
export function paperIdIn(content: string): string | null {
  const block = /```neru-document\n([\s\S]*?)\n```/.exec(content)
  if (!block) return null
  try { return (JSON.parse(block[1]) as { id?: string }).id ?? null } catch { return null }
}
