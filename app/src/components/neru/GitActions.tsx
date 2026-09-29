import { useState } from 'react'
import type { GitFile, RemoteInfo } from '../../types'

const conflict = (file: GitFile) => /U/.test(file.status) || file.status === 'AA' || file.status === 'DD'

export function GitActions({ busy, branch, branches, files, remote, onFetch, onPull, onCheckout, onMerge, onRebase, onStash, onPush, onPullRequest }: {
  busy: boolean
  branch: string
  branches: string[]
  files: GitFile[]
  remote: RemoteInfo | null
  onFetch: () => void
  onPull: () => void
  onCheckout: (name: string) => void
  onMerge: (name: string) => void
  onRebase: (name: string) => void
  onStash: (action: 'push' | 'pop') => void
  onPush: () => void
  onPullRequest: (title: string, body: string) => void
}) {
  const [other, setOther] = useState('')
  const [title, setTitle] = useState('')
  const [body, setBody] = useState('')
  const others = branches.filter(name => name !== branch)
  const web = remote?.web || remote?.github
  const conflicts = files.filter(conflict)
  return <div className="git-actions">
    <div className="remote-actions">
      <button className="button subtle" disabled={busy} onClick={onFetch}>Fetch</button>
      <button className="button subtle" disabled={busy || !remote?.remote} onClick={onPull}>Pull{remote?.behind ? ` ${remote.behind}` : ''}</button>
      <button className="button subtle" disabled={busy} onClick={() => onStash('push')}>Stash</button>
      <button className="button subtle" disabled={busy} onClick={() => onStash('pop')}>Pop stash</button>
      {remote?.remote && <button className="button subtle" disabled={busy || branch === 'detached HEAD'} onClick={onPush}>Push{remote.ahead ? ` ${remote.ahead}` : ''}</button>}
    </div>
    <label className="git-select">Switch branch
      <select value={branch} disabled={busy} onChange={event => onCheckout(event.target.value)}>
        {branches.map(name => <option key={name} value={name}>{name}</option>)}
      </select>
    </label>
    <div className="field-row">
      <select value={other} onChange={event => setOther(event.target.value)} disabled={busy}>
        <option value="">Another branch…</option>
        {others.map(name => <option key={name} value={name}>{name}</option>)}
      </select>
      <button className="button subtle" disabled={busy || !other} onClick={() => onMerge(other)}>Merge</button>
      <button className="button subtle" disabled={busy || !other} onClick={() => onRebase(other)}>Rebase</button>
    </div>
    {conflicts.length > 0 && <p className="clone-error">{conflicts.length} conflicted file{conflicts.length === 1 ? '' : 's'}: {conflicts.map(file => file.path).join(', ')}. Open each file, resolve the markers, then stage it.</p>}
    {web && <div className="pr-form">
      <input value={title} onChange={event => setTitle(event.target.value)} placeholder={`Pull request title into ${remote?.base ?? 'the base branch'}`} />
      <textarea value={body} onChange={event => setBody(event.target.value)} rows={3} placeholder="What changed, and how to review it" />
      <button className="button subtle" disabled={busy || branch === remote?.base} onClick={() => onPullRequest(title, body)}>Create pull request</button>
    </div>}
  </div>
}
