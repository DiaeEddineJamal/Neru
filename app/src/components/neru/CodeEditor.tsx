import { useEffect } from 'react'
import Editor, { loader } from '@monaco-editor/react'
import * as monaco from 'monaco-editor'
import EditorWorker from 'monaco-editor/editor/editor.worker.js?worker'
import CssWorker from 'monaco-editor/language/css/css.worker.js?worker'
import HtmlWorker from 'monaco-editor/language/html/html.worker.js?worker'
import JsonWorker from 'monaco-editor/language/json/json.worker.js?worker'
import TsWorker from 'monaco-editor/language/typescript/ts.worker.js?worker'

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
  return ({ ts: 'typescript', tsx: 'typescript', js: 'javascript', jsx: 'javascript', rs: 'rust', css: 'css', html: 'html', json: 'json', md: 'markdown', py: 'python', sh: 'shell', toml: 'ini' } as Record<string, string>)[extension ?? ''] ?? 'plaintext'
}

export default function CodeEditor({ path, value, onChange, light }: { path: string; value: string; onChange: (value: string) => void; light: boolean }) {
  useEffect(() => {
    const diagnostics = { noSemanticValidation: false, noSyntaxValidation: false, noSuggestionDiagnostics: false }
    monaco.typescript.typescriptDefaults.setDiagnosticsOptions(diagnostics)
    monaco.typescript.javascriptDefaults.setDiagnosticsOptions(diagnostics)
    monaco.typescript.typescriptDefaults.setEagerModelSync(true)
    monaco.editor.defineTheme('neru-dark', {
      base: 'vs-dark', inherit: true,
      rules: [{ token: 'comment', foreground: '777A72' }, { token: 'string', foreground: 'A2B69E' }],
      colors: { 'editor.background': '#111310', 'editor.foreground': '#EAE9E2', 'editorLineNumber.foreground': '#6C7068', 'editorCursor.foreground': '#8EA291', 'editor.selectionBackground': '#344A39' },
    })
  }, [])
  return <Editor height="100%" path={path} language={languageFor(path)} value={value} onChange={next => onChange(next ?? '')} theme={light ? 'vs' : 'neru-dark'} options={{ minimap: { enabled: false }, fontFamily: 'JetBrains Mono', fontSize: 12, lineNumbersMinChars: 3, scrollBeyondLastLine: false, padding: { top: 18 }, automaticLayout: true, tabSize: 2, renderValidationDecorations: 'on' }} />
}
