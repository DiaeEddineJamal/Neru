// Research-paper exports: Word (docx), PDF (pdfmake with the Tinos serif), LaTeX, a standalone HTML page
// and Markdown, all typeset from the same paper (types.ts Paper). The libraries load only on export.
import { api } from '../api'
import type { Paper } from '../types'
import { blocks, inlines, plain, type Block, type Inline } from './paperBlocks'

export type ExportFormat = 'docx' | 'pdf' | 'html' | 'tex' | 'md' | 'txt' | 'bib'

export const EXPORTS: { format: ExportFormat; label: string; hint: string; extension: string; filter: string }[] = [
  { format: 'docx', label: 'Word document', hint: 'Edit in Microsoft Word, Google Docs or LibreOffice', extension: 'docx', filter: 'Word document' },
  { format: 'pdf', label: 'PDF', hint: 'Ready to print or submit', extension: 'pdf', filter: 'PDF' },
  { format: 'tex', label: 'LaTeX', hint: 'For Overleaf or a LaTeX template', extension: 'tex', filter: 'LaTeX' },
  { format: 'html', label: 'Web page', hint: 'A standalone HTML page', extension: 'html', filter: 'HTML' },
  { format: 'md', label: 'Markdown', hint: 'Plain text with formatting marks', extension: 'md', filter: 'Markdown' },
  { format: 'txt', label: 'Plain text', hint: 'Just the words, no formatting', extension: 'txt', filter: 'Text' },
  { format: 'bib', label: 'BibTeX references', hint: 'The works cited, for Zotero, Mendeley or LaTeX', extension: 'bib', filter: 'BibTeX' },
]

