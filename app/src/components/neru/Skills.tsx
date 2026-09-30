import { useEffect, useState } from 'react'
import { open } from '@tauri-apps/plugin-dialog'
import { FilePlus2, FolderOpen, FolderPlus, Sparkles, Trash2 } from 'lucide-react'
import { api } from '../../api'
import type { SkillView } from '../../types'

const SOURCE_LABEL: Record<string, string> = { personal: 'Added in Settings', project: 'This project · .neru/skills', claude: 'This project · .claude/skills', 'built-in': 'Built in' }

/** Groups for the built-in skills, in the order they are shown. */
const GROUPS: { title: string; hint: string; names: string[] }[] = [
  { title: 'Design', hint: 'Distinctive interfaces instead of generic AI layouts', names: ['frontend-design', 'impeccable'] },
  { title: 'Engineering method', hint: 'Plan, test first, debug from evidence, verify before claiming done', names: ['brainstorming', 'writing-plans', 'executing-plans', 'test-driven-development', 'systematic-debugging', 'verification-before-completion', 'requesting-code-review', 'receiving-code-review', 'finishing-a-development-branch'] },
  { title: 'Security', hint: 'Reviews and audits from Trail of Bits, and an OWASP checklist', names: ['neru-web-security', 'differential-review', 'sharp-edges', 'supply-chain-risk-auditor', 'audit-context-building', 'property-based-testing'] },
  { title: 'Writing and tools', hint: 'Plain, specific prose, and building MCP servers', names: ['neru-writing', 'mcp-builder'] },
]
const CREDIT: Record<string, string> = { 'frontend-design': 'Anthropic', impeccable: 'Impeccable', 'mcp-builder': 'Anthropic', 'neru-writing': 'Neru', 'neru-web-security': 'Neru' }
const creditFor = (name: string) => CREDIT[name] ?? (GROUPS[1].names.includes(name) ? 'Superpowers' : 'Trail of Bits')

