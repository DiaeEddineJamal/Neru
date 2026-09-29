/**
 * Text extraction for attachments the model cannot read directly. PDFs go through PDF.js (loaded on
 * first use); Word .docx files are unzipped with the browser's DecompressionStream and their
 * document XML flattened to paragraphs.
 */

const MAX_CHARS = 200_000

async function pdfText(bytes: ArrayBuffer): Promise<string> {
  const [{ GlobalWorkerOptions, getDocument }, worker] = await Promise.all([
    import('pdfjs-dist'),
    import('pdfjs-dist/build/pdf.worker.min.mjs?url'),
  ])
  GlobalWorkerOptions.workerSrc = worker.default
  const task = getDocument({ data: new Uint8Array(bytes), useSystemFonts: true })
  const pdf = await task.promise
  try {
    const pages: string[] = []
    let total = 0
    for (let number = 1; number <= pdf.numPages && total < MAX_CHARS; number += 1) {
      const page = await pdf.getPage(number)
      const content = await page.getTextContent()
      let text = ''
      for (const item of content.items) {
        if (!('str' in item)) continue
        text += item.str + (item.hasEOL ? '\n' : ' ')
      }
      text = text.replace(/[ \t]+\n/g, '\n').replace(/ {2,}/g, ' ').trim()
      pages.push(`[Page ${number}]\n${text}`)
      total += text.length
    }
    const body = pages.join('\n\n')
    if (!body.replace(/\[Page \d+\]/g, '').trim()) throw new Error('This PDF has no selectable text (it may be a scan). Attach page images instead.')
    return body.slice(0, MAX_CHARS)
  } finally {
    void task.destroy()
  }
}

async function inflateRaw(data: Uint8Array): Promise<Uint8Array> {
  const stream = new Blob([data.slice()]).stream().pipeThrough(new DecompressionStream('deflate-raw'))
  return new Uint8Array(await new Response(stream).arrayBuffer())
}

/** Reads one file out of a zip archive by name. */
async function unzipEntry(bytes: ArrayBuffer, wanted: string): Promise<Uint8Array | null> {
  const view = new DataView(bytes)
  let end = -1
  for (let offset = bytes.byteLength - 22; offset >= Math.max(0, bytes.byteLength - 65_557); offset -= 1) {
    if (view.getUint32(offset, true) === 0x06054b50) { end = offset; break }
  }
  if (end < 0) throw new Error('This is not a valid .docx file.')
  const count = view.getUint16(end + 10, true)
  let pointer = view.getUint32(end + 16, true)
  const decoder = new TextDecoder()
  for (let index = 0; index < count; index += 1) {
    if (view.getUint32(pointer, true) !== 0x02014b50) break
    const method = view.getUint16(pointer + 10, true)
    const size = view.getUint32(pointer + 20, true)
    const nameLength = view.getUint16(pointer + 28, true)
    const extraLength = view.getUint16(pointer + 30, true)
    const commentLength = view.getUint16(pointer + 32, true)
    const local = view.getUint32(pointer + 42, true)
    const name = decoder.decode(new Uint8Array(bytes, pointer + 46, nameLength))
    if (name === wanted) {
      const start = local + 30 + view.getUint16(local + 26, true) + view.getUint16(local + 28, true)
      const data = new Uint8Array(bytes, start, size)
      if (method === 0) return data
      if (method === 8) return inflateRaw(data)
      throw new Error('This .docx uses an unsupported compression method.')
    }
    pointer += 46 + nameLength + extraLength + commentLength
  }
  return null
}

async function docxText(bytes: ArrayBuffer): Promise<string> {
  const xml = await unzipEntry(bytes, 'word/document.xml')
  if (!xml) throw new Error('This file has no Word document inside.')
  const doc = new DOMParser().parseFromString(new TextDecoder().decode(xml), 'application/xml')
  const paragraphs: string[] = []
  for (const paragraph of Array.from(doc.getElementsByTagName('w:p'))) {
    let text = ''
    const walk = (node: Element) => {
      for (const child of Array.from(node.children)) {
        if (child.tagName === 'w:t') text += child.textContent ?? ''
        else if (child.tagName === 'w:tab') text += '\t'
        else if (child.tagName === 'w:br' || child.tagName === 'w:cr') text += '\n'
        else walk(child)
      }
    }
    walk(paragraph)
    paragraphs.push(text)
  }
  const body = paragraphs.join('\n').replace(/\n{3,}/g, '\n\n').trim()
  if (!body) throw new Error('This Word document has no text.')
  return body.slice(0, MAX_CHARS)
}

/** Extracts readable text from a PDF or Word document's bytes. */
export function extractDocumentText(kind: 'pdf' | 'docx', bytes: ArrayBuffer): Promise<string> {
  return kind === 'pdf' ? pdfText(bytes) : docxText(bytes)
}

/** Reads a pasted or dropped image as a data URL, refusing anything over 5 MB. */
export function imageDataUrl(file: Blob): Promise<string> {
  if (!/^image\/(png|jpeg|gif|webp)$/.test(file.type)) return Promise.reject(new Error('Paste a PNG, JPEG, GIF or WebP image.'))
  if (file.size > 5_000_000) return Promise.reject(new Error('Images can be up to 5 MB.'))
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => resolve(String(reader.result))
    reader.onerror = () => reject(reader.error ?? new Error('Could not read the image.'))
    reader.readAsDataURL(file)
  })
}
