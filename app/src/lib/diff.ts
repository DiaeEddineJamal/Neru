import type { FileDiffLine } from '@/components/agents/file-diff'

/** Turns a unified diff into FileDiff rows with old/new line numbers. */
export function parseUnifiedDiff(diff: string): FileDiffLine[] {
  const lines: FileDiffLine[] = []
  let oldLine = 0
  let newLine = 0
  const rows = diff.split('\n')
  rows.forEach((raw, index) => {
    if (raw.startsWith('diff --git') || raw.startsWith('index ') || raw.startsWith('--- ') || raw.startsWith('+++ ') || raw.startsWith('\\ ') || raw.startsWith('new file mode') || raw.startsWith('deleted file mode')) return
    const hunk = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@(.*)$/.exec(raw)
    if (hunk) {
      oldLine = Number(hunk[1])
      newLine = Number(hunk[2])
      lines.push({ id: `h${index}`, content: raw })
      return
    }
    if (raw.startsWith('+')) lines.push({ id: `l${index}`, type: 'added', newLine: newLine++, content: raw.slice(1) })
    else if (raw.startsWith('-')) lines.push({ id: `l${index}`, type: 'removed', oldLine: oldLine++, content: raw.slice(1) })
    else if (raw.startsWith(' ') || (raw === '' && index < rows.length - 1 && (oldLine || newLine))) lines.push({ id: `l${index}`, type: 'context', oldLine: oldLine++, newLine: newLine++, content: raw.slice(1) })
  })
  return lines
}

export function diffCounts(lines: FileDiffLine[]) {
  return lines.reduce((counts, line) => {
    if (line.type === 'added') counts.added += 1
    if (line.type === 'removed') counts.removed += 1
    return counts
  }, { added: 0, removed: 0 })
}
