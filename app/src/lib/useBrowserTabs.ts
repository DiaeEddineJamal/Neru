import { useCallback, useMemo, useRef, useState } from 'react'
import { api } from '../api'
import type { TabRequest } from './browser'

export interface BrowserTabState { id: number; title: string; url: string; loading: boolean; request: TabRequest | null }

export interface BrowserTabs {
  tabs: BrowserTabState[]
  activeId: number
  select: (id: number) => void
  add: (url?: string) => void
  close: (id: number) => void
  meta: (id: number, meta: { title: string; url: string; loading: boolean }) => void
  /** Opens a URL in the empty active tab, or in a new one. */
  openUrl: (url: string) => void
  /** Starts the project's preview in the active tab (an empty one is made if needed). */
  startPreview: () => void
  /** Shows a preview that is already running, unless the person has pages open. */
  adoptRunning: () => void
  reset: () => void
}

const blank = (id: number): BrowserTabState => ({ id, title: '', url: '', loading: false, request: null })

/** The browser's tabs live above the pane, so links and “Open preview” can reach it before it is mounted. */
export function useBrowserTabs(): BrowserTabs {
  const [tabs, setTabs] = useState<BrowserTabState[]>([blank(1)])
  const [activeId, setActiveId] = useState(1)
  const nextId = useRef(2)
  const seq = useRef(0)
  const tabsRef = useRef(tabs)
  tabsRef.current = tabs
  const activeRef = useRef(activeId)
  activeRef.current = activeId

  const meta = useCallback((id: number, next: { title: string; url: string; loading: boolean }) => {
    setTabs(current => current.some(tab => tab.id === id && (tab.title !== next.title || tab.url !== next.url || tab.loading !== next.loading))
      ? current.map(tab => tab.id === id ? { ...tab, ...next } : tab) : current)
  }, [])

  const request = useCallback((ask: Omit<TabRequest, 'seq'>) => {
    const same = ask.url ? tabsRef.current.find(tab => tab.url === ask.url) : undefined
    if (same) {
      setTabs(list => list.map(tab => tab.id === same.id ? { ...tab, request: { ...ask, seq: ++seq.current } } : tab))
      setActiveId(same.id)
      return
    }
    const current = tabsRef.current.find(tab => tab.id === activeRef.current)
    if (current && !current.url && !current.request) {
      setTabs(list => list.map(tab => tab.id === current.id ? { ...tab, request: { ...ask, seq: ++seq.current } } : tab))
      return
    }
    if (ask.start && current && !current.request && !ask.url) {
      setTabs(list => list.map(tab => tab.id === current.id ? { ...tab, request: { ...ask, seq: ++seq.current } } : tab))
      return
    }
    const id = nextId.current++
    setTabs(list => [...list, { ...blank(id), request: { ...ask, seq: ++seq.current } }])
    setActiveId(id)
  }, [])

  const add = useCallback((url?: string) => {
    const id = nextId.current++
    setTabs(list => [...list, { ...blank(id), request: url ? { url, seq: ++seq.current } : null }])
    setActiveId(id)
  }, [])

  const close = useCallback((id: number) => {
    const list = tabsRef.current
    if (list.length === 1) { setTabs([blank(nextId.current++)]); setActiveId(nextId.current - 1); return }
    const at = list.findIndex(tab => tab.id === id)
    const left = list.filter(tab => tab.id !== id)
    setTabs(left)
    if (id === activeRef.current) setActiveId(left[Math.max(0, at - 1)].id)
  }, [])

  const reset = useCallback(() => { const id = nextId.current++; setTabs([blank(id)]); setActiveId(id) }, [])

  const adoptRunning = useCallback(() => {
    if (tabsRef.current.some(tab => tab.url || tab.request)) return
    void api.previewCurrent().then(current => { if (current?.url && !tabsRef.current.some(tab => tab.url || tab.request)) request({ url: current.url }) }).catch(() => undefined)
  }, [request])

  return useMemo(() => ({
    tabs, activeId, select: setActiveId, add, close, meta,
    openUrl: (url: string) => request({ url }), startPreview: () => request({ start: true }), adoptRunning, reset,
  }), [tabs, activeId, add, close, meta, request, adoptRunning, reset])
}
