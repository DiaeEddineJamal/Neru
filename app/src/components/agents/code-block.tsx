import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { Check, Copy, FileCode2, LoaderCircle } from 'lucide-react'
import { useReducedMotion } from 'motion/react'
import { AgentCodeLine, type AgentCodeLanguage, type AgentCodeToken, useAgentCodeTokens } from './agent-code'
import { cn } from '@/lib/utils'

export interface CodeBlockProps {
  code: string
  language: AgentCodeLanguage
  /** File path shown in the header; the language name is shown when absent. */
  title?: string
  /** Still being generated: shows a spinner and keeps the newest line in view. */
  streaming?: boolean
  maxHeight?: number
  /** Line number of the first line shown (for a tail of a longer file). */
  startLine?: number
  /** Leave out the header, when a surrounding card already names the file. */
  bare?: boolean
  className?: string
}

const LANGUAGE_NAMES: Record<string, string> = {
  bash: 'Shell', cpp: 'C++', csharp: 'C#', css: 'CSS', html: 'HTML', javascript: 'JavaScript',
  json: 'JSON', jsx: 'JavaScript JSX', markdown: 'Markdown', powershell: 'PowerShell', python: 'Python',
  rust: 'Rust', scss: 'SCSS', sql: 'SQL', text: 'Plain text', tsx: 'TypeScript JSX', typescript: 'TypeScript',
  yaml: 'YAML',
}

/** Tokens from an earlier, shorter version of streaming code only fit lines that have not grown since. */
const current = (tokens: AgentCodeToken[] | undefined, line: string) =>
  tokens && tokens.reduce((length, token) => length + token.content.length, 0) === line.length ? tokens : undefined

/** Code in VS Code's Dark+ / Light+ colors, with line numbers, a copy button and live streaming. */
export function CodeBlock({ code, language, title, streaming = false, maxHeight = 420, startLine = 1, bare = false, className }: CodeBlockProps) {
  const reduce = useReducedMotion() ?? false
  const tokens = useAgentCodeTokens(code, language)
  const scroller = useRef<HTMLDivElement>(null)
  const pinned = useRef(true)
  const [copied, setCopied] = useState(false)
  const lines = code.replace(/\n$/, '').split('\n')

  // While streaming, follow the newest line unless the reader scrolled up to look at something.
  useLayoutEffect(() => {
    const node = scroller.current
    if (streaming && node && pinned.current) node.scrollTop = node.scrollHeight
  }, [code, streaming])
  useEffect(() => { if (!copied) return; const timer = window.setTimeout(() => setCopied(false), 1400); return () => window.clearTimeout(timer) }, [copied])

  const copy = () => { void navigator.clipboard.writeText(code).then(() => setCopied(true)).catch(() => undefined) }
  const name = LANGUAGE_NAMES[language] ?? language
  return <figure className={cn('code-block', streaming && 'is-streaming', className)} data-language={language}>
    {!bare && <figcaption className="code-block-head">
      <span className="code-block-title">
        {streaming ? <LoaderCircle size={13} className={cn(!reduce && 'animate-spin')} aria-hidden /> : <FileCode2 size={13} aria-hidden />}
        <span className="truncate" title={title}>{title ?? name}</span>
      </span>
      <span className="code-block-meta">
        {title && <span>{name}</span>}
        <span>{lines.length} {lines.length === 1 ? 'line' : 'lines'}</span>
        {!streaming && <button type="button" className="code-block-copy" onClick={copy} aria-label={copied ? 'Copied' : 'Copy code'} title={copied ? 'Copied' : 'Copy'}>{copied ? <Check size={13} /> : <Copy size={13} />}</button>}
      </span>
    </figcaption>}
    <div ref={scroller} className="code-block-scroll" style={{ maxHeight }} onScroll={event => { const node = event.currentTarget; pinned.current = node.scrollHeight - node.scrollTop - node.clientHeight < 24 }}>
      <pre className="code-block-pre"><code>
        {lines.map((line, index) => <span className="code-block-line" key={index}>
          <span className="code-block-gutter" aria-hidden>{index + startLine}</span>
          <AgentCodeLine code={line || ' '} tokens={line ? current(tokens?.[index], line) : undefined} className="code-block-text" />
          {'\n'}
        </span>)}
      </code></pre>
    </div>
  </figure>
}