/** Settings → Skills: add Markdown skills the agent loads with read_skill when a task matches. */
export function Skills({ onError, onNotice }: { onError: (message: string) => void; onNotice: (message: string) => void }) {
  const [skills, setSkills] = useState<SkillView[]>([])
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    let active = true
    api.listSkills().then(list => { if (active) setSkills(list) }).catch(cause => onError(String(cause))).finally(() => { if (active) setLoading(false) })
    return () => { active = false }
  }, [onError])

  const importPaths = async (paths: string[]) => {
    if (paths.length === 0) return
    try {
      const before = new Set(skills.map(skill => skill.name))
      const next = await api.importSkills(paths)
      setSkills(next)
      const added = next.filter(skill => skill.source === 'personal' && !before.has(skill.name)).map(skill => skill.name)
      onNotice(added.length ? `Added ${added.join(', ')}. Neru uses a skill when a task matches its description.` : 'Skills updated.')
    } catch (cause) { onError(String(cause)) }
  }
  const addFiles = async () => {
    const picked = await open({ multiple: true, title: 'Add skill files', filters: [{ name: 'Skills', extensions: ['md', 'markdown', 'txt'] }] })
    await importPaths(Array.isArray(picked) ? picked : picked ? [picked] : [])
  }
  const addFolder = async () => {
    const picked = await open({ directory: true, multiple: true, title: 'Add skill folders (each with a SKILL.md)' })
    await importPaths(Array.isArray(picked) ? picked : picked ? [picked] : [])
  }
  const remove = async (name: string) => {
    try { setSkills(await api.removeSkill(name)); onNotice(`Removed ${name}.`) } catch (cause) { onError(String(cause)) }
  }

  const toggle = async (skill: SkillView) => {
    try { setSkills(await api.setSkillEnabled(skill.name, !skill.enabled)); onNotice(`${skill.enabled ? 'Turned off' : 'Turned on'} ${skill.name}.`) } catch (cause) { onError(String(cause)) }
  }
  const personal = skills.filter(skill => skill.source === 'personal')
  const project = skills.filter(skill => skill.source === 'project' || skill.source === 'claude')
  const builtIn = skills.filter(skill => skill.source === 'built-in')
  const grouped = GROUPS.map(group => ({ ...group, skills: group.names.flatMap(name => builtIn.filter(skill => skill.name === name)) }))
  const other = builtIn.filter(skill => !GROUPS.some(group => group.names.includes(skill.name)))
  if (other.length) grouped.push({ title: 'More', hint: '', names: [], skills: other })
  const row = (skill: SkillView) => <li key={`${skill.source}-${skill.name}`} className="skill-row">
    <span className="skill-icon"><Sparkles size={15} /></span>
    <div className="skill-text">
      <strong>/{skill.name}</strong>
      <p>{skill.description}</p>
      <small>{skill.source === 'built-in' ? `Built in · ${creditFor(skill.name)}` : SOURCE_LABEL[skill.source] ?? skill.source} · {skill.chars.toLocaleString()} characters</small>
    </div>
    {skill.source === 'built-in' && <label className="skill-switch" title={skill.enabled ? 'On: Neru uses it when a task matches' : 'Off'}>
      <input type="checkbox" role="switch" checked={skill.enabled} onChange={() => void toggle(skill)} aria-label={`${skill.name} ${skill.enabled ? 'on' : 'off'}`} />
      <span aria-hidden />
    </label>}
    {skill.source === 'personal' && <button type="button" className="icon-button" onClick={() => void remove(skill.name)} aria-label={`Remove ${skill.name}`} title="Remove"><Trash2 size={15} /></button>}
  </li>

  return <section className="settings-section skills-settings"><h2>Skills</h2>
    <p className="settings-lede">Skills are Markdown instructions for tasks you repeat: your review checklist, a framework’s conventions, how to deploy. Neru sees each skill’s description, loads the full text when a task matches, and you can run one yourself with <kbd>/name</kbd>.</p>
    <div className="skills-actions">
      <button type="button" className="button primary" onClick={() => void addFiles()}><FilePlus2 size={15} /> Add skill files</button>
      <button type="button" className="button subtle" onClick={() => void addFolder()}><FolderPlus size={15} /> Add skill folder</button>
      <button type="button" className="button subtle" onClick={() => void api.openSkillsFolder().catch(cause => onError(String(cause)))}><FolderOpen size={15} /> Open skills folder</button>
    </div>
    <p className="settings-note">A skill is a <code>SKILL.md</code> with <code>name</code> and <code>description</code> front matter, or any Markdown file; Neru names plain files after the file and uses the first line as the description. Folders keep their extra files for the skill to reference.</p>
    <h3 className="skills-heading">Your skills <span>{personal.length}</span></h3>
    {loading ? <p className="empty-small">Loading skills…</p> : personal.length ? <ul className="skill-list">{personal.map(row)}</ul> : <p className="empty-small">No skills yet. Add a Markdown file to get started; it works in every project.</p>}
    {project.length > 0 && <><h3 className="skills-heading">From the open project <span>{project.length}</span></h3><ul className="skill-list">{project.map(row)}</ul></>}
    {builtIn.length > 0 && <>
      <h3 className="skills-heading">Built in <span>{builtIn.filter(skill => skill.enabled).length} of {builtIn.length} on</span></h3>
      <p className="settings-note">Curated from the most used open skills — Anthropic, Superpowers, Impeccable and Trail of Bits — so any model plans, tests, reviews and designs like a careful engineer. Your own or a project’s skill with the same name replaces the built-in one.</p>
      {grouped.filter(group => group.skills.length).map(group => <div key={group.title} className="skill-group">
        <h4>{group.title}{group.hint && <span>{group.hint}</span>}</h4>
        <ul className="skill-list">{group.skills.map(row)}</ul>
      </div>)}
    </>}
  </section>
}
