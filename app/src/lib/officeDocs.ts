// Slide decks and spreadsheets made by create_document (paper.rs writes them as Markdown): parsed for the
// document panel's preview, and built into PowerPoint (.pptx, via pptxgenjs) and Excel (.xlsx, written
// directly as SpreadsheetML) files. The libraries load only on export.
import { parseChart, type ChartSpec } from './visual/chart'
import { blocks, inlines, plain, type Block } from './paperBlocks'

// ---------- slides ----------

export type Slide = { title: string; body: string; notes: string }

const FENCE = /^\s*(`{3,}|~{3,})/

/** Splits a deck at its `## ` lines (outside code fences). Text before the first is the title slide's subtitle. */
export function parseSlides(markdown: string, deckTitle = ''): Slide[] {
  const slides: Slide[] = []
  let current: { title: string; lines: string[] } | null = null
  let fence: string | null = null
  const lead: string[] = []
  for (const line of markdown.replace(/\r\n?/g, '\n').split('\n')) {
    const mark = FENCE.exec(line)?.[1]
    if (mark && (!fence || line.trim().startsWith(fence))) fence = fence ? null : mark
    const heading = !fence && /^##\s+(.+?)\s*#*\s*$/.exec(line)
    if (heading && !line.startsWith('###')) {
      if (current) slides.push(finish(current))
      current = { title: heading[1].trim(), lines: [] }
    } else if (current) current.lines.push(line)
    else lead.push(line)
  }
  if (current) slides.push(finish(current))
  if (lead.join('').trim()) slides.unshift({ title: deckTitle, body: lead.join('\n').trim(), notes: '' })
  return slides
}

function finish({ title, lines }: { title: string; lines: string[] }): Slide {
  const notes: string[] = []
  const body: string[] = []
  let inNotes = false
  for (const line of lines) {
    const note = /^\s*(?:\*\*)?(?:speaker\s+)?notes?(?:\*\*)?\s*:\s*(?:\*\*)?\s*(.*)$/i.exec(line)
    if (note) { inNotes = true; if (note[1].trim()) notes.push(note[1].trim()); continue }
    if (inNotes && line.trim() && !FENCE.test(line) && !/^\s*[-*|#]/.test(line)) { notes.push(line.trim()); continue }
    inNotes = false
    body.push(line)
  }
  return { title, body: body.join('\n').trim(), notes: notes.join(' ') }
}

/** The ```chart fences in a slide body, parsed, and the body without them. */
export function slideParts(body: string): { text: string; charts: ChartSpec[] } {
  const charts: ChartSpec[] = []
  const text = body.replace(/^\s*```chart[^\n]*\n([\s\S]*?)\n\s*```\s*$/gm, (_, source: string) => {
    const parsed = parseChart(source.trim())
    if (parsed.ok) charts.push(parsed.spec)
    return ''
  })
  return { text: text.replace(/^\s*```mermaid[^\n]*\n[\s\S]*?\n\s*```\s*$/gm, '').trim(), charts }
}

// ---------- sheets ----------

export type Cell = { raw: string; value: string | number | null; formula?: string; error?: boolean }
export type Sheet = { name: string; header: string[]; rows: Cell[][]; charts: string[] }

const splitRow = (line: string) => line.trim().replace(/^\||\|$/g, '').split(/(?<!\\)\|/).map(cell => cell.trim().replace(/\\\|/g, '|'))
const isRule = (line: string) => /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/.test(line)
const unmark = (text: string) => text.replace(/\*\*([^*]+)\*\*/g, '$1').replace(/__([^_]+)__/g, '$1').replace(/`([^`]+)`/g, '$1')

/** A cell's text as a number when it reads as one (1,234.5 · 12% · $40 · -3). */
export function asNumber(text: string): number | null {
  const clean = text.replace(/[\s,]/g, '').replace(/^[$€£¥]|[$€£¥]$/g, '')
  if (!/^[-+]?(\d+\.?\d*|\.\d+)(e[-+]?\d+)?%?$/i.test(clean)) return null
  const value = Number(clean.replace('%', ''))
  return Number.isFinite(value) ? clean.endsWith('%') ? value / 100 : value : null
}

/** Splits a workbook at its `## ` lines; each sheet is the first pipe table under its name. */
export function parseSheets(markdown: string, fallbackName = 'Sheet1'): Sheet[] {
  const lines = markdown.replace(/\r\n?/g, '\n').split('\n')
  const sheets: Sheet[] = []
  let name = ''
  let fence: string[] | null = null
  let fenceTag = ''
  const current = () => sheets[sheets.length - 1]
  for (let index = 0; index < lines.length; index++) {
    const line = lines[index]
    if (fence) {
      if (FENCE.test(line)) { if (fenceTag === 'chart' && current()) current().charts.push(fence.join('\n')); fence = null } else fence.push(line)
      continue
    }
    const open = /^\s*(?:`{3,}|~{3,})\s*([\w-]*)/.exec(line)
    if (open) { fence = []; fenceTag = open[1].toLowerCase(); continue }
    const heading = /^#{1,3}\s+(.+?)\s*#*\s*$/.exec(line.trim())
    if (heading) { name = unmark(heading[1]); continue }
    if (line.trim().startsWith('|') && index + 1 < lines.length && isRule(lines[index + 1])) {
      const header = splitRow(line).map(unmark)
      const rows: Cell[][] = []
      index += 2
      while (index < lines.length && lines[index].trim().startsWith('|')) {
        const cells = splitRow(lines[index])
        rows.push(header.map((_, at) => cellOf(unmark(cells[at] ?? ''))))
        index++
      }
      index--
      const used = new Set(sheets.map(sheet => sheet.name.toLowerCase()))
      let base = sheetName(name || (sheets.length ? `Sheet${sheets.length + 1}` : fallbackName)), unique = base, n = 2
      while (used.has(unique.toLowerCase())) unique = `${base.slice(0, 28)} ${n++}`
      sheets.push({ name: unique, header, rows, charts: [] })
      name = ''
    }
  }
  sheets.forEach(evaluate)
  return sheets
}

function cellOf(raw: string): Cell {
  if (raw.startsWith('=') && raw.length > 1) return { raw, value: null, formula: raw.slice(1) }
  const number = asNumber(raw)
  return { raw, value: number ?? (raw === '' ? null : raw) }
}

/** Excel's sheet-name rules: at most 31 characters, none of : \ / ? * [ ]. */
const sheetName = (name: string) => name.replace(/[:\\/?*[\]]/g, ' ').replace(/\s+/g, ' ').trim().slice(0, 31) || 'Sheet'

/** Column letters for a zero-based index: 0 → A, 26 → AA. */
export function column(index: number): string {
  let name = ''
  for (let n = index + 1; n > 0; n = Math.floor((n - 1) / 26)) name = String.fromCharCode(65 + ((n - 1) % 26)) + name
  return name
}
const columnIndex = (letters: string) => [...letters.toUpperCase()].reduce((sum, letter) => sum * 26 + letter.charCodeAt(0) - 64, 0) - 1

/**
 * Works out formula cells for the preview (Excel recalculates them on open). Covers what the writer is
 * told to use: + - * / ^, parentheses, A1 references and ranges, and SUM, AVERAGE, MIN, MAX, COUNT,
 * ROUND and ABS. Anything else is left showing its formula.
 */
export function evaluate(sheet: Sheet) {
  const seen = new Set<string>()
  const valueAt = (col: number, row: number): number | string | null => {
    // Row 1 is the header row; data starts at row 2.
    if (row === 1) return sheet.header[col] ?? null
    const cell = sheet.rows[row - 2]?.[col]
    if (!cell) return null
    if (cell.formula && cell.value === null && !cell.error) {
      const key = `${col}:${row}`
      if (seen.has(key)) throw new Error('cycle')
      seen.add(key)
      try { cell.value = compute(cell.formula, valueAt) } catch { cell.error = true }
      seen.delete(key)
    }
    if (cell.error) throw new Error('ref')
    return cell.value
  }
  sheet.rows.forEach((row, at) => row.forEach((cell, col) => { if (cell.formula) { try { valueAt(col, at + 2) } catch { cell.error = true } } }))
}

type Lookup = (col: number, row: number) => number | string | null

function compute(formula: string, at: Lookup): number {
  const tokens = formula.match(/\$?[A-Za-z]{1,3}\$?\d+(?::\$?[A-Za-z]{1,3}\$?\d+)?|[A-Za-z]+(?=\()|\d+\.?\d*(?:e[-+]?\d+)?%?|\.\d+|[-+*/^(),]/gi) ?? []
  if (tokens.join('').replace(/\s/g, '') !== formula.replace(/\s/g, '')) throw new Error('unsupported')
  let pos = 0
  const peek = () => tokens[pos]
  const next = () => tokens[pos++]
  const ref = (text: string) => { const match = /^\$?([A-Za-z]{1,3})\$?(\d+)$/.exec(text)!; return [columnIndex(match[1]), Number(match[2])] as const }
  const numeric = (value: number | string | null) => typeof value === 'number' ? value : value === null || value === '' ? 0 : (() => { throw new Error('text') })()
  const range = (text: string): (number | string | null)[] => {
    const [from, to] = text.split(':').map(ref)
    const out: (number | string | null)[] = []
    for (let row = Math.min(from[1], to[1]); row <= Math.max(from[1], to[1]); row++)
      for (let col = Math.min(from[0], to[0]); col <= Math.max(from[0], to[0]); col++) out.push(at(col, row))
    return out
  }
  const args = (): (number | string | null)[] => {
    const values: (number | string | null)[] = []
    if (next() !== '(') throw new Error('syntax')
    if (peek() === ')') { next(); return values }
    for (;;) {
      const token = peek()
      if (token && token.includes(':')) { next(); values.push(...range(token)) } else values.push(expression())
      const sep = next()
      if (sep === ')') return values
      if (sep !== ',') throw new Error('syntax')
    }
  }
  const call = (name: string): number => {
    const values = args()
    const numbers = values.filter((value): value is number => typeof value === 'number')
    switch (name.toUpperCase()) {
      case 'SUM': return numbers.reduce((sum, value) => sum + value, 0)
      case 'AVERAGE': case 'AVG': if (!numbers.length) throw new Error('div0'); return numbers.reduce((sum, value) => sum + value, 0) / numbers.length
      case 'MIN': return numbers.length ? Math.min(...numbers) : 0
      case 'MAX': return numbers.length ? Math.max(...numbers) : 0
      case 'COUNT': return numbers.length
      case 'ABS': return Math.abs(numeric(values[0] ?? 0))
      case 'ROUND': { const digits = numeric(values[1] ?? 0), factor = 10 ** digits; return Math.round(numeric(values[0] ?? 0) * factor) / factor }
      default: throw new Error('unsupported')
    }
  }
  const atom = (): number => {
    const token = next()
    if (token === undefined) throw new Error('syntax')
    if (token === '(') { const value = expression(); if (next() !== ')') throw new Error('syntax'); return value }
    if (token === '-') return -atom()
    if (token === '+') return atom()
    if (/^[A-Za-z]+$/.test(token) && peek() === '(') return call(token)
    if (/^\$?[A-Za-z]{1,3}\$?\d+$/.test(token)) { const [col, row] = ref(token); return numeric(at(col, row)) }
    const number = asNumber(token)
    if (number === null) throw new Error('syntax')
    return number
  }
  const power = (): number => { let value = atom(); while (peek() === '^') { next(); value = value ** atom() } return value }
  const term = (): number => {
    let value = power()
    while (peek() === '*' || peek() === '/') {
      const op = next(), right = power()
      if (op === '/' && right === 0) throw new Error('div0')
      value = op === '*' ? value * right : value / right
    }
    return value
  }
  function expression(): number {
    let value = term()
    while (peek() === '+' || peek() === '-') value = next() === '+' ? value + term() : value - term()
    return value
  }
  const result = expression()
  if (pos !== tokens.length || !Number.isFinite(result)) throw new Error('syntax')
  return result
}

/** A computed value as the preview shows it. */
export function display(cell: Cell): string {
  if (cell.error) return cell.raw
  if (typeof cell.value === 'number') {
    if (cell.raw.trim().endsWith('%')) return `${+(cell.value * 100).toFixed(4)}%`
    return Number.isInteger(cell.value) ? cell.value.toLocaleString() : cell.value.toLocaleString(undefined, { maximumFractionDigits: 4 })
  }
  return cell.value ?? ''
}

// ---------- CSV ----------

export function toCsv(sheet: Sheet): string {
  const quote = (text: string) => /[",\n\r]/.test(text) ? `"${text.replace(/"/g, '""')}"` : text
  const value = (cell: Cell) => cell.error ? cell.raw : typeof cell.value === 'number' ? String(cell.value) : cell.value ?? ''
  return [sheet.header.map(quote).join(','), ...sheet.rows.map(row => row.map(cell => quote(value(cell))).join(','))].join('\r\n') + '\r\n'
}

// ---------- XLSX ----------

const xml = (text: string) => text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f]/g, '')

