// Previews in the document panel for the files create_document makes besides Word and PDF: a slide deck
// (PowerPoint) shown as 16:9 slides, and a workbook (Excel) shown as a grid with sheet tabs. Charts and
// diagrams inside documents and slides are drawn with the chat's own renderers (Visual.tsx).
import { isValidElement, useMemo, useState, type ReactNode } from 'react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { t } from '../../i18n'
import { cn } from '@/lib/utils'
import { isVisualTag, remarkVisualFences } from '@/lib/visual/fence'
import { column, display, parseSheets, parseSlides } from '@/lib/officeDocs'
import { VisualFence } from './Visual'

/** Fenced blocks in a document: charts, diagrams and widgets are drawn; other code stays code. */
export function DocFence({ children }: { children?: ReactNode }) {
  const child = Array.isArray(children) ? children[0] : children
  if (!isValidElement(child)) return <pre>{children}</pre>
  const props = child.props as { className?: string; children?: ReactNode; 'data-open'?: string }
  const tag = /language-([\w+#.-]+)/.exec(props.className ?? '')?.[1]
  if (!isVisualTag(tag)) return <pre>{children}</pre>
  return <div className="doc-visual"><VisualFence tag={tag!} code={String(props.children ?? '').replace(/\n$/, '')} open={props['data-open'] === 'true'} /></div>
}

export const docPlugins = [remarkGfm, remarkVisualFences]
const link = ({ children }: { children?: ReactNode }) => <span>{children}</span>
const slideComponents = { pre: DocFence, a: link }

/** A deck: each `## ` section as a 16:9 slide, the first one as the title slide; speaker notes under each. */
export function SlideDeck({ markdown, title, writing }: { markdown: string; title: string; writing?: boolean }) {
  const slides = useMemo(() => parseSlides(markdown, title), [markdown, title])
  if (!slides.length) return <div className="deck-empty">{writing ? t('Writing…') : t('No slides yet.')}</div>
  return <div className="deck">
    {slides.map((slide, index) => <figure key={index} className={cn('deck-item', writing && index === slides.length - 1 && 'is-writing')}>
      <div className={cn('deck-slide', index === 0 && 'is-title')} aria-label={t('Slide {n}', { n: index + 1 })}>
        <h2>{slide.title}</h2>
        {slide.body && <div className="deck-body"><ReactMarkdown remarkPlugins={docPlugins} components={slideComponents}>{slide.body}</ReactMarkdown></div>}
        <span className="deck-number">{index + 1}</span>
      </div>
      {slide.notes && <figcaption className="deck-notes"><strong>{t('Notes')}</strong> {slide.notes}</figcaption>}
    </figure>)}
  </div>
}

/** A workbook: sheet tabs over a grid with column letters and row numbers; formulas show their results. */
export function SheetView({ markdown, writing }: { markdown: string; writing?: boolean }) {
  const sheets = useMemo(() => parseSheets(markdown), [markdown])
  const [active, setActive] = useState(0)
  const sheet = sheets[Math.min(active, sheets.length - 1)]
  if (!sheet) return <div className="deck-empty">{writing ? t('Writing…') : t('No sheets yet.')}</div>
  const numeric = sheet.header.map((_, col) => sheet.rows.some(row => typeof row[col]?.value === 'number'))
  return <div className="sheet">
    {sheets.length > 1 && <div className="sheet-tabs" role="tablist" aria-label={t('Sheets')}>
      {sheets.map((item, index) => <button key={item.name} type="button" role="tab" aria-selected={item === sheet} className={cn(item === sheet && 'active')} onClick={() => setActive(index)}>{item.name}</button>)}
    </div>}
    <div className="sheet-grid" role="region" aria-label={sheet.name} tabIndex={0}>
      <table>
        <thead>
          <tr><th className="sheet-corner" aria-hidden />{sheet.header.map((_, col) => <th key={col} className="sheet-letter" aria-hidden>{column(col)}</th>)}</tr>
        </thead>
        <tbody>
          <tr className="sheet-head"><th className="sheet-rownum">1</th>{sheet.header.map((head, col) => <th key={col} scope="col">{head}</th>)}</tr>
          {sheet.rows.map((row, at) => <tr key={at} className={cn(/^(total|sum|grand total|totals)\b/i.test(row[0]?.raw ?? '') && 'sheet-total')}>
            <th className="sheet-rownum">{at + 2}</th>
            {row.map((cell, col) => <td key={col} className={cn((typeof cell.value === 'number' || (numeric[col] && cell.formula)) && 'is-number', cell.formula && 'is-formula', cell.error && 'is-error')} title={cell.formula ? `=${cell.formula}` : undefined}>{display(cell)}</td>)}
          </tr>)}
        </tbody>
      </table>
    </div>
    {sheet.charts.map((code, index) => <div key={index} className="doc-visual sheet-chart"><VisualFence tag="chart" code={code} /></div>)}
    <p className="sheet-hint">{t('{rows} rows · {cols} columns. Formulas are kept and recalculate when the file opens in Excel.', { rows: sheet.rows.length, cols: sheet.header.length })}</p>
  </div>
}
