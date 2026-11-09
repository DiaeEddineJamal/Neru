import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { act, fireEvent, render, screen } from '@testing-library/react'
import ReactMarkdown from 'react-markdown'
import { isValidElement, type ReactNode } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { fenceIsClosed, remarkVisualFences } from '@/lib/visual/fence'
import { niceTicks, parseChart } from '@/lib/visual/chart'
import { WIDGET_SANDBOX_URL } from '@/lib/visual/widget'
import { VisualFence } from './Visual'

vi.mock('./Diagrams', async () => {
  const actual = await vi.importActual<typeof import('./Diagrams')>('./Diagrams')
  return { ...actual, MermaidBlock: ({ code }: { code: string }) => <div data-testid="mermaid">{code}</div> }
})

const BAR = JSON.stringify({ type: 'bar', title: 'Revenue', labels: ['Q1', 'Q2', 'Q3'], series: [{ name: '2025', data: [3, 5, 4] }, { name: '2026', data: [4, 6, 7] }], x: 'Quarter', y: 'USD (k)' })

/** Renders markdown the way the conversation should: plugin plus a `pre` hoisted out of render, so
 * react-markdown keeps the same component type between tokens and memoized visuals stay put. */
const COMPONENTS = {
  pre: ({ children }: { children?: ReactNode }) => {
    const child = (Array.isArray(children) ? children[0] : children) as ReactNode
    if (!isValidElement(child)) return <pre>{children}</pre>
    const props = child.props as { className?: string; children?: ReactNode; 'data-open'?: string }
    const tag = /language-([\w+#.-]+)/.exec(props.className ?? '')?.[1] ?? ''
    return <VisualFence tag={tag} code={String(props.children ?? '').replace(/\n$/, '')} open={props['data-open'] === 'true'} />
  },
}
const PLUGINS = [remarkVisualFences]
function Markdown({ content }: { content: string }) {
  return <ReactMarkdown remarkPlugins={PLUGINS} components={COMPONENTS}>{content}</ReactMarkdown>
}

describe('chart spec', () => {
  it('parses a valid spec and fills missing labels', () => {
    const result = parseChart('{"type":"line","series":[{"name":"a","data":[1,2,3]}]}')
    expect(result.ok && result.spec.labels).toEqual(['1', '2', '3'])
  })
  it('rejects malformed JSON, unknown types and non-numbers', () => {
    expect(parseChart('{"type":"bar", series: [}').ok).toBe(false)
    expect(parseChart('{"type":"radar","series":[1]}').ok).toBe(false)
    expect(parseChart('{"series":[{"name":"a","data":[1,"x"]}]}').ok).toBe(false)
  })
  it('makes round ticks', () => {
    expect(niceTicks(0, 7)).toEqual([0, 2, 4, 6, 8])
  })
})

describe('fences', () => {
  it('tells closed from still-streaming fences', () => {
    expect(fenceIsClosed('```chart\n{}\n```')).toBe(true)
    expect(fenceIsClosed('```chart\n{"type":')).toBe(false)
    expect(fenceIsClosed('````chart\n{}\n```')).toBe(false)
  })
})

describe('VisualFence', () => {
  it('draws a valid chart with a legend that toggles series and a hover tooltip', () => {
    const { container } = render(<VisualFence tag="chart" code={BAR} />)
    expect(screen.getByText('Revenue')).toBeTruthy()
    expect(container.querySelectorAll('rect.visual-mark')).toHaveLength(6)
    const legend = screen.getByRole('button', { name: /2026/ })
    fireEvent.click(legend)
    expect(legend.getAttribute('aria-pressed')).toBe('false')
    expect(container.querySelectorAll('rect.visual-mark')).toHaveLength(3)
    fireEvent.pointerEnter(container.querySelectorAll('rect.visual-hit')[1])
    expect(screen.getByRole('tooltip').textContent).toContain('Q2')
    expect(screen.getByRole('tooltip').textContent).toContain('5')
  })

  it('falls back to the code with a note for malformed JSON', () => {
    render(<VisualFence tag="chart" code={'{"type":"bar","series":[1,2,'} />)
    const fallback = screen.getByTestId('visual-fallback')
    expect(fallback.textContent).toContain('Could not draw this chart')
    expect(fallback.textContent).toContain('"type":"bar"')
  })

  it('shows a placeholder while the fence is still streaming, then draws once it closes', () => {
    const partial = 'Here it is:\n\n```chart\n{"type":"bar","labels":["Q1"'
    const { rerender, container } = render(<Markdown content={partial} />)
    expect(screen.getByTestId('visual-pending').textContent).toContain('Drawing chart')
    expect(screen.queryByTestId('visual-fallback')).toBeNull()
    rerender(<Markdown content={`Here it is:\n\n\`\`\`chart\n${BAR}\n\`\`\`\n`} />)
    expect(screen.queryByTestId('visual-pending')).toBeNull()
    expect(container.querySelectorAll('rect.visual-mark')).toHaveLength(6)
  })

  it('does not redraw a finished chart while later text streams in', () => {
    const { container, rerender } = render(<Markdown content={`\`\`\`chart\n${BAR}\n\`\`\`\n\nMore`} />)
    const first = container.querySelector('svg')
    rerender(<Markdown content={`\`\`\`chart\n${BAR}\n\`\`\`\n\nMore text arriving`} />)
    expect(container.querySelector('svg')).toBe(first)
  })

  it('keeps a render failure inside the block', () => {
    vi.spyOn(console, 'error').mockImplementation(() => {})
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    // A spec that parses but breaks while drawing must not take the message down.
    const original = Object.getOwnPropertyDescriptor(Intl.NumberFormat.prototype, 'format')!
    Object.defineProperty(Intl.NumberFormat.prototype, 'format', { configurable: true, get: () => () => { throw new Error('boom') } })
    try {
      render(<div><p>before</p><VisualFence tag="chart" code={'{"type":"pie","labels":["a","b"],"series":[{"name":"s","data":[1,2]}]}'} /></div>)
    } finally { Object.defineProperty(Intl.NumberFormat.prototype, 'format', original) }
    expect(screen.getByText('before')).toBeTruthy()
    expect(screen.getByTestId('visual-fallback').textContent).toContain('Could not draw this visual')
    vi.restoreAllMocks()
  })

  it('runs a widget in a locked-down sandbox and sizes it from its own messages only', () => {
    const { container } = render(<VisualFence tag="html-widget" code={'<button>Hi</button><script>1</script>'} />)
    const frame = container.querySelector('iframe')!
    expect(frame.getAttribute('sandbox')).toBe('allow-scripts')
    expect(frame.getAttribute('sandbox')).not.toContain('allow-same-origin')
    expect(frame.getAttribute('src')).toBe(WIDGET_SANDBOX_URL)
    expect(frame.hasAttribute('srcdoc')).toBe(false)

    const post = vi.spyOn(frame.contentWindow!, 'postMessage')
    const send = (data: unknown, source: MessageEventSource | null, origin = 'null') => act(() => { window.dispatchEvent(new MessageEvent('message', { data, source, origin })) })
    send({ source: 'neru-viz', type: 'height', height: 333 }, window) // not from the frame
    send({ source: 'neru-viz', type: 'height', height: 333 }, frame.contentWindow, 'http://localhost') // not opaque
    expect(frame.style.height).toBe('160px')
    send({ source: 'neru-viz', type: 'ready' }, frame.contentWindow)
    expect(post).toHaveBeenCalledWith(expect.objectContaining({ type: 'render', html: '<button>Hi</button><script>1</script>' }), '*')
    send({ source: 'neru-viz', type: 'height', height: 99999 }, frame.contentWindow)
    expect(frame.style.height).toBe('1600px')

    fireEvent.click(screen.getByRole('button', { name: 'Show source' }))
    expect(container.querySelector('.visual-source')?.textContent).toContain('<button>Hi</button>')
  })

  it('stops a widget that navigates its frame away', () => {
    const { container } = render(<VisualFence tag="visual" code="<p>x</p>" />)
    const frame = container.querySelector('iframe')!
    fireEvent.load(frame)
    fireEvent.load(frame)
    expect(screen.getByTestId('visual-fallback').textContent).toContain('leave its sandbox')
  })

  it('refuses widgets over 200 KB', () => {
    render(<VisualFence tag="html-widget" code={'x'.repeat(201 * 1024)} />)
    expect(screen.getByTestId('visual-fallback').textContent).toContain('too large')
    expect(document.querySelector('iframe')).toBeNull()
  })

  it('the sandbox page carries a strict CSP with no network', () => {
    const html = readFileSync(join(process.cwd(), 'public', 'viz-sandbox.html'), 'utf8')
    const csp = /http-equiv="Content-Security-Policy" content="([^"]+)"/.exec(html)?.[1] ?? ''
    expect(csp).toContain("default-src 'none'")
    expect(csp).toContain("script-src 'self' blob:")
    expect(csp).not.toMatch(/connect-src|https?:/)
    expect(csp).toContain("img-src data:")
  })

  it('keeps mermaid behind the same placeholder while streaming', () => {
    render(<Markdown content={'```mermaid\ngraph TD; A-->'} />)
    expect(screen.getByTestId('visual-pending').textContent).toContain('Drawing diagram')
    expect(screen.queryByTestId('mermaid')).toBeNull()
  })
})
