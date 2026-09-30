/**
 * Text extraction for attachments the model cannot read directly. PDFs go through PDF.js (loaded on
 * first use); Office files (Word, PowerPoint, Excel, OpenDocument) are unzipped with the browser's
 * DecompressionStream and their XML flattened to text; RTF has its control words stripped.
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

/** Reads the files of a zip archive (Office documents are zips) whose names pass `wanted`. */
async function unzip(bytes: ArrayBuffer, wanted: (name: string) => boolean): Promise<Map<string, Uint8Array>> {
  const view = new DataView(bytes)
  let end = -1
  for (let offset = bytes.byteLength - 22; offset >= Math.max(0, bytes.byteLength - 65_557); offset -= 1) {
    if (view.getUint32(offset, true) === 0x06054b50) { end = offset; break }
  }
  if (end < 0) throw new Error('This file is damaged or is not an Office document.')
  const count = view.getUint16(end + 10, true)
  let pointer = view.getUint32(end + 16, true)
  const decoder = new TextDecoder()
  const files = new Map<string, Uint8Array>()
  for (let index = 0; index < count; index += 1) {
    if (view.getUint32(pointer, true) !== 0x02014b50) break
    const method = view.getUint16(pointer + 10, true)
    const size = view.getUint32(pointer + 20, true)
    const nameLength = view.getUint16(pointer + 28, true)
    const extraLength = view.getUint16(pointer + 30, true)
    const commentLength = view.getUint16(pointer + 32, true)
    const local = view.getUint32(pointer + 42, true)
    const name = decoder.decode(new Uint8Array(bytes, pointer + 46, nameLength))
    if (wanted(name)) {
      const start = local + 30 + view.getUint16(local + 26, true) + view.getUint16(local + 28, true)
      const data = new Uint8Array(bytes, start, size)
      if (method === 0) files.set(name, data)
      else if (method === 8) files.set(name, await inflateRaw(data))
      else throw new Error('This document uses a compression Neru cannot read.')
    }
    pointer += 46 + nameLength + extraLength + commentLength
  }
  return files
}

const xmlOf = (data: Uint8Array) => new DOMParser().parseFromString(new TextDecoder().decode(data), 'application/xml')
/** The number in a name like "ppt/slides/slide12.xml", for ordering. */
const numberIn = (name: string) => Number(/(\d+)\.xml$/.exec(name)?.[1] ?? 0)

/** The text of each paragraph element, following runs, tabs and line breaks. */
function paragraphs(doc: Document, paragraph: string, text: string, tab: string, breaks: string[]): string[] {
  return Array.from(doc.getElementsByTagName(paragraph)).map(node => {
    let line = ''
    const walk = (element: Element) => {
      for (const child of Array.from(element.children)) {
        if (child.tagName === text) line += child.textContent ?? ''
        else if (child.tagName === tab) line += '\t'
        else if (breaks.includes(child.tagName)) line += '\n'
        else walk(child)
      }
    }
    walk(node)
    return line
  })
}

function tidy(text: string, empty: string) {
  const body = text.replace(/\n{3,}/g, '\n\n').trim()
  if (!body) throw new Error(empty)
  return body.slice(0, MAX_CHARS)
}

async function docxText(bytes: ArrayBuffer): Promise<string> {
  const xml = (await unzip(bytes, name => name === 'word/document.xml')).get('word/document.xml')
  if (!xml) throw new Error('This file has no Word document inside.')
  return tidy(paragraphs(xmlOf(xml), 'w:p', 'w:t', 'w:tab', ['w:br', 'w:cr']).join('\n'), 'This Word document has no text.')
}

/** PowerPoint: each slide's text in order, with its speaker notes. */
async function pptxText(bytes: ArrayBuffer): Promise<string> {
  const files = await unzip(bytes, name => /^ppt\/(slides\/slide|notesSlides\/notesSlide)\d+\.xml$/.test(name))
  const slides = [...files.keys()].filter(name => name.startsWith('ppt/slides/')).sort((a, b) => numberIn(a) - numberIn(b))
  if (slides.length === 0) throw new Error('This presentation has no slides.')
  const lines = (data: Uint8Array) => paragraphs(xmlOf(data), 'a:p', 'a:t', 'a:tab', ['a:br']).filter(line => line.trim())
  const parts = slides.map(name => {
    const number = numberIn(name)
    const notes = files.get(`ppt/notesSlides/notesSlide${number}.xml`)
    // Notes pages repeat the slide number as text; leave it out.
    const noteText = notes ? lines(notes).filter(line => !/^\d+$/.test(line.trim())).join('\n') : ''
    return `[Slide ${number}]\n${lines(files.get(name)!).join('\n') || '(no text)'}${noteText ? `\nSpeaker notes: ${noteText}` : ''}`
  })
  return tidy(parts.join('\n\n'), 'This presentation has no text.')
}