function sheetXml(sheet: Sheet): string {
  const widths = sheet.header.map((head, col) => Math.min(60, Math.max(8, head.length + 2, ...sheet.rows.map(row => display(row[col] ?? { raw: '', value: null }).length + 2))))
  const cellXml = (ref: string, cell: Cell, style: number) => {
    if (cell.formula) {
      const cached = typeof cell.value === 'number' && !cell.error ? `<v>${cell.value}</v>` : ''
      return `<c r="${ref}" s="${style}"><f>${xml(cell.formula)}</f>${cached}</c>`
    }
    if (typeof cell.value === 'number') return `<c r="${ref}" s="${cell.raw.trim().endsWith('%') ? 3 : style}"><v>${cell.value}</v></c>`
    if (cell.value === null || cell.value === '') return ''
    return `<c r="${ref}" s="${style}" t="inlineStr"><is><t xml:space="preserve">${xml(String(cell.value))}</t></is></c>`
  }
  const total = (row: Cell[]) => /^(total|sum|grand total|totals)\b/i.test(row[0]?.raw ?? '')
  const rows = [
    `<row r="1">${sheet.header.map((head, col) => `<c r="${column(col)}1" s="1" t="inlineStr"><is><t xml:space="preserve">${xml(head)}</t></is></c>`).join('')}</row>`,
    ...sheet.rows.map((row, at) => `<row r="${at + 2}">${row.map((cell, col) => cellXml(`${column(col)}${at + 2}`, cell, total(row) ? 2 : 0)).join('')}</row>`),
  ]
  const last = `${column(Math.max(0, sheet.header.length - 1))}${sheet.rows.length + 1}`
  return `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<dimension ref="A1:${last}"/>
<sheetViews><sheetView workbookViewId="0"><pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/></sheetView></sheetViews>
<sheetFormatPr defaultRowHeight="15"/>
<cols>${widths.map((width, col) => `<col min="${col + 1}" max="${col + 1}" width="${width}" customWidth="1"/>`).join('')}</cols>
<sheetData>${rows.join('')}</sheetData>
${sheet.header.length ? `<autoFilter ref="A1:${last}"/>` : ''}
</worksheet>`
}

