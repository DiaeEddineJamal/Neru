import { check, type Update } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'

/** Asks GitHub Releases for a newer, signed build. Null when up to date, offline, or in a browser. */
export async function findUpdate(): Promise<Update | null> {
  try {
    return await check({ timeout: 15_000 })
  } catch {
    return null
  }
}

/** Downloads and installs an update, reporting progress from 0 to 1, then restarts Neru. */
export async function installUpdate(update: Update, onProgress: (share: number | null) => void) {
  let total = 0
  let received = 0
  await update.downloadAndInstall(event => {
    if (event.event === 'Started') total = event.data.contentLength ?? 0
    else if (event.event === 'Progress') { received += event.data.chunkLength; onProgress(total ? Math.min(1, received / total) : null) }
    else onProgress(1)
  })
  await relaunch()
}

export type { Update }
