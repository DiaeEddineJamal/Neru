import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { beforeAll, describe, expect, it, vi } from 'vitest'
import type { Paper } from '../types'
import { build, toHtml, toLatex, toMarkdown, toText } from './paperExport'

const source = (authors: string[], year: number, title: string) => ({ id: title, title, url: 'https://doi.org/10.1000/x', domain: 'doi.org', authors, year, venue: 'Journal', doi: '10.1000/x' })
const paper: Paper = {
  id: 'p', title: 'Sleep & Memory: 100% of “Students”', kind: 'Literature review', authors: ['Sam Lee'], affiliation: 'Psychology student', date: '1 November 2026',
  abstractText: 'An abstract with *emphasis*.', keywords: ['sleep', 'memory'],
  sections: [
    { heading: 'Introduction', body: 'Sleep helps (Doe & Smith, 2020).\n\n- one\n- two' },
    { heading: 'Findings', body: '### Detail\n\n| Study | N |\n|---|---|\n| Doe | 88 |' },
  ],
  references: [{ number: 1, text: 'Doe, J., & Smith, A. (2020). Title. *Journal*.', source: source(['Jane Doe', 'Alan Smith'], 2020, 'Title') }],
  style: 'apa', format: 'docx', language: 'English', question: 'q', words: 12, created: 0, status: 'done', outline: [], cited: [1], note: '',
}

describe('text exports', () => {
  it('Markdown numbers sections and lists references', () => {
    const text = toMarkdown(paper)
    expect(text).toContain('# Sleep & Memory')
    expect(text).toContain('## 1. Introduction')
    expect(text).toContain('## 2. Findings')
    expect(text).toContain('## References\n\nDoe, J., & Smith, A. (2020).')
  })
  it('HTML escapes text and renders tables and hanging references', () => {
    const html = toHtml(paper)
    expect(html).toContain('<title>Sleep &amp; Memory: 100% of “Students”</title>')
    expect(html).toContain('<table><thead><tr><th>Study</th><th>N</th></tr></thead>')
    expect(html).toContain('<em>Journal</em>')
    expect(html).toContain('text-indent: -2.2em')
    expect(html).not.toContain('<script')
  })
  it('LaTeX escapes special characters and builds sections', () => {
    const tex = toLatex(paper)
    expect(tex).toContain('\\title{Sleep \\& Memory: 100\\% of ``Students\'\'}')
    expect(tex).toContain('\\section{Introduction}')
    expect(tex).toContain('\\begin{itemize}')
    expect(tex).toContain('\\begin{tabularx}')
    expect(tex).toContain('\\emph{Journal}')
    expect(tex.trim().endsWith('\\end{document}')).toBe(true)
  })
  it('IEEE references keep their numbers without a hanging indent', () => {
    expect(toHtml({ ...paper, style: 'ieee' })).not.toContain('text-indent: -2.2em')
  })
})

describe('binary exports', () => {
  beforeAll(() => {
    // The PDF fonts are fetched from the app bundle; in tests they come from disk.
    vi.stubGlobal('fetch', async (url: string) => {
      const name = decodeURIComponent(String(url)).split('/').pop()!.split('?')[0]
      return new Response(readFileSync(join(__dirname, '..', 'assets', 'fonts', 'tinos', name)))
    })
  })
  it('Word export is a valid .docx package', async () => {
    const bytes = await build(paper, 'docx')
    expect([bytes[0], bytes[1]]).toEqual([0x50, 0x4b])
    expect(bytes.length).toBeGreaterThan(5_000)
  }, 30_000)
  it('PDF export starts with a PDF header and embeds the serif font', async () => {
    const bytes = await build(paper, 'pdf')
    const text = new TextDecoder('latin1').decode(bytes)
    expect(text.startsWith('%PDF-')).toBe(true)
    expect(text).toContain('Tinos')
  }, 60_000)
})

describe('plain documents', () => {
  const plan: Paper = { ...paper, title: 'Launch plan', kind: 'Plan', authors: [], affiliation: '', abstractText: '', keywords: [], references: [], plain: true,
    sections: [{ heading: '', body: '## Goals\n\n- [ ] Ship **v1**\n- [x] Hire\n\n| Week | Task |\n|---|---|\n| 1 | Build |' }] }
  it('keep their own headings, unnumbered, in Markdown and HTML', () => {
    expect(toMarkdown(plan)).toContain('## Goals')
    expect(toMarkdown(plan)).not.toContain('## 1.')
    expect(toHtml(plan)).not.toContain('<h2>1. ')
  })
  it('export as plain text without formatting marks', () => {
    const text = toText(plan)
    expect(text).toContain('Launch plan')
    expect(text).toContain('Goals')
    expect(text).toContain('☐ Ship v1')
    expect(text).toContain('☑ Hire')
    expect(text).not.toMatch(/[#*|]/)
  })
})
