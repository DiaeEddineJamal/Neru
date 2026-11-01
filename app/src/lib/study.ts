import type { SlashCommand } from '../types'

/**
 * Study helpers for students: slash commands that work on the attached documents or, when nothing is
 * attached, on Neru's previous answer. Flashcards and quizzes come back as fenced JSON blocks that the
 * conversation renders as cards and a quiz (components/neru/StudyBlocks.tsx).
 */
export const STUDY_COMMANDS: SlashCommand[] = [
  { name: 'summarize', description: 'Summarize the attached file or the last answer: /summarize undergrad', template: '$ARGUMENTS', source: 'built-in', argumentHint: '[high school | undergrad | grad]' },
  { name: 'flashcards', description: 'Make flashcards to study, exportable to Anki', template: '$ARGUMENTS', source: 'built-in', argumentHint: '[how many or which topic]' },
  { name: 'quiz', description: 'Quiz me with multiple-choice questions and answers', template: '$ARGUMENTS', source: 'built-in', argumentHint: '[how many or which topic]' },
  { name: 'explain', description: 'Explain a concept, the attached file or the last answer step by step', template: '$ARGUMENTS', source: 'built-in', argumentHint: '[concept]' },
]

const LEVELS: [RegExp, string][] = [
  [/\b(high ?school|hs|secondary|beginner|simple)\b/i, 'a high-school student'],
  [/\b(grad(uate)?|master'?s?|phd|doctoral|advanced|expert)\b/i, 'a graduate student'],
  [/\b(undergrad(uate)?|college|university|bachelor'?s?)\b/i, 'an undergraduate'],
]

/** "graduate" from "/summarize grad", else empty (Neru then uses what it knows about the user). */
export function studyLevel(args: string): string {
  return LEVELS.find(([pattern]) => pattern.test(args))?.[1] ?? ''
}

export function studyPrompt(command: string, args: string, attached: string[]): string {
  const material = attached.length ? `the attached ${attached.length === 1 ? 'document' : 'documents'} (${attached.join(', ')})` : 'your previous answer in this conversation'
  const level = studyLevel(args)
  const reader = level || 'the student (match their level and field if you know them; otherwise an undergraduate)'
  const focus = args && !level ? ` Focus: ${args}.` : ''
  switch (command) {
    case 'summarize':
      return `Summarize ${material} for ${reader}. Start with a one-paragraph overview, then the key points as bullets, the important terms with one-line definitions, and finish with what to remember for an exam. Keep the source's structure where it helps, and do not add facts that are not in it.${focus}`
    case 'flashcards': {
      const count = /\b(\d{1,2})\b/.exec(args)?.[1] ?? '12'
      return `Make ${count} flashcards from ${material} for ${reader}.${focus} Each card tests one fact or concept: a precise question or term on the front, a short complete answer on the back. Reply with one sentence of introduction, then a fenced code block whose language is flashcards containing only a JSON array like [{"front": "…", "back": "…"}].`
    }
    case 'quiz': {
      const count = /\b(\d{1,2})\b/.exec(args)?.[1] ?? '8'
      return `Write a ${count}-question multiple-choice quiz on ${material} for ${reader}.${focus} Each question has four options with exactly one correct answer, plausible distractors, and a one- or two-sentence explanation of why the answer is right. Mix recall and application questions. Reply with one sentence of introduction, then a fenced code block whose language is quiz containing only a JSON array like [{"question": "…", "options": ["…", "…", "…", "…"], "answer": 0, "explanation": "…"}], where answer is the index of the correct option.`
    }
    default:
      return args && !level && !attached.length
        ? `Explain ${args} to ${reader}, step by step: start from what they likely already know, give an intuitive analogy, a worked example, the common misconceptions, and end with one question they can use to check their understanding.`
        : `Explain ${material} to ${reader}, step by step: start from what they likely already know, give an intuitive analogy, a worked example, the common misconceptions, and end with one question they can use to check their understanding.${focus}`
  }
}

// Flashcard and quiz blocks: fenced JSON from /flashcards and /quiz.

export interface Card { front: string; back: string }
export interface Question { question: string; options: string[]; answer: number; explanation?: string }

function parseJson(code: string): unknown {
  try { return JSON.parse(code) } catch { return null }
}

export function parseCards(code: string): Card[] | null {
  const data = parseJson(code)
  if (!Array.isArray(data)) return null
  const cards = data.flatMap(item => item && typeof item === 'object' && typeof item.front === 'string' && typeof item.back === 'string' ? [{ front: item.front.trim(), back: item.back.trim() }] : [])
  return cards.length ? cards : null
}

export function parseQuiz(code: string): Question[] | null {
  const data = parseJson(code)
  if (!Array.isArray(data)) return null
  const questions = data.flatMap(item => {
    if (!item || typeof item !== 'object' || typeof item.question !== 'string' || !Array.isArray(item.options)) return []
    const options = item.options.filter((option: unknown): option is string => typeof option === 'string')
    // The answer may come as an index, a letter, or the option's text.
    const raw = item.answer
    const answer = typeof raw === 'number' ? raw : typeof raw === 'string' && /^[A-Ha-h]$/.test(raw.trim()) ? raw.trim().toUpperCase().charCodeAt(0) - 65 : options.indexOf(String(raw))
    return options.length >= 2 && answer >= 0 && answer < options.length ? [{ question: item.question, options, answer, explanation: typeof item.explanation === 'string' ? item.explanation : undefined }] : []
  })
  return questions.length ? questions : null
}

/** Cards as Anki imports them: two quoted columns, front and back, no header. */
export const cardsCsv = (cards: Card[]) => cards.map(card => [card.front, card.back].map(text => `"${text.replace(/"/g, '""')}"`).join(',')).join('\r\n') + '\r\n'
