// Self-check for the artifact editor's text transforms: node --experimental-strip-types ArtifactEditor.check.ts
import { continueList, cycleHeading, indentList, insertTable, splitFrontMatter, toggleCodeBlock, toggleLinePrefix, toggleLink, toggleWrap, type EditState } from '../../../lib/markdownEdit.ts'

let failed = 0
function eq(name: string, actual: unknown, expected: unknown) {
  const a = JSON.stringify(actual), e = JSON.stringify(expected)
  if (a === e) console.log(`ok   ${name}`)
  else { failed++; console.log(`FAIL ${name}\n     got      ${a}\n     expected ${e}`) }
}
const at = (text: string, start: number, end = start): EditState => ({ text, start, end })
const sel = (text: string, part: string) => at(text, text.indexOf(part), text.indexOf(part) + part.length)

eq('bold wraps', toggleWrap(sel('a word b', 'word'), '**'), at('a **word** b', 4, 8))
eq('bold unwraps around selection', toggleWrap(at('a **word** b', 4, 8), '**'), sel('a word b', 'word'))
eq('bold unwraps selected markers', toggleWrap(sel('a **word** b', '**word**'), '**'), sel('a word b', 'word'))
eq('italic inside bold adds, not unwraps', toggleWrap(at('**w**', 2, 3), '*').text, '***w***')
eq('italic toggles back off', toggleWrap(at('***w***', 3, 4), '*'), at('**w**', 2, 3))
eq('empty selection inserts placeholder', toggleWrap(at('', 0), '`', 'code'), at('`code`', 1, 5))
eq('bullets every line', toggleLinePrefix(at('a\nb', 0, 3), 'ul').text, '- a\n- b')
eq('bullets toggle off', toggleLinePrefix(at('- a\n- b', 0, 7), 'ul').text, 'a\nb')
eq('numbers replace bullets', toggleLinePrefix(at('- a\n- b', 0, 7), 'ol').text, '1. a\n2. b')
eq('checklist', toggleLinePrefix(at('a', 1), 'task'), at('- [ ] a', 7))
eq('heading cycles H1', cycleHeading(at('Title', 0)).text, '# Title')
eq('heading cycles H3 to none', cycleHeading(at('### Title', 9)), at('Title', 5))
eq('enter continues bullet', continueList(at('- one', 5)), at('- one\n- ', 8))
eq('enter increments number', continueList(at('  9. one', 8))?.text, '  9. one\n  10. ')
eq('enter continues checklist unchecked', continueList(at('- [x] done', 10))?.text, '- [x] done\n- [ ] ')
eq('enter on empty item ends list', continueList(at('- one\n- ', 8)), at('- one\n', 6))
eq('enter outside list is native', continueList(at('plain', 5)), null)
eq('tab indents list line', indentList(at('- a', 3), false), at('  - a', 5))
eq('shift+tab outdents', indentList(at('  - a', 5), true), at('- a', 3))
eq('tab outside list is native', indentList(at('plain', 0), false), null)
eq('link selects url', toggleLink(sel('see docs', 'docs')), at('see [docs](url)', 11, 14))
eq('link unwraps', toggleLink(sel('see [docs](url)', '[docs](url)')).text, 'see docs')
eq('code block fences', toggleCodeBlock(sel('x', 'x')).text, '```\nx\n```\n\n')
eq('table on own lines', insertTable(at('para', 4)).text.split('\n').slice(0, 3), ['para', '', '| Column | Column | Column |'])
eq('front matter split', splitFrontMatter('---\nkind: spec\n---\n# Hi\n'), ['---\nkind: spec\n---\n', '# Hi\n'])
eq('no front matter', splitFrontMatter('# Hi\n---\n'), ['', '# Hi\n---\n'])

if (failed) throw new Error(`${failed} failed`)
console.log('all passed')
