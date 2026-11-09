import { invoke } from '@tauri-apps/api/core'
import type { MemoryCategory, MemoryStore } from '../types'

/** The memory summary Neru writes from past chats (global, or one Chat project's), as memory.rs keeps it. */
export interface MemorySummary {
  /** Markdown. */
  text: string
  /** When the text last changed, by Neru or the user (ms). */
  updatedAt: number
  /** When Neru last wrote it from chats (ms); 0 for never. */
  generatedAt: number
  /** The newest chat the last generation read (ms). */
  chatsThrough: number
  /** The user edited it after Neru last wrote it. */
  edited: boolean
  /** A refresh is being written now. */
  generating: boolean
  /** "Generate memory from chat history" is on. */
  enabled: boolean
}

/** What the incognito toggle shows (sessions.rs). */
export interface IncognitoState {
  /** The next new chat starts incognito. */
  next: boolean
  /** The shown session is an incognito chat. */
  active: boolean
  activeId: string | null
  /** The shown session has no messages yet. */
  activeEmpty: boolean
}

const scope = (projectId?: string) => ({ projectId: projectId ?? null })

/** Memory calls that take a scope: no `projectId` is the global memory, one is that Chat project's own. */
export const memoryApi = {
  summary: (projectId?: string) => invoke<MemorySummary>('memory_summary_get', scope(projectId)),
  setSummary: (text: string, projectId?: string) => invoke<MemorySummary>('memory_summary_set', { ...scope(projectId), text }),
  regenerateSummary: (projectId?: string) => invoke<MemorySummary>('memory_summary_regenerate', scope(projectId)),
  reviseSummary: (instruction: string, projectId?: string) => invoke<MemorySummary>('memory_summary_revise', { ...scope(projectId), instruction }),
  projectMemories: (projectId: string) => invoke<MemoryStore>('project_memory_state', { projectId }),
  addMemory: (text: string, category: MemoryCategory, projectId?: string) => invoke<MemoryStore>('add_memory', { text, category, ...scope(projectId) }),
  editMemory: (id: string, text: string, category: MemoryCategory, projectId?: string) => invoke<MemoryStore>('edit_memory', { id, text, category, ...scope(projectId) }),
  deleteMemory: (id: string, projectId?: string) => invoke<MemoryStore>('delete_memory', { id, ...scope(projectId) }),
  clearMemories: (projectId?: string) => invoke<MemoryStore>('clear_memories', scope(projectId)),
  incognito: () => invoke<IncognitoState>('chat_incognito_state'),
  setIncognito: (on: boolean) => invoke<IncognitoState>('set_chat_incognito', { on }),
}

/** Calls `onChange` when memory changes in the background (a learned fact, a rewritten summary). */
export function onMemoryChanged(onChange: () => void): () => void {
  let stop: (() => void) | undefined, live = true
  void import('@tauri-apps/api/event')
    .then(({ listen }) => listen('memory-changed', () => onChange()))
    .then(unlisten => { if (live) stop = unlisten; else unlisten() })
    .catch(() => undefined)
  return () => { live = false; stop?.() }
}

/** "3 Nov 2026, 14:05"-style time for "Updated …". */
export const memoryTime = (millis: number) => new Date(millis).toLocaleString(undefined, { year: 'numeric', month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' })
