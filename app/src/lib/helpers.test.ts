import { describe, expect, it } from 'vitest'
import { SHORTCUTS, actionFor, comboOf, conflictOf, defaultBindings } from './shortcuts'
import { cardsCsv, parseCards, parseQuiz } from './study'
import { describeError } from './friendlyError'
import { paperIdIn } from './artifactStore'
import { findSecrets, redact } from '../redact'

describe('shortcuts', () => {
  it('reads key combinations, with Cmd as Ctrl', () => {
    expect(comboOf({ key: 'k', ctrlKey: true, metaKey: false, altKey: false, shiftKey: false })).toBe('Ctrl+K')
    expect(comboOf({ key: 'k', ctrlKey: false, metaKey: true, altKey: false, shiftKey: false })).toBe('Ctrl+K')
    expect(comboOf({ key: 'Shift', ctrlKey: false, metaKey: false, altKey: false, shiftKey: true })).toBeNull()
  })
  it('maps default bindings to actions and flags conflicts', () => {
    const bindings = defaultBindings()
    expect(actionFor(bindings, new KeyboardEvent('keydown', { key: 'k', ctrlKey: true }))).toBe('palette')
    expect(SHORTCUTS.every(item => conflictOf(bindings, item.id) === null)).toBe(true)
    expect(conflictOf({ ...bindings, compare: 'Ctrl+K' }, 'compare')).toMatch(/Also bound/)
    expect(conflictOf({ ...bindings, compare: 'Ctrl+C' }, 'compare')).toMatch(/Overrides Copy/)
  })
})

describe('study helpers', () => {
  it('parses flashcards and exports Anki CSV', () => {
    const cards = parseCards('[{"front":"Q \\"1\\"","back":"A"},{"bad":1}]')
    expect(cards).toEqual([{ front: 'Q "1"', back: 'A' }])
    expect(cardsCsv(cards!)).toBe('"Q ""1""","A"\r\n')
    expect(parseCards('nope')).toBeNull()
  })
  it('accepts quiz answers as index, letter or text', () => {
    const quiz = parseQuiz(JSON.stringify([
      { question: 'a', options: ['x', 'y'], answer: 1 },
      { question: 'b', options: ['x', 'y'], answer: 'B' },
      { question: 'c', options: ['x', 'y'], answer: 'x' },
      { question: 'd', options: ['x'], answer: 0 },
    ]))
    expect(quiz?.map(item => item.answer)).toEqual([1, 1, 0])
  })
})

describe('errors, secrets and paper cards', () => {
  it('turns provider failures into plain advice', () => {
    expect(describeError('HTTP 429 Too Many Requests').action).toBe('retry')
    expect(describeError(new Error('HTTP 401 invalid api key')).title).toMatch(/rejected your API key/)
    expect(describeError('maximum context length exceeded').action).toBe('compact')
  })
  it('finds and removes secrets', () => {
    const text = `key nvapi-${'a'.repeat(40)} here`
    expect(findSecrets(text).map(item => item.kind)).toContain('NVIDIA key')
    expect(redact(text).text).not.toContain('nvapi-aaaa')
    expect(redact('nothing here').secrets).toEqual([])
  })
  it('finds the paper card in a reply, and ignores broken ones', () => {
    expect(paperIdIn('Done.\n\n```neru-document\n{"id":"abc-1","title":"T"}\n```')).toBe('abc-1')
    expect(paperIdIn('No card here')).toBeNull()
    expect(paperIdIn('```neru-document\nnot json\n```')).toBeNull()
  })
})
