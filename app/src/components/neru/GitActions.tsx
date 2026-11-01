import { useState } from 'react'
import type { GitFile, RemoteInfo } from '../../types'
import { t } from '../../i18n'
import { Select } from '../ui/Select'

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
      <button className="button subtle" disabled={busy} onClick={onFetch}>{t('Fetch')}</button>
      <button className="button subtle" disabled={busy || !remote?.remote} onClick={onPull}>{t('Pull')}{remote?.behind ? ` ${remote.behind}` : ''}</button>
      <button className="button subtle" disabled={busy} onClick={() => onStash('push')}>{t('Stash')}</button>
      <button className="button subtle" disabled={busy} onClick={() => onStash('pop')}>{t('Pop stash')}</button>
      {remote?.remote && <button className="button subtle" disabled={busy || branch === 'detached HEAD'} onClick={onPush}>{t('Push')}{remote.ahead ? ` ${remote.ahead}` : ''}</button>}
    </div>
    <label className="git-select">{t('Switch branch')}
      <Select value={branch} disabled={busy} onChange={event => onCheckout(event.target.value)}>
        {branches.map(name => <option key={name} value={name}>{name}</option>)}
      </Select>
    </label>
    <div className="field-row">
      <Select value={other} onChange={event => setOther(event.target.value)} disabled={busy}>
        <option value="">{t('Another branch…')}</option>
        {others.map(name => <option key={name} value={name}>{name}</option>)}
      </Select>
      <button className="button subtle" disabled={busy || !other} onClick={() => onMerge(other)}>{t('Merge')}</button>
      <button className="button subtle" disabled={busy || !other} onClick={() => onRebase(other)}>{t('Rebase')}</button>
    </div>
    {conflicts.length > 0 && <p className="clone-error">{conflicts.length === 1 ? t('1 conflicted file: {files}. Open each file, resolve the markers, then stage it.', { files: conflicts[0].path }) : t('{n} conflicted files: {files}. Open each file, resolve the markers, then stage it.', { n: conflicts.length, files: conflicts.map(file => file.path).join(', ') })}</p>}
    {web && <div className="pr-form">
      <input value={title} onChange={event => setTitle(event.target.value)} placeholder={remote?.base ? t('Pull request title into {base}', { base: remote.base }) : t('Pull request title into the base branch')} />
      <textarea value={body} onChange={event => setBody(event.target.value)} rows={3} placeholder={t('What changed, and how to review it')} />
      <button className="button subtle" disabled={busy || branch === remote?.base} onClick={() => onPullRequest(title, body)}>{t('Create pull request')}</button>
    </div>}
  </div>
}
