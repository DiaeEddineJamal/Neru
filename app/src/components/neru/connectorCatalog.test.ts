import { describe, expect, it } from 'vitest'
import { CATALOG, CATEGORIES, explainConnectorError, filterCatalog } from './connectorCatalog'

describe('connector catalog', () => {
  it('has unique ids that are valid connector names', () => {
    const ids = CATALOG.map(entry => entry.id)
    expect(new Set(ids).size).toBe(ids.length)
    // mcp.rs accepts letters, digits, - and _ only.
    for (const id of ids) expect(id).toMatch(/^[a-z0-9_-]+$/)
  })

  it('gives every entry a known category and a description', () => {
    const known = new Set(CATEGORIES.map(category => category.id))
    for (const entry of CATALOG) {
      expect(known.has(entry.category), entry.id).toBe(true)
      expect(entry.description.length, entry.id).toBeGreaterThan(5)
    }
    for (const category of CATEGORIES) expect(CATALOG.some(entry => entry.category === category.id), category.id).toBe(true)
  })

  it('points hosted entries at https and gives local entries a command', () => {
    for (const entry of CATALOG) {
      if (entry.url) {
        expect(new URL(entry.url).protocol, entry.id).toBe('https:')
        expect(entry.auth, entry.id).toBeDefined()
        expect(entry.command, entry.id).toBeUndefined()
      } else {
        expect(['npx', 'uvx'], entry.id).toContain(entry.command)
        expect(entry.args?.trim(), entry.id).toBeTruthy()
      }
      if (entry.setupUrl) expect(new URL(entry.setupUrl).protocol, entry.id).toBe('https:')
      for (const field of entry.env ?? []) expect(field.key, entry.id).toMatch(/^[A-Z][A-Z0-9_]*$/)
    }
  })

  it('marks /sse endpoints as the older transport', () => {
    for (const entry of CATALOG.filter(entry => entry.url && /\/sse\/?$/.test(entry.url))) expect(entry.transport, entry.id).toBe('sse')
  })

  it('searches names, descriptions and keywords, within a category', () => {
    expect(filterCatalog(CATALOG, 'gmail').map(entry => entry.id)).toContain('google-workspace')
    expect(filterCatalog(CATALOG, 'GOOGLE calendar').map(entry => entry.id)).toContain('google-workspace')
    expect(filterCatalog(CATALOG, 'arxiv').map(entry => entry.id)).toContain('arxiv')
    expect(filterCatalog(CATALOG, 'zotéro').map(entry => entry.id)).toEqual(['zotero'])
    expect(filterCatalog(CATALOG, 'no-such-connector-xyz')).toEqual([])
    const research = filterCatalog(CATALOG, '', 'research')
    expect(research.length).toBeGreaterThan(3)
    expect(research.every(entry => entry.category === 'research')).toBe(true)
    expect(filterCatalog(CATALOG, 'notion', 'research')).toEqual([])
    expect(filterCatalog(CATALOG, '')).toHaveLength(CATALOG.length)
  })

  it('explains common failures in plain words', () => {
    expect(explainConnectorError("npx was not found on this PC. Install Node.js from https://nodejs.org/en/download").link?.url).toContain('nodejs.org')
    expect(explainConnectorError("'uvx' is not recognized as an internal or external command").link?.url).toContain('astral.sh')
    expect(explainConnectorError('tools/list timed out after 30 seconds').message).toMatch(/too long/)
    expect(explainConnectorError('The connector answered 404 Not Found: ').message).toMatch(/URL/)
    expect(explainConnectorError('something odd').message).toBe('The connector could not start.')
  })
})
