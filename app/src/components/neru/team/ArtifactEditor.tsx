import { useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react'
import { Bold, Code, Columns2, Eye, Heading, Italic, Link, List, ListChecks, ListOrdered, Minus, PenLine, Quote, Redo2, SquareCode, Table, Undo2 } from 'lucide-react'
import { cn } from '@/lib/utils'
import { continueList, cycleHeading, indentList, insertRule, insertTable, splitFrontMatter, toggleCodeBlock, toggleLinePrefix, toggleLink, toggleWrap, type EditState } from '@/lib/markdownEdit'
import './ArtifactEditor.css'

export * from '@/lib/markdownEdit'

type Mode = 'write' | 'preview' | 'split'
type Props = { value: string; onChange: (value: string) => void; onSave: () => void; onCancel: () => void; renderPreview: (markdown: string) => ReactNode }

const MODES: { mode: Mode; label: string; icon: typeof Eye }[] = [
  { mode: 'write', label: 'Write', icon: PenLine },
  { mode: 'split', label: 'Split', icon: Columns2 },
  { mode: 'preview', label: 'Preview', icon: Eye },
]

export function ArtifactEditor({ value, onChange, onSave, onCancel, renderPreview }: Props) {
  const [front, body] = splitFrontMatter(value)
  const [mode, setMode] = useState<Mode>('write')
  const textarea = useRef<HTMLTextAreaElement>(null)
  const initial = useRef(value)
  // Our own history: programmatic edits bypass the textarea's native undo stack.
  const history = useRef({ stack: [{ text: body, start: 0, end: 0 }] as EditState[], index: 0, typedAt: 0 })
  const pending = useRef<[number, number] | null>(null)

  useLayoutEffect(() => {
    const element = textarea.current
    if (!pending.current || !element) return
    element.focus()
    element.setSelectionRange(...pending.current)
    pending.current = null
  })

  const current = (): EditState => ({ text: body, start: textarea.current?.selectionStart ?? body.length, end: textarea.current?.selectionEnd ?? body.length })
  const record = (state: EditState, typing: boolean) => {
    const log = history.current
    const now = Date.now()
    log.stack = log.stack.slice(0, log.index + 1)
    // Coalesce bursts of typing into one undo step.
    if (typing && now - log.typedAt < 800 && log.index > 0) log.stack[log.index] = state
    else { log.stack.push(state); log.index = log.stack.length - 1 }
    log.typedAt = typing ? now : 0
  }
  const apply = (state: EditState | null) => {
    if (!state) return false
    record(state, false)
    pending.current = [state.start, state.end]
    onChange(front + state.text)
    return true
  }
  const edit = (transform: (state: EditState) => EditState | null) => apply(transform(current()))
  const travel = (step: number) => {
    const log = history.current
    const index = log.index + step
    if (index < 0 || index >= log.stack.length) return
    log.index = index
    log.typedAt = 0
    const state = log.stack[index]
    pending.current = [state.start, state.end]
    onChange(front + state.text)
  }
  const cancel = () => { if (value === initial.current || window.confirm('Discard your changes to this artifact?')) onCancel() }

  const onKeyDown = (event: KeyboardEvent) => {
    const mod = event.ctrlKey || event.metaKey
    const key = event.key.toLowerCase()
    if (mod && key === 's') { event.preventDefault(); onSave(); return }
    if (event.key === 'Escape') { event.preventDefault(); cancel(); return }
    if (event.target !== textarea.current || event.nativeEvent.isComposing) return
    const run = (transform: (state: EditState) => EditState | null) => { if (edit(transform)) event.preventDefault() }
    if (mod && !event.altKey) {
      if (key === 'z') { event.preventDefault(); travel(event.shiftKey ? 1 : -1) }
      else if (key === 'y') { event.preventDefault(); travel(1) }
      else if (key === 'b') run(state => toggleWrap(state, '**'))
      else if (key === 'i') run(state => toggleWrap(state, '*'))
      else if (key === 'e') run(state => toggleWrap(state, '`', 'code'))
      else if (key === 'k') run(toggleLink)
    } else if (event.key === 'Enter' && !event.shiftKey && !event.altKey) run(continueList)
    else if (event.key === 'Tab' && !event.altKey) run(state => indentList(state, event.shiftKey))
  }

  const tools: { label: string; keys?: string; icon: typeof Bold; run: (state: EditState) => EditState | null }[][] = [
    [
      { label: 'Bold', keys: 'Ctrl+B', icon: Bold, run: state => toggleWrap(state, '**') },
      { label: 'Italic', keys: 'Ctrl+I', icon: Italic, run: state => toggleWrap(state, '*') },
      { label: 'Heading', icon: Heading, run: cycleHeading },
    ],
    [
      { label: 'Bulleted list', icon: List, run: state => toggleLinePrefix(state, 'ul') },
      { label: 'Numbered list', icon: ListOrdered, run: state => toggleLinePrefix(state, 'ol') },
      { label: 'Checklist', icon: ListChecks, run: state => toggleLinePrefix(state, 'task') },
      { label: 'Quote', icon: Quote, run: state => toggleLinePrefix(state, 'quote') },
    ],
    [
      { label: 'Inline code', keys: 'Ctrl+E', icon: Code, run: state => toggleWrap(state, '`', 'code') },
      { label: 'Code block', icon: SquareCode, run: toggleCodeBlock },
      { label: 'Link', keys: 'Ctrl+K', icon: Link, run: toggleLink },
      { label: 'Table', icon: Table, run: insertTable },
      { label: 'Horizontal rule', icon: Minus, run: insertRule },
    ],
  ]
  const { stack, index } = history.current
  const writing = mode !== 'preview'

  return <div className={cn('artifact-editor', `mode-${mode}`)} onKeyDown={onKeyDown}>
    <div className="artifact-editor-toolbar" role="toolbar" aria-label="Formatting">
      {tools.map((group, at) => <div key={at} className="artifact-editor-group">
        {group.map(tool => <button key={tool.label} type="button" className="icon-button small" disabled={!writing}
          title={tool.keys ? `${tool.label} (${tool.keys})` : tool.label} aria-label={tool.label} aria-keyshortcuts={tool.keys?.replace('Ctrl', 'Control')}
          onMouseDown={event => event.preventDefault()} onClick={() => edit(tool.run)}><tool.icon size={14} /></button>)}
      </div>)}
      <div className="artifact-editor-group">
        <button type="button" className="icon-button small" disabled={!writing || index === 0} title="Undo (Ctrl+Z)" aria-label="Undo" onMouseDown={event => event.preventDefault()} onClick={() => travel(-1)}><Undo2 size={14} /></button>
        <button type="button" className="icon-button small" disabled={!writing || index >= stack.length - 1} title="Redo (Ctrl+Shift+Z)" aria-label="Redo" onMouseDown={event => event.preventDefault()} onClick={() => travel(1)}><Redo2 size={14} /></button>
      </div>
      <div className="theme-toggle artifact-editor-modes" role="group" aria-label="View">
        {MODES.map(item => <button key={item.mode} type="button" className={mode === item.mode ? 'active' : ''} aria-pressed={mode === item.mode} title={item.label} onClick={() => setMode(item.mode)}><item.icon size={13} /><span>{item.label}</span></button>)}
      </div>
    </div>
    <div className="artifact-editor-panes">
      {writing && <textarea ref={textarea} className="artifact-editor-input" value={body} spellCheck={false} aria-label="Markdown"
        onChange={event => { const element = event.target; record({ text: element.value, start: element.selectionStart, end: element.selectionEnd }, true); onChange(front + element.value) }} />}
      {mode !== 'write' && <div className="artifact-editor-preview">{renderPreview(body)}</div>}
    </div>
  </div>
}
