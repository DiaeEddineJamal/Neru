import { useSyncExternalStore } from 'react'
import { api } from '../api'
import type { CitationStyle } from '../types'

// One style for every bibliography on screen, kept in the core (research.json) so reports use it too.
let style: CitationStyle = 'apa'
let loaded = false
const listeners = new Set<() => void>()
const subscribe = (listener: () => void) => {
  listeners.add(listener)
  if (!loaded) { loaded = true; void api.citationStyle().then(value => { style = value; listeners.forEach(notify => notify()) }).catch(() => undefined) }
  return () => { listeners.delete(listener) }
}
export function setCitationStyle(next: CitationStyle) {
  style = next
  listeners.forEach(notify => notify())
  void api.setCitationStyle(next).catch(() => undefined)
}
export const useCitationStyle = () => useSyncExternalStore(subscribe, () => style)

/** Asks where to save, then writes `contents` there. False when the user cancelled. */
export async function saveTextFile(defaultName: string, filter: { name: string; extension: string }, contents: string) {
  const { save } = await import('@tauri-apps/plugin-dialog')
  const path = await save({ defaultPath: defaultName, filters: [{ name: filter.name, extensions: [filter.extension] }] })
  if (!path) return false
  await api.saveStudyFile(path, contents)
  return true
}