/** Papers number their sections; plain documents (plans, reports) do not. */
const heading = (index: number, text: string, plainDoc = false) => plainDoc ? text : `${index + 1}. ${text}`
const fileName = (paper: Paper) => (paper.title.replace(/[\\/:*?"<>|]+/g, '').replace(/\s+/g, ' ').trim().slice(0, 80) || 'Research paper')

// ---------- Markdown ----------

export function toMarkdown(paper: Paper) {
  const parts = [`# ${paper.title}`]
  const byline = [paper.authors.join(', '), paper.affiliation, paper.date].filter(Boolean).join(' · ')
  if (byline) parts.push(byline)
  if (paper.abstractText) parts.push(`## Abstract\n\n${paper.abstractText}`)
  if (paper.keywords.length) parts.push(`*Keywords:* ${paper.keywords.join(', ')}`)
  paper.sections.forEach((section, index) => parts.push(section.heading.trim() ? `## ${heading(index, section.heading, paper.plain)}\n\n${section.body.trim()}` : section.body.trim()))
  if (paper.references.length) parts.push(`## References\n\n${paper.references.map(reference => reference.text).join('\n\n')}`)
  return `${parts.join('\n\n')}\n`
}

// ---------- HTML ----------

const escapeHtml = (text: string) => text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;')
const htmlRuns = (runs: Inline[]) => runs.map(run => {
  const text = escapeHtml(run.text)
  return run.code ? `<code>${text}</code>` : run.bold ? `<strong>${text}</strong>` : run.italic ? `<em>${text}</em>` : text
}).join('')

function htmlBlock(block: Block): string {
  switch (block.kind) {
    case 'paragraph': return `<p>${htmlRuns(block.inlines)}</p>`
    case 'heading': return `<h${block.level}>${htmlRuns(block.inlines)}</h${block.level}>`
    case 'quote': return `<blockquote>${htmlRuns(block.inlines)}</blockquote>`
    case 'list': return `<${block.ordered ? 'ol' : 'ul'}>${block.items.map(item => `<li>${htmlRuns(item)}</li>`).join('')}</${block.ordered ? 'ol' : 'ul'}>`
    case 'table': return `<table><thead><tr>${block.header.map(cell => `<th>${htmlRuns(cell)}</th>`).join('')}</tr></thead><tbody>${block.rows.map(row => `<tr>${row.map(cell => `<td>${htmlRuns(cell)}</td>`).join('')}</tr>`).join('')}</tbody></table>`
  }
}

export function toHtml(paper: Paper) {
  const hanging = paper.style !== 'ieee'
  const body = [
    `<header><p class="kind">${escapeHtml(paper.kind)}</p><h1>${escapeHtml(paper.title)}</h1>`,
    `<p class="byline">${escapeHtml([paper.authors.join(', '), paper.affiliation].filter(Boolean).join(' · '))}</p><p class="date">${escapeHtml(paper.date)}</p></header>`,
    paper.abstractText ? `<section class="abstract"><h2>Abstract</h2><p>${htmlRuns(inlines(paper.abstractText))}</p>${paper.keywords.length ? `<p class="keywords"><em>Keywords:</em> ${escapeHtml(paper.keywords.join(', '))}</p>` : ''}</section>` : '',
    ...paper.sections.map((section, index) => `<section>${section.heading.trim() ? `<h2>${escapeHtml(heading(index, section.heading, paper.plain))}</h2>` : ''}${blocks(section.body).map(htmlBlock).join('\n')}</section>`),
    paper.references.length ? `<section class="references"><h2>References</h2>${paper.references.map(reference => `<p>${htmlRuns(inlines(reference.text))}</p>`).join('\n')}</section>` : '',
  ].join('\n')
  return `<!doctype html>
<html lang="${escapeHtml(paper.language ? paper.language.slice(0, 2).toLowerCase() : 'en')}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escapeHtml(paper.title)}</title>
<style>
  :root { color-scheme: light; }
  body { margin: 0; background: #f3f2ee; color: #1b1b1b; font: 12pt/1.6 'Times New Roman', Tinos, Georgia, serif; }
  main { max-width: 210mm; margin: 32px auto; padding: 25mm 25mm 30mm; background: #fff; box-shadow: 0 2px 24px rgb(0 0 0 / .08); }
  header { text-align: center; margin-bottom: 28px; }
  .kind { margin: 0 0 6px; color: #666; font: 600 9pt/1.4 system-ui, sans-serif; letter-spacing: .12em; text-transform: uppercase; }
  h1 { margin: 0 0 14px; font-size: 20pt; line-height: 1.25; }
  .byline, .date { margin: 2px 0; color: #444; text-align: center; }
  h2 { margin: 26px 0 8px; font-size: 13.5pt; }
  h3 { margin: 18px 0 6px; font-size: 12pt; font-style: italic; }
  h4 { margin: 14px 0 4px; font-size: 12pt; font-weight: normal; text-decoration: underline; }
  p { margin: 0 0 10px; text-align: justify; hyphens: auto; }
  .abstract { margin: 0 6% 24px; }
  .abstract h2 { text-align: center; font-size: 12pt; }
  .keywords { text-align: left; }
  blockquote { margin: 10px 28px; color: #333; }
  table { width: 100%; margin: 14px 0; border-collapse: collapse; font-size: 10.5pt; }
  th, td { padding: 5px 8px; border-top: 1px solid #999; border-bottom: 1px solid #ccc; text-align: left; vertical-align: top; }
  thead th { border-top: 1.5px solid #222; border-bottom: 1px solid #222; }
  .references p { text-align: left; ${hanging ? 'padding-left: 2.2em; text-indent: -2.2em;' : ''} }
  @media print { body { background: #fff; } main { margin: 0; padding: 0; box-shadow: none; } @page { size: A4; margin: 25mm; } }
</style></head>
<body><main>
${body}
</main></body></html>
`
}

// ---------- LaTeX ----------

const tex = (text: string) => text
  .replace(/\\/g, '\\textbackslash{}').replace(/([&%$#_{}])/g, '\\$1').replace(/~/g, '\\textasciitilde{}').replace(/\^/g, '\\textasciicircum{}')
  .replace(/“/g, '``').replace(/”/g, "''")
const texRuns = (runs: Inline[]) => runs.map(run => run.code ? `\\texttt{${tex(run.text)}}` : run.bold ? `\\textbf{${tex(run.text)}}` : run.italic ? `\\emph{${tex(run.text)}}` : tex(run.text)).join('')

function texBlock(block: Block): string {
  switch (block.kind) {
    case 'paragraph': return texRuns(block.inlines)
    case 'heading': return `\\${block.level === 3 ? 'subsection' : 'subsubsection'}*{${texRuns(block.inlines)}}`
    case 'quote': return `\\begin{quote}\n${texRuns(block.inlines)}\n\\end{quote}`
    case 'list': return `\\begin{${block.ordered ? 'enumerate' : 'itemize'}}\n${block.items.map(item => `  \\item ${texRuns(item)}`).join('\n')}\n\\end{${block.ordered ? 'enumerate' : 'itemize'}}`
    case 'table': {
      const columns = Math.max(block.header.length, ...block.rows.map(row => row.length))
      const spec = Array.from({ length: columns }, () => `>{\\raggedright\\arraybackslash}X`).join('')
      const row = (cells: Inline[][]) => `${Array.from({ length: columns }, (_, index) => texRuns(cells[index] ?? [])).join(' & ')} \\\\`
      return `\\begin{table}[htbp]\n\\small\n\\begin{tabularx}{\\linewidth}{${spec}}\n\\toprule\n${row(block.header)}\n\\midrule\n${block.rows.map(row).join('\n')}\n\\bottomrule\n\\end{tabularx}\n\\end{table}`
    }
  }
}

export function toLatex(paper: Paper) {
  const hanging = paper.style !== 'ieee'
  return `\\documentclass[12pt,a4paper]{article}
\\usepackage[utf8]{inputenc}
\\usepackage[T1]{fontenc}
\\usepackage{mathptmx}
\\usepackage[margin=2.5cm]{geometry}
\\usepackage{setspace}
\\usepackage{booktabs,tabularx}
\\usepackage[hidelinks]{hyperref}
\\onehalfspacing

\\title{${tex(paper.title)}}
\\author{${tex([paper.authors.join(', '), paper.affiliation].filter(Boolean).join(' \\\\ '))}}
\\date{${tex(paper.date)}}

\\begin{document}
\\maketitle

${paper.abstractText ? `\\begin{abstract}\n${texRuns(inlines(paper.abstractText))}\n${paper.keywords.length ? `\n\\noindent\\emph{Keywords:} ${tex(paper.keywords.join(', '))}\n` : ''}\\end{abstract}\n` : ''}
${paper.sections.map(section => `\\section{${tex(section.heading)}}\n\n${blocks(section.body).map(texBlock).join('\n\n')}`).join('\n\n')}

${paper.references.length ? `\\section*{References}\n\\begin{list}{}{\\setlength{\\leftmargin}{${hanging ? '2.2em' : '0pt'}}\\setlength{\\itemindent}{${hanging ? '-2.2em' : '0pt'}}\\setlength{\\itemsep}{4pt}}\n${paper.references.map(reference => `\\item ${texRuns(inlines(reference.text))}`).join('\n')}\n\\end{list}` : ''}

\\end{document}
`
}

// ---------- Word ----------

async function toDocx(paper: Paper): Promise<Uint8Array> {
  const docx = await import('docx')
  const { AlignmentType, BorderStyle, Document, Footer, HeadingLevel, LevelFormat, Packer, PageNumber, Paragraph, Table, TableCell, TableRow, TextRun, WidthType } = docx
  const runs = (items: Inline[], extra: { italics?: boolean; size?: number } = {}) => items.map(run => new TextRun({ text: run.text, bold: run.bold, italics: run.italic || extra.italics, font: run.code ? 'Consolas' : undefined, size: extra.size }))
  const body: (InstanceType<typeof Paragraph> | InstanceType<typeof Table>)[] = []
  const blockOut = (block: Block) => {
    switch (block.kind) {
      case 'paragraph': body.push(new Paragraph({ children: runs(block.inlines), indent: { firstLine: 567 }, alignment: AlignmentType.JUSTIFIED })); break
      case 'heading': body.push(new Paragraph({ children: runs(block.inlines), heading: block.level === 3 ? HeadingLevel.HEADING_2 : HeadingLevel.HEADING_3 })); break
      case 'quote': body.push(new Paragraph({ children: runs(block.inlines, { italics: true }), indent: { left: 720, right: 720 } })); break
      case 'list': block.items.forEach(item => body.push(new Paragraph({ children: runs(item), numbering: { reference: block.ordered ? 'numbered' : 'bullets', level: 0 } }))); break
      case 'table': {
        const line = { style: BorderStyle.SINGLE, size: 4, color: '999999' }
        const none = { style: BorderStyle.NONE, size: 0, color: 'FFFFFF' }
        const cell = (content: Inline[], header: boolean) => new TableCell({
          children: [new Paragraph({ children: content.map(run => new TextRun({ text: run.text, bold: header || run.bold, italics: run.italic, size: 20 })), spacing: { before: 40, after: 40, line: 260 } })],
          borders: { top: header ? { ...line, size: 8, color: '222222' } : line, bottom: header ? { ...line, size: 8, color: '222222' } : line, left: none, right: none },
        })
        body.push(new Table({ width: { size: 100, type: WidthType.PERCENTAGE }, rows: [new TableRow({ tableHeader: true, children: block.header.map(content => cell(content, true)) }), ...block.rows.map(row => new TableRow({ children: block.header.map((_, index) => cell(row[index] ?? [], false)) }))] }))
        body.push(new Paragraph({ children: [] }))
        break
      }
    }
  }
  const centered = (text: string, size: number, options: { bold?: boolean; italics?: boolean; after?: number } = {}) => new Paragraph({ alignment: AlignmentType.CENTER, spacing: { after: options.after ?? 120 }, children: [new TextRun({ text, size, bold: options.bold, italics: options.italics })] })
  body.push(centered(paper.kind.toUpperCase(), 18, { after: 160 }))
  body.push(centered(paper.title, 32, { bold: true, after: 240 }))
  if (paper.authors.length) body.push(centered(paper.authors.join(', '), 24))
  if (paper.affiliation) body.push(centered(paper.affiliation, 24, { italics: true }))
  body.push(centered(paper.date, 22, { after: 360 }))
  if (paper.abstractText) {
    body.push(new Paragraph({ alignment: AlignmentType.CENTER, heading: HeadingLevel.HEADING_1, children: [new TextRun({ text: 'Abstract' })] }))
    body.push(new Paragraph({ children: runs(inlines(paper.abstractText)), alignment: AlignmentType.JUSTIFIED, indent: { left: 567, right: 567 } }))
    if (paper.keywords.length) body.push(new Paragraph({ indent: { left: 567, right: 567 }, spacing: { after: 360 }, children: [new TextRun({ text: 'Keywords: ', italics: true }), new TextRun({ text: paper.keywords.join(', ') })] }))
  }
  paper.sections.forEach((section, index) => {
    if (section.heading.trim()) body.push(new Paragraph({ heading: HeadingLevel.HEADING_1, children: [new TextRun({ text: heading(index, section.heading, paper.plain) })] }))
    blocks(section.body).forEach(blockOut)
  })
  if (paper.references.length) {
    body.push(new Paragraph({ heading: HeadingLevel.HEADING_1, children: [new TextRun({ text: 'References' })] }))
    const hanging = paper.style !== 'ieee'
    paper.references.forEach(reference => body.push(new Paragraph({ children: runs(inlines(reference.text)), indent: hanging ? { left: 567, hanging: 567 } : undefined, spacing: { after: 120, line: 276 } })))
  }
  const document = new Document({
    creator: paper.authors[0] || 'Neru',
    title: paper.title,
    description: paper.kind,
    keywords: paper.keywords.join(', '),
    styles: {
      default: { document: { run: { font: 'Times New Roman', size: 24 }, paragraph: { spacing: { line: 360, after: 120 } } } },
      paragraphStyles: [
        { id: 'Heading1', name: 'Heading 1', basedOn: 'Normal', next: 'Normal', quickFormat: true, run: { font: 'Times New Roman', size: 26, bold: true, color: '000000' }, paragraph: { spacing: { before: 360, after: 120, line: 276 }, keepNext: true } },
        { id: 'Heading2', name: 'Heading 2', basedOn: 'Normal', next: 'Normal', quickFormat: true, run: { font: 'Times New Roman', size: 24, bold: true, italics: true, color: '000000' }, paragraph: { spacing: { before: 240, after: 80, line: 276 }, keepNext: true } },
        { id: 'Heading3', name: 'Heading 3', basedOn: 'Normal', next: 'Normal', quickFormat: true, run: { font: 'Times New Roman', size: 24, italics: true, color: '000000' }, paragraph: { spacing: { before: 200, after: 60, line: 276 }, keepNext: true } },
      ],
    },
    numbering: {
      config: [
        { reference: 'bullets', levels: [{ level: 0, format: LevelFormat.BULLET, text: '•', alignment: AlignmentType.LEFT, style: { paragraph: { indent: { left: 720, hanging: 360 } } } }] },
        { reference: 'numbered', levels: [{ level: 0, format: LevelFormat.DECIMAL, text: '%1.', alignment: AlignmentType.LEFT, style: { paragraph: { indent: { left: 720, hanging: 360 } } } }] },
      ],
    },
    sections: [{
      properties: { page: { size: { width: 11906, height: 16838 }, margin: { top: 1418, bottom: 1418, left: 1418, right: 1418 } } },
      footers: { default: new Footer({ children: [new Paragraph({ alignment: AlignmentType.CENTER, children: [new TextRun({ children: [PageNumber.CURRENT], size: 20 })] })] }) },
      children: body,
    }],
  })
  // Base64 works the same in the window and in tests, unlike Blob.arrayBuffer.
  const binary = atob(await Packer.toBase64String(document))
  return Uint8Array.from(binary, char => char.charCodeAt(0))
}

// ---------- PDF ----------

let fontsReady: Promise<Record<string, string>> | null = null
/** Tinos (metric-compatible with Times New Roman, Apache 2.0), fetched from the app bundle on first use. */
function tinos() {
  fontsReady ??= (async () => {
    const files = import.meta.glob<string>('../assets/fonts/tinos/*.ttf', { eager: true, query: '?url', import: 'default' })
    const vfs: Record<string, string> = {}
    for (const [path, url] of Object.entries(files)) {
      const bytes = new Uint8Array(await (await fetch(url)).arrayBuffer())
      vfs[path.split('/').pop() as string] = toBase64(bytes)
    }
    return vfs
  })()
  return fontsReady
}

// pdfmake's content tree, loosely typed.
type Node = Record<string, unknown> | string

async function toPdf(paper: Paper): Promise<Uint8Array> {
  const [{ default: pdfMake }, vfs] = await Promise.all([import('pdfmake/build/pdfmake'), tinos()])
  const runs = (items: Inline[]): Node[] => items.map(run => ({ text: run.text, bold: run.bold, italics: run.italic, font: 'Tinos' }))
  const content: Node[] = []
  const blockOut = (block: Block) => {
    switch (block.kind) {
      case 'paragraph': content.push({ text: runs(block.inlines), style: 'body' }); break
      case 'heading': content.push({ text: runs(block.inlines), style: block.level === 3 ? 'h2' : 'h3' }); break
      case 'quote': content.push({ text: runs(block.inlines), italics: true, margin: [28, 2, 28, 8] }); break
      case 'list': content.push({ [block.ordered ? 'ol' : 'ul']: block.items.map(item => ({ text: runs(item) })), margin: [12, 0, 0, 8], style: 'list' }); break
      case 'table': {
        const columns = block.header.length
        content.push({
          table: { headerRows: 1, dontBreakRows: true, widths: Array.from({ length: columns }, () => '*'), body: [block.header.map(cell => ({ text: runs(cell), bold: true })), ...block.rows.map(row => block.header.map((_, index) => ({ text: runs(row[index] ?? []) })))] },
          layout: { hLineWidth: (index: number, node: { table: { body: unknown[] } }) => (index === 0 || index === 1 || index === node.table.body.length ? 1 : 0.4), vLineWidth: () => 0, hLineColor: () => '#555555', paddingTop: () => 3, paddingBottom: () => 3 },
          fontSize: 10, margin: [0, 6, 0, 12],
        })
        break
      }
    }
  }
  content.push({ text: paper.kind.toUpperCase(), style: 'kind' })
  content.push({ text: paper.title, style: 'title' })
  if (paper.authors.length) content.push({ text: paper.authors.join(', '), style: 'byline' })
  if (paper.affiliation) content.push({ text: paper.affiliation, style: 'byline', italics: true })
  content.push({ text: paper.date, style: 'byline', margin: [0, 0, 0, 22] })
  if (paper.abstractText) {
    content.push({ text: 'Abstract', style: 'abstractHead' })
    content.push({ text: runs(inlines(paper.abstractText)), style: 'body', margin: [28, 0, 28, 6] })
    if (paper.keywords.length) content.push({ text: [{ text: 'Keywords: ', italics: true }, paper.keywords.join(', ')], margin: [28, 0, 28, 18] })
  }
  paper.sections.forEach((section, index) => {
    if (section.heading.trim()) content.push({ text: heading(index, section.heading, paper.plain), style: 'h1' })
    blocks(section.body).forEach(blockOut)
  })
  if (paper.references.length) {
    content.push({ text: 'References', style: 'h1' })
    const hanging = paper.style !== 'ieee'
    // pdfmake has no hanging indent: the first line is pulled back with a negative leading indent.
    paper.references.forEach(reference => content.push({ text: runs(inlines(reference.text)), margin: [hanging ? 24 : 0, 0, 0, 6], leadingIndent: hanging ? -24 : 0, alignment: 'left', lineHeight: 1.2 }))
  }
  const definition = {
    pageSize: 'A4',
    pageMargins: [72, 72, 72, 72],
    info: { title: paper.title, author: paper.authors.join(', ') || 'Neru', subject: paper.kind, keywords: paper.keywords.join(', ') },
    defaultStyle: { font: 'Tinos', fontSize: 12, lineHeight: 1.35 },
    footer: (page: number) => ({ text: String(page), alignment: 'center', fontSize: 10, margin: [0, 30, 0, 0] }),
    styles: {
      kind: { fontSize: 9, characterSpacing: 1.5, alignment: 'center', color: '#555555', margin: [0, 0, 0, 6] },
      title: { fontSize: 18, bold: true, alignment: 'center', lineHeight: 1.2, margin: [0, 0, 0, 12] },
      byline: { fontSize: 11.5, alignment: 'center', color: '#333333', margin: [0, 0, 0, 2] },
      abstractHead: { fontSize: 12, bold: true, alignment: 'center', margin: [0, 6, 0, 6] },
      h1: { fontSize: 13, bold: true, margin: [0, 16, 0, 6] },
      h2: { fontSize: 12, bold: true, italics: true, margin: [0, 10, 0, 4] },
      h3: { fontSize: 12, italics: true, margin: [0, 8, 0, 3] },
      body: { alignment: 'justify', margin: [0, 0, 0, 8] },
      list: { lineHeight: 1.3 },
    },
    content,
  }
  const fonts = { Tinos: { normal: 'Tinos-Regular.ttf', bold: 'Tinos-Bold.ttf', italics: 'Tinos-Italic.ttf', bolditalics: 'Tinos-BoldItalic.ttf' } }
  // The browser build takes the virtual font files and the font table as arguments.
  const pdf = (pdfMake as unknown as { createPdf: (doc: unknown, layouts: unknown, fonts: unknown, vfs: unknown) => { getBuffer: (done: (buffer: Uint8Array) => void) => void } }).createPdf(definition, undefined, fonts, vfs)
  return new Promise(resolve => pdf.getBuffer(buffer => resolve(new Uint8Array(buffer))))
}

// ---------- saving ----------

function toBase64(bytes: Uint8Array) {
  let binary = ''
  for (let index = 0; index < bytes.length; index += 0x8000) binary += String.fromCharCode(...bytes.subarray(index, index + 0x8000))
  return btoa(binary)
}

const encoder = new TextEncoder()

export async function build(paper: Paper, format: ExportFormat): Promise<Uint8Array> {
  switch (format) {
    case 'docx': return toDocx(paper)
    case 'pdf': return toPdf(paper)
    case 'tex': return encoder.encode(toLatex(paper))
    case 'html': return encoder.encode(toHtml(paper))
    case 'md': return encoder.encode(toMarkdown(paper))
    case 'txt': return encoder.encode(toText(paper))
    case 'bib': return encoder.encode(await api.exportCitations(paper.references.map(reference => reference.source), 'bibtex'))
  }
}

/** Asks where to save, then writes the export there. Resolves to the path, or null when cancelled. */
export async function download(paper: Paper, format: ExportFormat): Promise<string | null> {
  const option = EXPORTS.find(item => item.format === format)!
  const { save } = await import('@tauri-apps/plugin-dialog')
  const path = await save({ defaultPath: `${fileName(paper)}.${option.extension}`, filters: [{ name: option.filter, extensions: [option.extension] }] })
  if (!path) return null
  const bytes = await build(paper, format)
  await api.saveExportFile(path, toBase64(bytes))
  return path
}

export const paperText = (paper: Paper) => toMarkdown(paper)

/** The Markdown export without its marks: heading signs, emphasis, code ticks and table rules removed. */
export function toText(paper: Paper) {
  return toMarkdown(paper).split('\n')
    .filter(line => !/^\s*\|?\s*:?-{3,}/.test(line))
    .map(line => line.replace(/^#{1,6}\s+/, '').replace(/^(\s*)[-*]\s+\[( |x)\]\s+/i, (_, pad: string, done: string) => `${pad}${done.trim() ? '☑' : '☐'} `)
      .replace(/^(\s*)[-*]\s+/, '$1• ').replace(/\*\*|__|`/g, '').replace(/(^|[^*])\*([^*\n]+)\*/g, '$1$2').replace(/\s*\|\s*/g, '  ').trim())
    .join('\n').replace(/\n{3,}/g, '\n\n')
}
export const paperTitle = (paper: Paper) => plain(inlines(paper.title))
