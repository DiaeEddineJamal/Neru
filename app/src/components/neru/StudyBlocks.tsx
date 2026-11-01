import { useState } from 'react'
import { Check, Download, RotateCcw, X } from '@/components/icons'
import { CodeBlock } from '@/components/agents/code-block'
import { cn } from '@/lib/utils'
import { saveTextFile } from '@/lib/citations'
import { cardsCsv, parseCards, parseQuiz } from '@/lib/study'

// Study helpers (/flashcards, /quiz) answer with a fenced JSON block that renders as cards or a quiz.
// While the block is still streaming, or if a model writes something else, it shows as plain code.

export function FlashcardsBlock({ code }: { code: string }) {
  const cards = parseCards(code)
  const [flipped, setFlipped] = useState<Set<number>>(new Set())
  const [note, setNote] = useState('')
  if (!cards) return <CodeBlock code={code} language="json" />
  const flip = (index: number) => setFlipped(current => { const next = new Set(current); if (next.has(index)) next.delete(index); else next.add(index); return next })
  const exportCsv = async () => {
    try { if (await saveTextFile('flashcards.csv', { name: 'CSV for Anki', extension: 'csv' }, cardsCsv(cards))) setNote('Saved. In Anki: File → Import, fields Front and Back.') }
    catch (cause) { setNote(cause instanceof Error ? cause.message : String(cause)) }
  }
  return <section className="not-prose my-3" aria-label={`${cards.length} flashcards`}>
    <div className="mb-2 flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
      <span className="mr-auto">{cards.length} flashcards · click a card to turn it</span>
      {flipped.size > 0 && <button type="button" className="inline-flex h-7 items-center gap-1 rounded-lg px-2 hover:bg-muted hover:text-foreground" onClick={() => setFlipped(new Set())}><RotateCcw size={12} aria-hidden />Reset</button>}
      <button type="button" className="inline-flex h-7 items-center gap-1 rounded-lg px-2 hover:bg-muted hover:text-foreground" onClick={() => void exportCsv()}><Download size={12} aria-hidden />Export for Anki (CSV)</button>
    </div>
    {note && <p className="mb-2 text-xs text-muted-foreground" role="status">{note}</p>}
    <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
      {cards.map((card, index) => {
        const back = flipped.has(index)
        return <button key={index} type="button" aria-pressed={back} onClick={() => flip(index)}
          className={cn('min-h-24 rounded-xl border border-border p-3 text-left text-sm leading-6 transition-colors hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring outline-none', back && 'bg-muted/40')}>
          <span className="mb-1 block text-[11px] uppercase tracking-wide text-muted-foreground">{back ? 'Answer' : `Card ${index + 1}`}</span>
          <span className="text-foreground">{back ? card.back : card.front}</span>
        </button>
      })}
    </div>
  </section>
}

export function QuizBlock({ code }: { code: string }) {
  const questions = parseQuiz(code)
  const [chosen, setChosen] = useState<Record<number, number>>({})
  const [reveal, setReveal] = useState(false)
  if (!questions) return <CodeBlock code={code} language="json" />
  const answered = Object.keys(chosen).length
  const correct = questions.filter((question, index) => chosen[index] === question.answer).length
  return <section className="not-prose my-3 flex flex-col gap-3" aria-label={`Quiz with ${questions.length} questions`}>
    {questions.map((question, index) => {
      const pick = chosen[index]
      const settled = reveal || pick !== undefined
      return <fieldset key={index} className="rounded-xl border border-border p-3">
        <legend className="px-1 text-sm font-medium text-foreground">{index + 1}. {question.question}</legend>
        <div className="mt-1 flex flex-col gap-1">
          {question.options.map((option, choice) => {
            const right = choice === question.answer
            return <button key={choice} type="button" disabled={pick !== undefined} onClick={() => setChosen(current => ({ ...current, [index]: choice }))}
              className={cn('flex items-start gap-2 rounded-lg px-2 py-1.5 text-left text-sm hover:bg-muted/60 disabled:cursor-default', settled && right && 'bg-[color-mix(in_oklab,var(--sage)_18%,transparent)]', pick === choice && !right && 'bg-destructive/10')}>
              <span className="w-5 shrink-0 font-medium text-muted-foreground">{String.fromCharCode(65 + choice)}</span>
              <span className="min-w-0 flex-1 text-foreground">{option}</span>
              {settled && right && <Check size={14} className="mt-1 shrink-0 text-[var(--sage)]" aria-label="Correct answer" />}
              {pick === choice && !right && <X size={14} className="mt-1 shrink-0 text-destructive" aria-label="Your answer, incorrect" />}
            </button>
          })}
        </div>
        {settled && question.explanation && <p className="mt-2 text-sm text-muted-foreground">{question.explanation}</p>}
      </fieldset>
    })}
    <div className="flex items-center gap-3 text-xs text-muted-foreground">
      <span role="status">{answered ? `${correct} of ${answered} correct` : `${questions.length} questions`}</span>
      <button type="button" className="h-7 rounded-lg px-2 hover:bg-muted hover:text-foreground" onClick={() => setReveal(value => !value)}>{reveal ? 'Hide answers' : 'Show all answers'}</button>
      {answered > 0 && <button type="button" className="h-7 rounded-lg px-2 hover:bg-muted hover:text-foreground" onClick={() => { setChosen({}); setReveal(false) }}>Start over</button>}
    </div>
  </section>
}
