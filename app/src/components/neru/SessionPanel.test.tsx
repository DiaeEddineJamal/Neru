import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import type { ContextUsage } from '../../types'
import { SessionPanel, tokenLabel } from './SessionPanel'

vi.mock('../../api', () => ({
  api: {
    mcpServers: vi.fn(() => Promise.resolve([{ name: 'github', command: 'npx', args: [], env: {}, headers: {}, enabled: true, status: 'connected', signedIn: true, error: null, tools: [{ name: 'create_issue', description: 'Open an issue', readOnly: false }] }])),
    memoryFiles: vi.fn(() => Promise.resolve({ project: 'D:/work/app/NERU.md', user: 'C:/Users/me/.neru/NERU.md' })),
    projectMemoryFacts: vi.fn(() => Promise.resolve(['Tests run with npm test'])),
    listSkills: vi.fn(() => Promise.resolve([])),
    outputStyles: vi.fn(() => Promise.resolve([{ name: 'Default', description: '', source: 'built-in', current: true }])),
    projectTrustStatus: vi.fn(() => Promise.resolve({ trusted: true, mcpServers: [], hookEvents: ['post_edit'] })),
    openMemoryFile: vi.fn(() => Promise.resolve('')),
    setOutputStyle: vi.fn(() => Promise.resolve([])),
  },
}))

const context: ContextUsage = { used: 24_000, window: 128_000, limit: 64_000, measured: true, model: 'm', breakdown: [{ kind: 'system', tokens: 6_000 }, { kind: 'tools', tokens: 3_000 }, { kind: 'messages', tokens: 5_000 }, { kind: 'results', tokens: 10_000 }] }

describe('SessionPanel', () => {
  it('breaks the context down and compacts on request', () => {
    const onCompact = vi.fn()
    render(<SessionPanel context={context} todos={[]} canCompact onCompact={onCompact} onOpenSettings={vi.fn()} onError={vi.fn()} />)
    expect(screen.getByText('of 64k tokens · 38%')).toBeTruthy()
    expect(screen.getByText('Tool results')).toBeTruthy()
    expect(screen.getByText('Free space').parentElement?.textContent).toContain('40k')
    fireEvent.click(screen.getByRole('button', { name: 'Compact now' }))
    expect(onCompact).toHaveBeenCalled()
  })

  it('lists MCP servers with their tools, memory files and tasks', async () => {
    render(<SessionPanel context={null} todos={[{ content: 'Write tests', status: 'in_progress' }, { content: 'Fix bug', status: 'completed' }]} canCompact={false} onCompact={vi.fn()} onOpenSettings={vi.fn()} onError={vi.fn()} />)
    expect(screen.getByText('1/2')).toBeTruthy()
    await waitFor(() => expect(screen.getByText('github')).toBeTruthy())
    fireEvent.click(screen.getByText('github'))
    expect(screen.getByText('create_issue')).toBeTruthy()
    expect(screen.getByText('app/NERU.md')).toBeTruthy()
    expect(screen.getByText('Tests run with npm test')).toBeTruthy()
  })

  it('formats token counts compactly', () => {
    expect(tokenLabel(950)).toBe('950')
    expect(tokenLabel(64_000)).toBe('64k')
    expect(tokenLabel(1_048_576)).toBe('1M')
  })
})
