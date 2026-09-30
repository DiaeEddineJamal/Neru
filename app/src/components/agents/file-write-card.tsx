import { memo, useState } from 'react'
import { Check, ChevronDown, CircleAlert, FilePen, FilePlus2, LoaderCircle, ShieldCheck } from 'lucide-react'
import { useReducedMotion } from 'motion/react'
import { AgentDisclosure } from './agent-disclosure'
import { CodeBlock } from './code-block'
import { languageForPath } from './agent-code'
import { cn } from '@/lib/utils'

export type FileWriteState = 'writing' | 'done' | 'pending' | 'error'

export interface FileWriteCardProps {
  path: string
  content: string
  state: FileWriteState
  /** An edit (snippet replacement) rather than a whole file. */
  edit?: boolean
}

// Lines of the newest code kept on screen while a file is being written.
const TAIL = 10

/**
 * One file the agent writes, shown the way Claude Code shows a Write/Edit call: a compact row that
 * names the file and counts lines, with the newest lines streaming underneath while it is written.
 * The whole file is one click away; only the visible part is highlighted, so long files stay smooth.
 */
function FileWriteCardImpl({ path, content, state, edit = false }: FileWriteCardProps) {
  const reduce = useReducedMotion() ?? false
  const [open, setOpen] = useState(false)
  const writing = state === 'writing'
  const lines = content.replace(/\n$/, '').split('\n')
  const count = content ? lines.length : 0
  const name = path.split(/[\/]/).pop() || path || 'file'
  const verb = edit ? (writing ? 'Editing' : 'Edited') : writing ? 'Writing' : state === 'pending' ? 'Proposed' : 'Wrote'
  const icon = writing ? <LoaderCircle size={13} className={cn(!reduce && 'animate-spin')} />
    : state === 'error' ? <CircleAlert size={13} className="text-destructive" />
    : state === 'pending' ? <ShieldCheck size={13} className="text-amber-500" />
    : <Check size={13} className="text-[var(--sage)]" />
  const tailStart = Math.max(0, lines.length - TAIL)
  return <div className={cn('file-write', writing && 'is-writing')} data-state={state}>
    <button type="button" className="file-write-row" aria-expanded={open} onClick={() => setOpen(value => !value)} title={path}>
      <span className="file-write-icon" aria-hidden>{icon}</span>
      {edit ? <FilePen size={13} className="file-write-kind" aria-hidden /> : <FilePlus2 size={13} className="file-write-kind" aria-hidden />}
      <span className="file-write-verb">{verb}</span>
      <span className="file-write-path truncate">{path || name}</span>
      <span className="file-write-count">{count} {count === 1 ? 'line' : 'lines'}{writing && content.length > 2048 ? ` · ${(content.length / 1024).toFixed(1)} KB` : ''}</span>
      <ChevronDown size={13} className={cn('file-write-chevron', open && 'open')} aria-hidden />
    </button>
    {writing && !open && content && <CodeBlock bare streaming code={lines.slice(tailStart).join('\n')} startLine={tailStart + 1} language={languageForPath(path)} maxHeight={TAIL * 19 + 20} className="file-write-tail" />}
    <AgentDisclosure open={open}>
      {open && <CodeBlock code={content} title={path} language={languageForPath(path)} streaming={writing} maxHeight={440} className="file-write-full" />}
    </AgentDisclosure>
  </div>
}

export const FileWriteCard = memo(FileWriteCardImpl)
