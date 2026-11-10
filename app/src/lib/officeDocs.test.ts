import { describe, expect, it } from 'vitest'
import { unzipSync, strFromU8 } from 'fflate'
import { column, display, parseSheets, parseSlides, slideParts, toCsv, toPptx, toXlsx } from './officeDocs'
import { blocks } from './paperBlocks'

const DECK = `## Launch plan
Q3 review for the board

## Where we are
- Revenue up 12%
  - Mostly from renewals
- Churn down

Notes: Pause here for questions.

## Growth
- Two new markets
\`\`\`chart
{"type":"bar","labels":["Q1","Q2"],"series":[{"name":"Sales","data":[3,5]}]}
\`\`\`
`

const BOOK = `## Budget
| Item | Qty | Price (USD) | Total |
|---|---|---|---|
| Tent | 2 | 120 | =B2*C2 |
| Stove | 1 | 45.5 | =B3*C3 |
| Total | | | =SUM(D2:D3) |

## Notes: plan
| Day | Place |
|---|---|
| 2026-07-01 | Lakes |
`

describe('slides', () => {
  it('splits a deck at its ## lines and keeps notes apart', () => {
    const slides = parseSlides(DECK, 'Deck')
    expect(slides.map(slide => slide.title)).toEqual(['Launch plan', 'Where we are', 'Growth'])
    expect(slides[0].body).toBe('Q3 review for the board')
    expect(slides[1].notes).toBe('Pause here for questions.')
    expect(slides[1].body).not.toContain('Notes')
    const { text, charts } = slideParts(slides[2].body)
    expect(charts).toHaveLength(1)
    expect(text).toBe('- Two new markets')
  })

  it('builds a pptx with one slide per section', async () => {
    const bytes = await toPptx(DECK, 'Deck')
    const files = unzipSync(bytes)
    const slides = Object.keys(files).filter(name => /^ppt\/slides\/slide\d+\.xml$/.test(name))
    expect(slides).toHaveLength(3)
    expect(Object.keys(files).some(name => name.startsWith('ppt/charts/'))).toBe(true)
    expect(strFromU8(files['ppt/slides/slide2.xml'])).toContain('Revenue up 12%')
  }, 20_000)
})

describe('sheets', () => {
  it('parses sheets and works out formulas', () => {
    const [budget, plan] = parseSheets(BOOK)
    expect(budget.name).toBe('Budget')
    expect(plan.name).toBe('Notes plan')
    expect(budget.rows[0][3].value).toBe(240)
    expect(budget.rows[1][3].value).toBe(45.5)
    expect(budget.rows[2][3].value).toBe(285.5)
    expect(display(budget.rows[2][3])).toBe((285.5).toLocaleString(undefined, { maximumFractionDigits: 4 }))
    expect(plan.rows[0][0].value).toBe('2026-07-01')
  })

  it('leaves unsupported formulas showing as written', () => {
    const [sheet] = parseSheets('| A | B |\n|---|---|\n| 1 | =VLOOKUP(A2,A2:A3,1) |')
    expect(sheet.rows[0][1].error).toBe(true)
    expect(display(sheet.rows[0][1])).toBe('=VLOOKUP(A2,A2:A3,1)')
  })

  it('names columns like Excel', () => {
    expect([column(0), column(25), column(26), column(701)]).toEqual(['A', 'Z', 'AA', 'ZZ'])
  })

  it('builds an xlsx with formulas and cached values', async () => {
    const files = unzipSync(await toXlsx(parseSheets(BOOK), 'Trip'))
    const sheet = strFromU8(files['xl/worksheets/sheet1.xml'])
    expect(sheet).toContain('<f>SUM(D2:D3)</f><v>285.5</v>')
    expect(strFromU8(files['xl/workbook.xml'])).toContain('name="Notes plan"')
  })

  it('writes CSV with quoting', () => {
    const [sheet] = parseSheets('| Name | Note |\n|---|---|\n| A, B | say "hi" |')
    expect(toCsv(sheet)).toBe('Name,Note\r\n"A, B","say ""hi"""\r\n')
  })
})

describe('document exports', () => {
  it('turn a chart fence into its data table and drop diagrams', () => {
    const out = blocks('Intro\n\n```chart\n{"type":"line","title":"Users","labels":["Jan","Feb"],"series":[{"name":"2026","data":[10,20]}]}\n```\n\n```mermaid\nflowchart LR\n  A --> B\n```\n\nEnd')
    expect(out.map(block => block.kind)).toEqual(['paragraph', 'paragraph', 'table', 'paragraph'])
    const table = out[2]
    expect(table.kind === 'table' && table.rows.length).toBe(2)
  })
})
