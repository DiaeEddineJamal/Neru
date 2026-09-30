import { useEffect, useState } from 'react'
import { isTauri } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { api } from '../api'
import type { IndexStatus } from '../types'

const same = (a: string, b: string) => {
  const clean = (path: string) => path.replace(/^\\\\\?\\/, '').replace(/[\\/]+$/, '').replace(/\\/g, '/').toLowerCase()
  return clean(a) === clean(b)
}

/**
 * State of the project index for the open project: "indexing" with a running file count while it
 * builds, then "ready". It also asks the backend to look for edits made outside Neru whenever the
 * window regains focus; what it finds arrives as a workspace change and the tree updates itself.
 */
export function useIndexStatus(projectKey: string): IndexStatus | null {
  const [status, setStatus] = useState<IndexStatus | null>(null)
  useEffect(() => {
    if (!isTauri()) return
    let active = true
    let stop: (() => void) | undefined
    let lastRefresh = 0
    setStatus(null)
    void api.indexStatus().then(next => { if (active) setStatus(next) }).catch(() => undefined)
    void listen<IndexStatus>('index://progress', event => { if (active && same(event.payload.root, projectKey)) setStatus(event.payload) }).then(off => { if (active) stop = off; else off() })
    const onFocus = () => {
      if (Date.now() - lastRefresh < 2500) return
      lastRefresh = Date.now()
      void api.indexRefresh().then(next => { if (active) setStatus(next) }).catch(() => undefined)
    }
    window.addEventListener('focus', onFocus)
    return () => { active = false; stop?.(); window.removeEventListener('focus', onFocus) }
  }, [projectKey])
  return status
}

export const formatCount = (value: number) => value.toLocaleString('en-US')