/** An .xlsx workbook: a bold, frozen, filterable header row on each sheet; formulas kept as formulas. */
export async function toXlsx(sheets: Sheet[], title: string): Promise<Uint8Array> {
  const fflate = await import('fflate')
  // Copied into this realm's Uint8Array: fflate takes anything else for a folder (as in jsdom tests).
  const strToU8 = (text: string) => Uint8Array.from(fflate.strToU8(text))
  const zipSync = fflate.zipSync
  const list = sheets.length ? sheets : [{ name: 'Sheet1', header: [], rows: [], charts: [] }]
  const files: Record<string, Uint8Array> = {
    '[Content_Types].xml': strToU8(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>${list.map((_, at) => `<Override PartName="/xl/worksheets/sheet${at + 1}.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>`).join('')}</Types>`),
    '_rels/.rels': strToU8(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/></Relationships>`),
    'docProps/core.xml': strToU8(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:title>${xml(title)}</dc:title><dc:creator>Neru</dc:creator><dcterms:created xsi:type="dcterms:W3CDTF">${new Date().toISOString().replace(/\.\d+Z$/, 'Z')}</dcterms:created></cp:coreProperties>`),
    'xl/workbook.xml': strToU8(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>${list.map((sheet, at) => `<sheet name="${xml(sheet.name)}" sheetId="${at + 1}" r:id="rId${at + 1}"/>`).join('')}</sheets>${list.some(sheet => sheet.header.length) ? `<definedNames>${list.map((sheet, at) => sheet.header.length ? `<definedName name="_xlnm._FilterDatabase" localSheetId="${at}" hidden="1">'${xml(sheet.name.replace(/'/g, "''"))}'!$A$1:$${column(sheet.header.length - 1)}$${sheet.rows.length + 1}</definedName>` : '').join('')}</definedNames>` : ''}<calcPr calcId="191029" fullCalcOnLoad="1"/></workbook>`),
    'xl/_rels/workbook.xml.rels': strToU8(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">${list.map((_, at) => `<Relationship Id="rId${at + 1}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet${at + 1}.xml"/>`).join('')}<Relationship Id="rId${list.length + 1}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>`),
    // Styles: 0 plain, 1 header (bold on a light moss fill, bottom border), 2 totals (bold), 3 percent.
    'xl/styles.xml': strToU8(`<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="2"><font><sz val="11"/><name val="Calibri"/><family val="2"/></font><font><b/><sz val="11"/><name val="Calibri"/><family val="2"/></font></fonts><fills count="3"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FFDFE9DF"/><bgColor indexed="64"/></patternFill></fill></fills><borders count="2"><border><left/><right/><top/><bottom/><diagonal/></border><border><left/><right/><top/><bottom style="thin"><color rgb="FF54745B"/></bottom><diagonal/></border></borders><cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs><cellXfs count="4"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/><xf numFmtId="0" fontId="1" fillId="2" borderId="1" xfId="0" applyFont="1" applyFill="1" applyBorder="1"/><xf numFmtId="0" fontId="1" fillId="0" borderId="0" xfId="0" applyFont="1"/><xf numFmtId="10" fontId="0" fillId="0" borderId="0" xfId="0" applyNumberFormat="1"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>`),
  }
  list.forEach((sheet, at) => { files[`xl/worksheets/sheet${at + 1}.xml`] = strToU8(sheetXml(sheet)) })
  return zipSync(files, { level: 6 })
}

// ---------- PPTX ----------

const ACCENT = '54745B'
const INK = '1C1C1A'
const MUTED = '6A6A64'

type Bullet = { text: string; options: Record<string, unknown> }

function bulletsOf(list: Block[]): Bullet[] {
  const out: Bullet[] = []
  for (const block of list) {
    if (block.kind === 'list') block.items.forEach(item => out.push({ text: plain(item), options: { bullet: block.ordered ? { type: 'number' } : { code: '2022' }, paraSpaceAfter: 6 } }))
    else if (block.kind === 'paragraph' || block.kind === 'quote') out.push({ text: plain(block.inlines), options: { paraSpaceAfter: 8, italic: block.kind === 'quote' } })
    else if (block.kind === 'heading') out.push({ text: plain(block.inlines), options: { bold: true, color: ACCENT, paraSpaceAfter: 4 } })
  }
  // pptxgenjs breaks lines between runs only when asked to.
  return out.map((bullet, index) => ({ ...bullet, options: { ...bullet.options, breakLine: index < out.length - 1 } }))
}

/** Indented "- " lines become second-level bullets (paperBlocks flattens them). */
function nestedBullets(body: string): Bullet[] | null {
  const lines = body.split('\n').filter(line => line.trim())
  if (!lines.length || !lines.every(line => /^\s*([-*+]|\d+[.)])\s+/.test(line))) return null
  return lines.map((line, index) => {
    const depth = Math.min(3, Math.floor((/^\s*/.exec(line)?.[0].length ?? 0) / 2))
    const ordered = /^\s*\d+[.)]/.test(line)
    const text = plain(inlines(line.trim().replace(/^([-*+]|\d+[.)])\s+/, '')))
    return { text, options: { bullet: ordered ? { type: 'number' } : { code: depth ? '2013' : '2022' }, indentLevel: depth, paraSpaceAfter: 6, fontSize: depth ? 18 : 22, breakLine: index < lines.length - 1 } }
  })
}

