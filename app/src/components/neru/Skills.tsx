import { useEffect, useState } from 'react'
import { open } from '@tauri-apps/plugin-dialog'
import { FilePlus2, FolderOpen, FolderPlus, Sparkles, Trash2 } from 'lucide-react'
import { api } from '../../api'
import type { SkillView } from '../../types'

const SOURCE_LABEL: Record<string, string> = { personal: 'Added in Settings', project: 'This project · .neru/skills', claude: 'This project · .claude/skills' }

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

  const personal = skills.filter(skill => skill.source === 'personal')
  const project = skills.filter(skill => skill.source !== 'personal')
  const row = (skill: SkillView) => <li key={`${skill.source}-${skill.name}`} className="skill-row">
    <span className="skill-icon"><Sparkles size={15} /></span>
    <div className="skill-text">
      <strong>/{skill.name}</strong>
      <p>{skill.description}</p>
      <small>{SOURCE_LABEL[skill.source] ?? skill.source} · {skill.chars.toLocaleString()} characters</small>
    </div>
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
  </section>
}
