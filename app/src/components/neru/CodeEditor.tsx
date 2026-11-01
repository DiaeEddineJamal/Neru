import { useEffect, useRef, useState } from 'react'
import Editor, { loader, type OnMount } from '@monaco-editor/react'
import * as monaco from 'monaco-editor'
import EditorWorker from 'monaco-editor/editor/editor.worker.js?worker'
import CssWorker from 'monaco-editor/language/css/css.worker.js?worker'
import HtmlWorker from 'monaco-editor/language/html/html.worker.js?worker'
import JsonWorker from 'monaco-editor/language/json/json.worker.js?worker'
import TsWorker from 'monaco-editor/language/typescript/ts.worker.js?worker'
import { Check, LoaderCircle, Sparkles, X } from 'lucide-react'
import { api } from '../../api'
import { t } from '../../i18n'
import { describeError } from '../../lib/friendlyError'

type WorkerEnvironment = { getWorker: (_moduleId: string, label: string) => Worker }
;(self as unknown as { MonacoEnvironment: WorkerEnvironment }).MonacoEnvironment = {
  getWorker(_moduleId, label) {
    if (label === 'json') return new JsonWorker()
    if (label === 'css' || label === 'scss' || label === 'less') return new CssWorker()
    if (label === 'html' || label === 'handlebars' || label === 'razor') return new HtmlWorker()
    if (label === 'typescript' || label === 'javascript') return new TsWorker()
    return new EditorWorker()
  },
}
loader.config({ monaco })

const languageFor = (path: string) => {
  const extension = path.split('.').pop()?.toLowerCase()
  return ({
    ts: 'typescript', tsx: 'typescript', mts: 'typescript', cts: 'typescript', js: 'javascript', jsx: 'javascript', mjs: 'javascript', cjs: 'javascript',
    rs: 'rust', css: 'css', scss: 'scss', less: 'less', html: 'html', htm: 'html', vue: 'html', svelte: 'html', json: 'json', jsonc: 'json', md: 'markdown', mdx: 'markdown',
    py: 'python', sh: 'shell', bash: 'shell', zsh: 'shell', ps1: 'powershell', psm1: 'powershell', bat: 'bat', cmd: 'bat', toml: 'ini', ini: 'ini',
    yaml: 'yaml', yml: 'yaml', xml: 'xml', svg: 'xml', sql: 'sql', go: 'go', java: 'java', kt: 'kotlin', c: 'c', h: 'c', cpp: 'cpp', cc: 'cpp', hpp: 'cpp',
    cs: 'csharp', php: 'php', rb: 'ruby', swift: 'swift', dart: 'dart', lua: 'lua', r: 'r', graphql: 'graphql', gql: 'graphql', dockerfile: 'dockerfile',
  } as Record<string, string>)[extension ?? ''] ?? (path.toLowerCase().endsWith('dockerfile') ? 'dockerfile' : 'plaintext')
}

type Editor = monaco.editor.IStandaloneCodeEditor
/** The Ctrl+K box: where it sits, what it covers, and the proposed rewrite once it is in. */
interface InlineEdit {
  top: number
  left: number
  range: monaco.Range
  original: string
  busy: boolean
  error: string
  /** Set once the rewrite is in the editor, shown as a diff until it is accepted or rejected. */
  applied?: { range: monaco.Range; decorations: monaco.editor.IEditorDecorationsCollection; zone: string }
}

const sleep = (ms: number) => new Promise(resolve => window.setTimeout(resolve, ms))
/** How long typing must pause before Neru asks for a completion. */
const COMPLETE_DELAY = 450

