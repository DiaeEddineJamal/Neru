// Export a document as PDF through the system print dialog (WebView2 offers "Save as PDF").

const PRINT_CSS = `
@page { margin: 18mm 16mm; }
* { box-sizing: border-box; }
html { -webkit-print-color-adjust: exact; print-color-adjust: exact; }
body { margin: 0; color: #1d1d1b; background: #fff; font: 11pt/1.6 -apple-system, "Segoe UI", system-ui, sans-serif; }
h1, h2, h3, h4, h5, h6 { font-family: Georgia, "Times New Roman", serif; font-weight: 600; line-height: 1.25; margin: 1.4em 0 0.5em; break-after: avoid; }
h1 { font-size: 22pt; margin-top: 0; } h2 { font-size: 16pt; border-bottom: 1px solid #ddd; padding-bottom: 0.2em; } h3 { font-size: 13pt; } h4, h5, h6 { font-size: 11pt; }
p, ul, ol, blockquote, pre, table { margin: 0 0 0.8em; }
ul, ol { padding-left: 1.4em; } li { margin: 0.15em 0; }
li:has(> input[type="checkbox"]) { list-style: none; margin-left: -1.3em; }
a { color: #2f5d3a; text-decoration: underline; }
code, pre { font-family: "JetBrains Mono", Consolas, monospace; font-size: 9.5pt; }
code { background: #f2f1ec; border-radius: 3px; padding: 0.05em 0.3em; }
pre { background: #f6f5f0; border: 1px solid #e3e1d8; border-radius: 4px; padding: 0.7em 0.9em; white-space: pre-wrap; word-break: break-word; break-inside: avoid; }
pre code { background: none; padding: 0; }
blockquote { border-left: 3px solid #c9c6b8; color: #555; padding: 0.1em 0 0.1em 0.9em; margin-left: 0; }
table { border-collapse: collapse; width: 100%; break-inside: auto; } tr { break-inside: avoid; }
th, td { border: 1px solid #c9c6b8; padding: 0.35em 0.6em; text-align: left; vertical-align: top; }
th { background: #f2f1ec; font-weight: 600; }
hr { border: 0; border-top: 1px solid #ccc; margin: 1.4em 0; }
img, svg { max-width: 100%; height: auto; }
button, [role="button"], .no-print { display: none !important; }
`

const escape = (text: string) => text.replace(/[&<>"]/g, char => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' })[char]!)

/** Copies rendered markdown out of the app for printing: drops scripts, buttons (copy, etc.) and inline styles tied to app themes. */
export function markdownToPrintHtml(container: HTMLElement): string {
  const copy = container.cloneNode(true) as HTMLElement
  copy.querySelectorAll('script, style, button, [aria-hidden="true"]:not(svg *)').forEach(node => node.remove())
  copy.querySelectorAll<HTMLElement>('[style]').forEach(node => node.removeAttribute('style'))
  copy.querySelectorAll<HTMLElement>('[class]').forEach(node => node.removeAttribute('class'))
  return copy.innerHTML
}

/** Opens the print dialog for `html` in a clean, chrome-free page titled `title` (the default PDF file name). */
export async function printDoc(title: string, html: string): Promise<void> {
  const frame = document.createElement('iframe')
  frame.setAttribute('aria-hidden', 'true')
  frame.style.cssText = 'position:fixed;right:0;bottom:0;width:0;height:0;border:0;visibility:hidden'
  document.body.appendChild(frame)
  // Removing the frame while the dialog is open cancels the job, so wait for afterprint (with a fallback).
  const remove = () => setTimeout(() => frame.remove(), 500)
  try {
    const doc = frame.contentDocument!
    doc.open()
    doc.write(`<!doctype html><html><head><meta charset="utf-8"><title>${escape(title)}</title><style>${PRINT_CSS}</style></head><body>${html}</body></html>`)
    doc.close()
    await doc.fonts?.ready
    await Promise.all([...doc.images].filter(image => !image.complete).map(image => new Promise(done => { image.onload = image.onerror = done })))
    const view = frame.contentWindow!
    view.addEventListener('afterprint', remove, { once: true })
    setTimeout(() => frame.remove(), 10 * 60_000)
    view.focus()
    view.print()
  } catch (cause) {
    remove()
    throw cause
  }
}