/** A PowerPoint deck: a title slide, then one slide per `## ` section with its bullets, table or native chart. */
export async function toPptx(markdown: string, title: string): Promise<Uint8Array> {
  const { default: PptxGenJS } = await import('pptxgenjs')
  const pptx = new PptxGenJS()
  pptx.layout = 'LAYOUT_WIDE' // 13.33 × 7.5 in
  pptx.title = title
  pptx.author = 'Neru'
  const slides = parseSlides(markdown, title)
  slides.forEach((slide, index) => {
    const page = pptx.addSlide()
    page.background = { color: 'FFFFFF' }
    const { text, charts } = slideParts(slide.body)
    if (index === 0) {
      page.addShape('rect', { x: 0, y: 0, w: 13.33, h: 0.18, fill: { color: ACCENT }, line: { color: ACCENT } })
      page.addText(slide.title || title, { x: 0.9, y: 2.3, w: 11.5, h: 1.6, fontSize: 40, bold: true, color: INK, fontFace: 'Calibri', valign: 'bottom' })
      const subtitle = plain(blocks(text).flatMap(block => block.kind === 'paragraph' || block.kind === 'heading' || block.kind === 'quote' ? block.inlines : []))
      if (subtitle) page.addText(subtitle, { x: 0.9, y: 4.0, w: 11.5, h: 1.0, fontSize: 20, color: MUTED, fontFace: 'Calibri', valign: 'top' })
    } else {
      page.addText(slide.title, { x: 0.6, y: 0.35, w: 12.1, h: 0.9, fontSize: 30, bold: true, color: INK, fontFace: 'Calibri', valign: 'middle' })
      page.addShape('line', { x: 0.6, y: 1.25, w: 1.2, h: 0, line: { color: ACCENT, width: 3 } })
      const parsed = blocks(text)
      const table = parsed.find(block => block.kind === 'table')
      const rest = parsed.filter(block => block.kind !== 'table')
      const chart = charts[0]
      const hasText = rest.length > 0
      const textBox = { x: 0.6, y: 1.55, w: chart || table ? 5.6 : 12.1, h: 5.4 }
      if (hasText) {
        const items = (!table && nestedBullets(text)) || bulletsOf(rest)
        page.addText(items as never, { ...textBox, fontSize: 22, color: INK, fontFace: 'Calibri', valign: 'top', paraSpaceAfter: 6 })
      }
      const area = hasText ? { x: 6.5, y: 1.55, w: 6.2, h: 5.4 } : { x: 0.6, y: 1.55, w: 12.1, h: 5.4 }
      if (chart) addChart(pptx, page, chart, area)
      else if (table) {
        const rows = [table.header.map(cell => ({ text: plain(cell), options: { bold: true, color: 'FFFFFF', fill: { color: ACCENT } } })), ...table.rows.map(row => table.header.map((_, at) => ({ text: plain(row[at] ?? []) })))]
        page.addTable(rows as never, { ...area, fontSize: rows.length > 10 ? 12 : 14, color: INK, border: { type: 'solid', pt: 0.5, color: 'CFCFC8' }, autoPage: false })
      }
    }
    page.addText(String(index + 1), { x: 12.3, y: 6.95, w: 0.6, h: 0.35, fontSize: 10, color: MUTED, align: 'right' })
    if (slide.notes) page.addNotes(slide.notes)
  })
  const out = await pptx.write({ outputType: 'uint8array' })
  return out instanceof Uint8Array ? out : new Uint8Array(out as ArrayBuffer)
}

