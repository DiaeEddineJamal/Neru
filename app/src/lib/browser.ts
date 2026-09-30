/** Shared types and helpers for the in-app browser (PreviewPane) and its page bridge. */

export type Route = { frameUrl: string; realUrl: string; proxyOrigin: string | null; realOrigin: string | null; bridged: boolean }

/** Asks a tab to open a URL, or to start the project's preview. `seq` makes each request unique. */
export type TabRequest = { seq: number; url?: string; start?: boolean }

export type Region = { x: number; y: number; w: number; h: number }

/** What the page bridge reports about a picked element. */
export type Picked = {
  tag: string
  id: string
  classes: string[]
  selector: string
  text: string
  attrs: Record<string, string>
  styles: Record<string, string>
  components: string[]
  rect: Region
  page: { x: number; y: number }
  html: string
}

export type Note = { id: string; n: number; page: string; selector: string; region: Region | null; comment: string; text: string; element: Picked | null }

export type ConsoleLevel = 'log' | 'info' | 'warn' | 'error' | 'debug'
export type ConsoleEntry = { id: number; level: ConsoleLevel; kind?: string; text: string; stack?: string; t: number }
export type NetEntry = { id: number; method: string; url: string; status: number; ms: number; ok: boolean; failure: string; t: number }

export type Device = { id: string; name: string; w: number; h: number; kind: 'fill' | 'desktop' | 'tablet' | 'phone' | 'custom' }

export const DEVICES: Device[] = [
  { id: 'fit', name: 'Responsive', w: 0, h: 0, kind: 'fill' },
  { id: 'desktop', name: 'Desktop', w: 1440, h: 900, kind: 'desktop' },
  { id: 'laptop', name: 'Laptop', w: 1280, h: 800, kind: 'desktop' },
  { id: 'tablet', name: 'Tablet', w: 820, h: 1180, kind: 'tablet' },
  { id: 'phone', name: 'Phone', w: 390, h: 844, kind: 'phone' },
  { id: 'small', name: 'Small phone', w: 360, h: 640, kind: 'phone' },
]

export const ZOOMS = [0.5, 0.75, 1, 1.25, 1.5] as const

/** Turns whatever was typed in the address bar into a URL the preview can open, or '' when it is not one. */
export function normalizeUrl(input: string): string {
  const text = input.trim()
  if (!text || /\s/.test(text)) return ''
  if (/^https?:\/\//i.test(text)) return text
  if (/^\d{2,5}$/.test(text)) return `http://127.0.0.1:${text}`
  if (/^:\d{2,5}(\/|$)/.test(text)) return `http://127.0.0.1${text}`
  if (/^(localhost|127\.0\.0\.1|\[::1\])(:\d+)?(\/|$|\?|#)/i.test(text)) return `http://${text}`
  if (/^[\w-]+(\.[\w-]+)+(:\d+)?(\/|$|\?|#)/.test(text)) return `https://${text}`
  return ''
}

export function isLocalUrl(url: string): boolean {
  try { return /^(localhost|127\.0\.0\.1|\[::1\]|::1)$/.test(new URL(url).hostname) } catch { return false }
}

/** The address the person sees for a URL the frame reported. */
export function toRealUrl(href: string, route: Route | null): string {
  if (route?.proxyOrigin && route.realOrigin && href.startsWith(route.proxyOrigin)) return route.realOrigin + href.slice(route.proxyOrigin.length)
  return href
}

export function shortUrl(url: string): string {
  try { const parsed = new URL(url); return `${parsed.pathname}${parsed.search}` || '/' } catch { return url }
}

export function elementName(item: Picked): string {
  return `<${item.tag}${item.id ? `#${item.id}` : ''}${item.classes.length ? `.${item.classes.slice(0, 3).join('.')}` : ''}>`
}

/** A short chip label for a picked element, like Claude's: `<button /> Sign up`. */
export function elementChipLabel(item: Picked): string {
  const text = item.text.replace(/\s+/g, ' ').trim()
  const name = item.components[0] ? `<${item.components[0]} />` : `<${item.tag}${item.classes[0] ? `.${item.classes[0]}` : ''} />`
  return text ? `${name} ${text.length > 48 ? `${text.slice(0, 47)}…` : text}` : name
}

/** One element, written for the coding agent: enough to find it in the source. */
export function describePicked(item: Picked): string {
  const styles = Object.entries(item.styles).slice(0, 14).map(([key, value]) => `${key}: ${value}`).join('; ')
  return [
    `\`${elementName(item)}\` (selector \`${item.selector}\`)`,
    item.components.length ? `React/Vue component chain: ${item.components.join(' ← ')}` : '',
    item.text ? `Text: "${item.text}"` : '',
    `Box: ${item.rect.w}×${item.rect.h} at (${item.page.x}, ${item.page.y}) on the page`,
    styles ? `Styles: ${styles}` : '',
    `HTML: ${item.html}`,
  ].filter(Boolean).join('\n')
}

export function pickedMessage(item: Picked, url: string): string {
  return `About this element in the running preview (${url}):\n\n${describePicked(item)}\n\n`
}

export function notesMessage(notes: Note[], url: string, viewport: string): string {
  const lines = notes.map(note => {
    const where = note.element ? `${describePicked(note.element).split('\n').filter((_, index) => index < 3).join(' · ')}` : `\`${note.selector || note.text}\``
    return `${note.n}. ${where}${note.region ? `\n   Marked area: ${note.region.w}×${note.region.h} at (${note.region.x}, ${note.region.y})` : ''}\n   ${note.comment.replace(/\n/g, '\n   ')}`
  })
  return `Feedback on the running preview at ${url}${viewport ? ` (${viewport})` : ''}. Please make these changes in the source:\n\n${lines.join('\n\n')}\n`
}

export function errorsMessage(entries: (ConsoleEntry | NetEntry)[], url: string): string {
  const lines = entries.slice(-15).map(entry => 'text' in entry ? `- ${entry.kind ? `${entry.kind}: ` : ''}${entry.text}${entry.stack ? `\n    ${entry.stack.split('\n').slice(0, 4).join('\n    ')}` : ''}` : `- ${entry.method} ${entry.url} → ${entry.status || entry.failure || 'failed'}`)
  return `The running preview at ${url} reports these problems. Find the cause in the source and fix it:\n\n${lines.join('\n')}\n`
}

export function readPref<T>(key: string, fallback: T): T {
  try { const raw = localStorage.getItem(key); return raw ? JSON.parse(raw) as T : fallback } catch { return fallback }
}

export function writePref(key: string, value: unknown) {
  try { localStorage.setItem(key, JSON.stringify(value)) } catch { /* storage unavailable */ }
}
