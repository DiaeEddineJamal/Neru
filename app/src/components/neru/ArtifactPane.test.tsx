import { act, fireEvent, render, screen, waitFor } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Paper } from '../../types'
import type { LiveDraft } from '@/lib/artifactStore'
import { ArtifactPane } from './ArtifactPane'

const doc: Paper = {
  id: 'd1', title: 'Launch plan', kind: 'Plan', authors: [], affiliation: '', date: '', abstractText: '', keywords: [], references: [], style: 'apa', format: '', language: 'English',
  question: '', words: 20, created: 0, status: 'done', outline: [], cited: [], note: '', plain: true,
  sections: [{ heading: '', body: '## Goals\n\nShip the first version in June.\n\nHire two engineers.' }],
}
let live: LiveDraft | null = null
const updatePaperPart = vi.fn((_session: string, _id: string, part: string, text: string) => Promise.resolve({ ...doc, sections: [{ heading: '', body: part === '0' ? text : doc.sections[0].body }] }))

vi.mock('../../api', () => ({ api: { readPaper: vi.fn(() => Promise.resolve(doc)), listPapers: vi.fn(() => Promise.resolve([])), updatePaperPart: (...args: [string, string, string, string]) => updatePaperPart(...args) } }))
vi.mock('@/lib/artifactStore', () => ({ closePaper: vi.fn(), openPaper: vi.fn(), useLiveDraft: () => live, useSaves: () => 0 }))

beforeEach(() => { live = null; updatePaperPart.mockClear() })

describe('ArtifactPane', () => {
  it('edits one paragraph in place and saves the part back together', async () => {
    render(<ArtifactPane sessionId="s" id="d1" onError={vi.fn()} onNotice={vi.fn()} />)
    const paragraph = await screen.findByText('Ship the first version in June.')
    fireEvent.click(paragraph)
    const box = await screen.findByRole('textbox', { name: 'Edit paragraph' })
    expect((box as HTMLTextAreaElement).value).toBe('Ship the first version in June.')
    fireEvent.change(box, { target: { value: 'Ship the first version in May.' } })
    await act(async () => { fireEvent.blur(box) })
    expect(updatePaperPart).toHaveBeenCalledWith('s', 'd1', '0', '## Goals\n\nShip the first version in May.\n\nHire two engineers.')
    await waitFor(() => expect(screen.getByText('Ship the first version in May.')).toBeTruthy())
    expect(screen.getByText('Hire two engineers.')).toBeTruthy()
  })

  it('puts back a paragraph on Escape without saving', async () => {
    render(<ArtifactPane sessionId="s" id="d1" onError={vi.fn()} onNotice={vi.fn()} />)
    fireEvent.click(await screen.findByText('Hire two engineers.'))
    const box = await screen.findByRole('textbox', { name: 'Edit paragraph' })
    fireEvent.change(box, { target: { value: 'Something else' } })
    fireEvent.keyDown(box, { key: 'Escape' })
    expect(updatePaperPart).not.toHaveBeenCalled()
    expect(screen.getByText('Hire two engineers.')).toBeTruthy()
  })

  it('shows a passage being rewritten between the text kept before and after it', async () => {
    live = { sessionId: 's', id: 'd1', index: 0, heading: '', text: 'Ship it in May', before: '## Goals', after: 'Hire two engineers.' }
    const { container } = render(<ArtifactPane sessionId="s" id="d1" onError={vi.fn()} onNotice={vi.fn()} />)
    await screen.findByText('Ship it in May')
    expect(container.querySelector('.is-writing-part')?.textContent).toBe('Ship it in May')
    expect(screen.getByText('Hire two engineers.')).toBeTruthy()
    expect(screen.queryByRole('textbox')).toBeNull()
  })
})