/** Excel: every sheet as tab-separated rows, with shared strings resolved. */
async function xlsxText(bytes: ArrayBuffer): Promise<string> {
  const files = await unzip(bytes, name => name === 'xl/sharedStrings.xml' || name === 'xl/workbook.xml' || /^xl\/worksheets\/sheet\d+\.xml$/.test(name))
  const shared = files.get('xl/sharedStrings.xml')
  const strings = shared ? Array.from(xmlOf(shared).getElementsByTagName('si')).map(item => Array.from(item.getElementsByTagName('t')).map(t => t.textContent ?? '').join('')) : []
  const workbook = files.get('xl/workbook.xml')
  const names = workbook ? Array.from(xmlOf(workbook).getElementsByTagName('sheet')).map(sheet => sheet.getAttribute('name') ?? '') : []
  const sheets = [...files.keys()].filter(name => name.startsWith('xl/worksheets/')).sort((a, b) => numberIn(a) - numberIn(b))
  const parts = sheets.map((name, index) => {
    const rows = Array.from(xmlOf(files.get(name)!).getElementsByTagName('row')).slice(0, 2000).map(row => Array.from(row.getElementsByTagName('c')).map(cell => {
      const value = cell.getElementsByTagName('v')[0]?.textContent ?? cell.getElementsByTagName('t')[0]?.textContent ?? ''
      return cell.getAttribute('t') === 's' ? strings[Number(value)] ?? '' : value
    }).join('\t'))
    return `[Sheet ${names[index] || index + 1}]\n${rows.join('\n')}`
  })
  return tidy(parts.join('\n\n'), 'This spreadsheet is empty.')
}

/** OpenDocument text, slides and sheets keep their text in content.xml. */
async function openDocumentText(bytes: ArrayBuffer): Promise<string> {
  const xml = (await unzip(bytes, name => name === 'content.xml')).get('content.xml')
  if (!xml) throw new Error('This OpenDocument file has no content.')
  const doc = xmlOf(xml)
  const blocks = [...Array.from(doc.getElementsByTagName('text:h')), ...Array.from(doc.getElementsByTagName('text:p'))]
  return tidy(blocks.map(node => node.textContent ?? '').join('\n'), 'This document has no text.')
}

/** RTF: control words dropped, text kept. */
function rtfText(bytes: ArrayBuffer): string {
  const raw = new TextDecoder('latin1').decode(bytes)
  const text = raw
    .replace(/\\'([0-9a-f]{2})/gi, (_, hex: string) => String.fromCharCode(parseInt(hex, 16)))
    .replace(/\\u(-?\d+)\??/g, (_, code: string) => String.fromCharCode((Number(code) + 65536) % 65536))
    .replace(/\\(par|line)\b ?/g, '\n')
    .replace(/\\tab\b ?/g, '\t')
    .replace(/\{\\\*[^{}]*\}/g, '')
    .replace(/\\[a-z]+-?\d* ?/gi, '')
    .replace(/[{}]/g, '')
  return tidy(text, 'This RTF document has no text.')
}

export type ReadableDocument = 'pdf' | 'docx' | 'pptx' | 'xlsx' | 'odt' | 'odp' | 'ods' | 'rtf'
export const READABLE_DOCUMENTS: ReadableDocument[] = ['pdf', 'docx', 'pptx', 'xlsx', 'odt', 'odp', 'ods', 'rtf']
export const isReadableDocument = (kind: string): kind is ReadableDocument => (READABLE_DOCUMENTS as string[]).includes(kind)

/** Extracts readable text from a PDF or Office document's bytes. */
export async function extractDocumentText(kind: ReadableDocument, bytes: ArrayBuffer): Promise<string> {
  switch (kind) {
    case 'pdf': return pdfText(bytes)
    case 'docx': return docxText(bytes)
    case 'pptx': return pptxText(bytes)
    case 'xlsx': return xlsxText(bytes)
    case 'rtf': return rtfText(bytes)
    default: return openDocumentText(bytes)
  }
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
