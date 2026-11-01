import { readFileSync } from 'node:fs'
import { join } from 'node:path'
import { describe, expect, it } from 'vitest'

// jsdom has no layout, so this guards the rule instead: every floating surface caps its height to the
// window (and scrolls inside), so a short or narrow window never pushes part of it off-screen.
const css = ['App.css', 'components/neru/ArtifactPane.css', 'components/neru/SidePanels.css', 'components/neru/Tour.css']
  .map(file => readFileSync(join(__dirname, '..', file), 'utf8'))
  .join('\n')
  .replace(/\/\*[\s\S]*?\*\//g, '')

/** True when some rule that names `selector` also limits the height to the window. */
function capped(selector: string) {
  const rules = css.match(/[^{}]+\{[^{}]*\}/g) ?? []
  return rules.some(rule => {
    const [head, body] = rule.split('{')
    const names = head.split(',').map(name => name.trim())
    return names.some(name => name === selector || name.endsWith(` ${selector}`)) && /max-height:[^;]*(dvh|vh)/.test(body)
  })
}

describe('floating surfaces fit the window', () => {
  const surfaces = ['.palette', '.clone-dialog', '.deny-dialog', '.whats-new', '.compare-panel', '.slash-menu', '.mic-menu-panel', '.send-menu-panel', '.context-panel', '.title-menu', '.rewind-menu', '.pv-menu-panel', '.artifact-menu', '.menu', '.context-menu', '.tour-card']
  it.each(surfaces)('%s caps its height to the window', selector => {
    expect(capped(selector)).toBe(true)
  })
  it('the command palette scrolls its results, not the search box', () => {
    expect(css).toMatch(/\.palette-results \{[^}]*overflow-y: auto/)
    expect(css).toMatch(/\.palette-backdrop \{[^}]*align-items: flex-start/)
  })
  it('dialog backdrops start tall content at the top instead of clipping it', () => {
    expect(css).toMatch(/\.modal-backdrop, \.whats-new-backdrop, \.compare-overlay, \.settings-backdrop \{[^}]*align-items: safe center/)
  })
})