export default function CodeEditor({ path, value, onChange, light, complete = true }: { path: string; value: string; onChange: (value: string) => void; light: boolean; complete?: boolean }) {
  const editorRef = useRef<Editor | null>(null)
  const pathRef = useRef(path)
  const completeRef = useRef(complete)
  const [inline, setInline] = useState<InlineEdit | null>(null)
  const [instruction, setInstruction] = useState('')
  const inlineRef = useRef<InlineEdit | null>(null)
  const promptRef = useRef<HTMLInputElement>(null)
  const acceptRef = useRef<HTMLButtonElement>(null)
  useEffect(() => { pathRef.current = path }, [path])
  useEffect(() => { completeRef.current = complete }, [complete])
  useEffect(() => { inlineRef.current = inline }, [inline])

  useEffect(() => {
    const diagnostics = { noSemanticValidation: false, noSyntaxValidation: false, noSuggestionDiagnostics: false }
    monaco.typescript.typescriptDefaults.setDiagnosticsOptions(diagnostics)
    monaco.typescript.javascriptDefaults.setDiagnosticsOptions(diagnostics)
    monaco.typescript.typescriptDefaults.setEagerModelSync(true)
    // VS Code's Dark+ token colors as they are; only the canvas is tinted to sit in Neru's window.
    monaco.editor.defineTheme('neru-dark', {
      base: 'vs-dark', inherit: true, rules: [],
      colors: { 'editor.background': '#1E1E1E', 'editorCursor.foreground': '#AEAFAD' },
    })
  }, [])

  // Tab completion: ghost text from inline_complete once typing pauses.
  useEffect(() => {
    let asked = 0
    const provider = monaco.languages.registerInlineCompletionsProvider({ pattern: '**' }, {
      provideInlineCompletions: async (model, position, _context, token) => {
        if (!completeRef.current || inlineRef.current || model !== editorRef.current?.getModel()) return { items: [] }
        const ticket = ++asked
        await sleep(COMPLETE_DELAY)
        if (token.isCancellationRequested || ticket !== asked) return { items: [] }
        const end = model.getFullModelRange().getEndPosition()
        const prefix = model.getValueInRange(new monaco.Range(1, 1, position.lineNumber, position.column))
        const suffix = model.getValueInRange(new monaco.Range(position.lineNumber, position.column, end.lineNumber, end.column))
        // Nothing to add in the middle of a word.
        if (/\w/.test(suffix.charAt(0))) return { items: [] }
        try {
          const text = await api.inlineComplete(prefix, suffix, pathRef.current)
          if (token.isCancellationRequested || !text.trim()) return { items: [] }
          return { items: [{ insertText: text, range: new monaco.Range(position.lineNumber, position.column, position.lineNumber, position.column) }] }
        } catch {
          return { items: [] }
        }
      },
      disposeInlineCompletions: () => {},
    })
    return () => provider.dispose()
  }, [])

  const closeInline = () => { setInline(null); setInstruction(''); editorRef.current?.focus() }

  const clearDiff = (edit: InlineEdit) => {
    if (!edit.applied) return
    edit.applied.decorations.clear()
    const zone = edit.applied.zone
    editorRef.current?.changeViewZones(accessor => accessor.removeZone(zone))
  }

  const accept = () => { const edit = inlineRef.current; if (edit) clearDiff(edit); closeInline() }

  const reject = () => {
    const edit = inlineRef.current
    const editor = editorRef.current
    if (edit?.applied && editor) {
      clearDiff(edit)
      editor.executeEdits('neru-inline-edit', [{ range: edit.applied.range, text: edit.original, forceMoveMarkers: true }])
      editor.pushUndoStop()
    }
    closeInline()
  }

  const openInline = (editor: Editor) => {
    if (inlineRef.current?.applied) return
    const selection = editor.getSelection()
    const model = editor.getModel()
    if (!selection || !model) return
    // An empty selection asks for code at the cursor.
    const range = new monaco.Range(selection.startLineNumber, selection.isEmpty() ? selection.startColumn : 1, selection.endLineNumber, selection.isEmpty() ? selection.endColumn : (selection.endColumn === 1 && selection.endLineNumber > selection.startLineNumber ? 1 : model.getLineMaxColumn(selection.endLineNumber)))
    const spot = editor.getScrolledVisiblePosition({ lineNumber: range.startLineNumber, column: 1 })
    const layout = editor.getLayoutInfo()
    setInline({ top: Math.max(4, (spot?.top ?? 0) - 44), left: layout.contentLeft, range, original: model.getValueInRange(range), busy: false, error: '' })
    setInstruction('')
    window.setTimeout(() => promptRef.current?.focus(), 0)
  }

  const submit = async () => {
    const edit = inlineRef.current
    const editor = editorRef.current
    const model = editor?.getModel()
    if (!edit || !editor || !model || !instruction.trim() || edit.busy) return
    setInline({ ...edit, busy: true, error: '' })
    const end = model.getFullModelRange().getEndPosition()
    const before = model.getValueInRange(new monaco.Range(1, 1, edit.range.startLineNumber, edit.range.startColumn))
    const after = model.getValueInRange(new monaco.Range(edit.range.endLineNumber, edit.range.endColumn, end.lineNumber, end.column))
    try {
      let replacement = await api.inlineEdit(pathRef.current, edit.original, instruction.trim(), before, after)
      // Keep the selection's trailing newline when whole lines were rewritten.
      if (edit.original.endsWith('\n') && !replacement.endsWith('\n')) replacement += '\n'
      if (inlineRef.current?.range !== edit.range) return
      editor.pushUndoStop()
      editor.executeEdits('neru-inline-edit', [{ range: edit.range, text: replacement, forceMoveMarkers: true }])
      editor.pushUndoStop()
      const lines = replacement.split('\n')
      const endLine = edit.range.startLineNumber + lines.length - 1
      const endColumn = lines.length === 1 ? edit.range.startColumn + lines[0].length : lines[lines.length - 1].length + 1
      const range = new monaco.Range(edit.range.startLineNumber, edit.range.startColumn, endLine, endColumn)
      const decorations = editor.createDecorationsCollection([{ range, options: { className: 'neru-inline-added', isWholeLine: true, linesDecorationsClassName: 'neru-inline-added-gutter' } }])
      // The replaced code, struck through above the new code.
      let zone = ''
      const removed = edit.original.replace(/\n$/, '')
      if (removed) {
        const node = document.createElement('div')
        node.className = 'neru-inline-removed'
        node.textContent = removed
        editor.changeViewZones(accessor => { zone = accessor.addZone({ afterLineNumber: edit.range.startLineNumber - 1, heightInLines: removed.split('\n').length, domNode: node }) })
      }
      setInline({ ...edit, busy: false, applied: { range, decorations, zone } })
      window.setTimeout(() => acceptRef.current?.focus(), 0)
    } catch (cause) {
      setInline({ ...edit, busy: false, error: t(describeError(cause).title) })
    }
  }

  const mount: OnMount = editor => {
    editorRef.current = editor
    editor.addAction({
      id: 'neru.inlineEdit',
      label: t('Edit with Neru'),
      keybindings: [monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyK],
      contextMenuGroupId: 'navigation',
      contextMenuOrder: 0,
      run: () => openInline(editor),
    })
  }

  return <div className="code-editor">
    <Editor height="100%" path={path} language={languageFor(path)} value={value} onChange={next => onChange(next ?? '')} onMount={mount} theme={light ? 'vs' : 'neru-dark'} options={{ minimap: { enabled: false }, fontFamily: "Consolas, 'JetBrains Mono', 'Courier New', monospace", fontSize: 13, lineNumbersMinChars: 3, scrollBeyondLastLine: false, padding: { top: 18 }, automaticLayout: true, tabSize: 2, renderValidationDecorations: 'on', bracketPairColorization: { enabled: true }, guides: { indentation: true }, inlineSuggest: { enabled: complete } }} />
    {inline && <div className="inline-edit" style={{ top: inline.top, left: inline.left }} role="dialog" aria-label={t('Edit with Neru')}>
      {inline.applied
        ? <div className="inline-edit-row">
          <span className="inline-edit-note"><Sparkles size={13} /> {t('Keep this change?')}</span>
          <button ref={acceptRef} type="button" className="button primary small" onClick={accept} onKeyDown={event => { if (event.key === 'Escape') reject() }}><Check size={13} /> {t('Accept')}</button>
          <button type="button" className="button subtle small" onClick={reject}><X size={13} /> {t('Reject')}</button>
        </div>
        : <form className="inline-edit-row" onSubmit={event => { event.preventDefault(); void submit() }}>
          {inline.busy ? <LoaderCircle size={14} className="animate-spin" aria-label={t('Editing…')} /> : <Sparkles size={14} />}
          <input ref={promptRef} value={instruction} disabled={inline.busy} placeholder={inline.original ? t('Change the selection…') : t('Write code here…')} aria-label={t('What to change')} onChange={event => setInstruction(event.target.value)} onKeyDown={event => { if (event.key === 'Escape') closeInline() }} />
          <button type="submit" className="button primary small" disabled={inline.busy || !instruction.trim()}>{inline.busy ? t('Editing…') : t('Edit')}</button>
          <button type="button" className="icon-button small" aria-label={t('Close')} onClick={closeInline}><X size={13} /></button>
        </form>}
      {inline.error && <p className="inline-edit-error">{inline.error}</p>}
    </div>}
  </div>
}