const PALETTE = ['54745B', 'C08A3E', '4F7CAC', 'A25750', '7A6AA8', '3E9C9A', 'B5895B', '6A6A64']

function addChart(pptx: InstanceType<typeof import('pptxgenjs').default>, page: ReturnType<InstanceType<typeof import('pptxgenjs').default>['addSlide']>, spec: ChartSpec, area: { x: number; y: number; w: number; h: number }) {
  const types = pptx.ChartType
  const kind = spec.type === 'bar' ? types.bar : spec.type === 'line' ? types.line : spec.type === 'area' ? types.area : spec.type === 'pie' ? types.doughnut : types.scatter
  const data = spec.type === 'scatter'
    ? [{ name: spec.x ?? 'X', values: (spec.series[0]?.points ?? []).map(point => point[0]) }, ...spec.series.map(series => ({ name: series.name, values: (series.points ?? []).map(point => point[1]) }))]
    : (spec.type === 'pie' ? spec.series.slice(0, 1) : spec.series).map(series => ({ name: series.name, labels: spec.labels, values: series.data }))
  page.addChart(kind, data, {
    ...area,
    chartColors: PALETTE,
    showTitle: Boolean(spec.title), title: spec.title, titleFontSize: 14, titleColor: INK,
    showLegend: spec.series.length > 1 || spec.type === 'pie', legendPos: 'b', legendFontSize: 11,
    barGrouping: spec.stacked ? 'stacked' : 'clustered',
    showValAxisTitle: Boolean(spec.y), valAxisTitle: spec.y, showCatAxisTitle: Boolean(spec.x), catAxisTitle: spec.x,
    catAxisLabelFontSize: 11, valAxisLabelFontSize: 11, valGridLine: { color: 'E4E3DC', size: 0.5 },
    showPercent: spec.type === 'pie', holeSize: 55, lineSize: 2, lineDataSymbol: spec.type === 'scatter' ? 'circle' : 'none',
  })
}
