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
  return ({
    ts: 'typescript', tsx: 'typescript', mts: 'typescript', cts: 'typescript', js: 'javascript', jsx: 'javascript', mjs: 'javascript', cjs: 'javascript',
    rs: 'rust', css: 'css', scss: 'scss', less: 'less', html: 'html', htm: 'html', vue: 'html', svelte: 'html', json: 'json', jsonc: 'json', md: 'markdown', mdx: 'markdown',
    py: 'python', sh: 'shell', bash: 'shell', zsh: 'shell', ps1: 'powershell', psm1: 'powershell', bat: 'bat', cmd: 'bat', toml: 'ini', ini: 'ini',
    yaml: 'yaml', yml: 'yaml', xml: 'xml', svg: 'xml', sql: 'sql', go: 'go', java: 'java', kt: 'kotlin', c: 'c', h: 'c', cpp: 'cpp', cc: 'cpp', hpp: 'cpp',
    cs: 'csharp', php: 'php', rb: 'ruby', swift: 'swift', dart: 'dart', lua: 'lua', r: 'r', graphql: 'graphql', gql: 'graphql', dockerfile: 'dockerfile',
  } as Record<string, string>)[extension ?? ''] ?? (path.toLowerCase().endsWith('dockerfile') ? 'dockerfile' : 'plaintext')
}

export default function CodeEditor({ path, value, onChange, light }: { path: string; value: string; onChange: (value: string) => void; light: boolean }) {
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
  return <Editor height="100%" path={path} language={languageFor(path)} value={value} onChange={next => onChange(next ?? '')} theme={light ? 'vs' : 'neru-dark'} options={{ minimap: { enabled: false }, fontFamily: "Consolas, 'JetBrains Mono', 'Courier New', monospace", fontSize: 13, lineNumbersMinChars: 3, scrollBeyondLastLine: false, padding: { top: 18 }, automaticLayout: true, tabSize: 2, renderValidationDecorations: 'on', bracketPairColorization: { enabled: true }, guides: { indentation: true } }} />
}
