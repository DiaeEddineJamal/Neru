import { isTauri } from '@tauri-apps/api/core'
import { getCurrentWindow, UserAttentionType } from '@tauri-apps/api/window'
import { isPermissionGranted, requestPermission, sendNotification } from '@tauri-apps/plugin-notification'

let permission: Promise<boolean> | null = null

async function allowed() {
  permission ??= (async () => (await isPermissionGranted()) || (await requestPermission()) === 'granted')().catch(() => false)
  return permission
}

/**
 * Tells you about a session that needs you while you are elsewhere: a system notification plus a
 * taskbar/dock flash. Stays quiet when the window is focused on that same session.
 */
export async function notifyUser(title: string, body: string) {
  if (!isTauri()) return
  try {
    const window = getCurrentWindow()
    if (!(await window.isFocused())) void window.requestUserAttention(UserAttentionType.Informational).catch(() => undefined)
  } catch { /* not in the desktop shell */ }
  if (await allowed()) sendNotification({ title, body })
}
