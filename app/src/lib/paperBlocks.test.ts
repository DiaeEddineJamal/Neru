import { describe, expect, it } from 'vitest'
import { blocks, inlines, plain } from './paperBlocks'

describe('inlines', () => {
  it('splits bold, italic, code and links', () => {
    expect(inlines('A **bold** and *italic* with `code` and [a link](https://x.org).')).toEqual([
      { text: 'A ' }, { text: 'bold', bold: true }, { text: ' and ' }, { text: 'italic', italic: true }, { text: ' with ' }, { text: 'code', code: true }, { text: ' and ' }, { text: 'a link' }, { text: '.' },
    ])
  })
  it('leaves snake_case words and lone stars alone', () => {
    expect(plain(inlines('a snake_case_name * 2'))).toBe('a snake_case_name * 2')
    expect(inlines('a snake_case_name').every(run => !run.italic)).toBe(true)
  })
  it('reads citations as plain text', () => {
    expect(inlines('(Doe & Smith, 2020)')).toEqual([{ text: '(Doe & Smith, 2020)' }])
  })
})

describe('blocks', () => {
  it('parses paragraphs, headings, lists, quotes and tables', () => {
    const parsed = blocks('First line\nsame paragraph.\n\n### Sub\n\n- one\n- two\n\n1. first\n2. second\n\n> quoted\n\n| A | B |\n|---|---|\n| 1 | 2 |\n| 3 | 4 |\n\nEnd.')
    expect(parsed.map(block => block.kind)).toEqual(['paragraph', 'heading', 'list', 'list', 'quote', 'table', 'paragraph'])
    const first = parsed[0]
    expect(first.kind === 'paragraph' && plain(first.inlines)).toBe('First line same paragraph.')
    const table = parsed[5]
    if (table.kind !== 'table') throw new Error('expected a table')
    expect(table.header.map(plain)).toEqual(['A', 'B'])
    expect(table.rows.map(row => row.map(plain))).toEqual([['1', '2'], ['3', '4']])
    const numbered = parsed[3]
    expect(numbered.kind === 'list' && numbered.ordered).toBe(true)
  })
  it('keeps wrapped list lines with their item', () => {
    const [list] = blocks('- a long item\n  that wraps\n- next')
    expect(list.kind === 'list' && list.items.map(plain)).toEqual(['a long item that wraps', 'next'])
  })
  it('handles Windows line endings and empty input', () => {
    expect(blocks('One\r\n\r\nTwo')).toHaveLength(2)
    expect(blocks('')).toEqual([])
  })
})
