import { useEffect, useState } from 'react'
import { Check, ChevronDown, CircleAlert, ExternalLink, Eye, EyeOff, KeyRound, LoaderCircle, ShieldCheck } from '@/components/icons'
import { api } from '../../api'
import { formatForModel, privacyLabels, providerGroups, providerPresets, type ApiFormat, type ProviderPreset } from '../../providerCatalog'
import type { Allowance, EngineSettings, ModelInfo, ProviderView } from '../../types'
import { hostOf } from '@/lib/modelFamily'
import { cn } from '@/lib/utils'
import { t } from '../../i18n'
import { ModelLogo } from './ModelSelector'
import { SettingsModelPicker } from './ModelPicker'
import { LocalModels } from './LocalModels'
import { BuiltinModels } from './BuiltinModels'
import { PrivacyTag } from './PrivacySettings'
import './ProviderSettings.css'
import { Select } from '../ui/Select'

// Settings → Model provider, in three plain steps: pick a provider, connect it with a key, pick a model.
// The technical fields (API format, address) wait under Advanced, already filled in for every preset.

type Props = {
  provider: ProviderView
  savedKeys: string[]
  providerId: string; onProvider: (id: string) => void
  format: ApiFormat; onFormat: (format: ApiFormat) => void
  baseUrl: string; onBaseUrl: (url: string) => void
  apiKey: string; onApiKey: (key: string) => void
  model: string; onModel: (model: string) => void
  models: ModelInfo[]; onModels: (update: (models: ModelInfo[]) => ModelInfo[]) => void
  loading: boolean; error: string
  engine: EngineSettings | null; onEngine: (next: EngineSettings['architectEditor']) => void
  onSave: () => void
  onForgetKeys: () => void
}

const isLocalUrl = (url: string) => /localhost|127\.0\.0\.1|\[::1\]/.test(url)

function ProviderCard({ preset, selected, inUse, hasKey, onPick }: { preset: ProviderPreset; selected: boolean; inUse: boolean; hasKey: boolean; onPick: () => void }) {
  const host = hostOf(preset.id)
  return <button type="button" role="radio" aria-checked={selected} className={cn('provider-card', selected && 'is-selected')} onClick={onPick}>
    <span className="provider-card-logo" aria-hidden>{host.logo ? <ModelLogo logo={host.logo} size={20} /> : <span className="provider-card-initial">{preset.name.slice(0, 1)}</span>}</span>
    <span className="provider-card-text">
      <span className="provider-card-name">{preset.name}
        {preset.recommended && <span className="provider-pill recommended">{t('Recommended')}</span>}
        {inUse ? <span className="provider-pill in-use"><Check size={11} />{t('In use')}</span> : hasKey && <span className="provider-pill">{t('Key saved')}</span>}
      </span>
      <span className="provider-card-tagline">{t(preset.tagline)}</span>
    </span>
  </button>
}

function Step({ n, title, done, children }: { n: number; title: string; done?: boolean; children: React.ReactNode }) {
  return <section className="provider-step" aria-labelledby={`provider-step-${n}`}>
    <h3 id={`provider-step-${n}`}><span className={cn('provider-step-n', done && 'is-done')} aria-hidden>{done ? <Check size={13} /> : n}</span>{t(title)}</h3>
    <div className="provider-step-body">{children}</div>
  </section>
}

export function ProviderSettings(props: Props) {
  const { provider, savedKeys, providerId, models, loading, error } = props
  const preset = providerPresets.find(item => item.id === providerId) ?? providerPresets[0]
  const [showKey, setShowKey] = useState(false)
  const [morePaid, setMorePaid] = useState(false)
  const [found, setFound] = useState<{ id: string; allowance: Allowance | null } | null>(null)
  const allowance = found?.id === providerId ? found.allowance : null
  const savedKey = (provider.hasKey && provider.providerId === providerId) || savedKeys.includes(providerId)
  const local = preset.group === 'local' || isLocalUrl(props.baseUrl)
  const keyless = local || Boolean(preset.keyOptional)
  const connected = models.length > 0
  const ready = models.filter(info => info.verified === 'ok').length
  const inUse = provider.configured && provider.providerId === providerId && provider.model === props.model
  const canSave = Boolean(props.model.trim()) && models.some(info => info.id === props.model && info.verified !== 'unavailable')

  // What the key's free plan has left, for providers that say (OpenRouter).
  useEffect(() => {
    if (!connected || preset.group !== 'free') return
    let live = true
    void api.providerAllowance(providerId, props.baseUrl, props.apiKey).then(allowance => { if (live) setFound({ id: providerId, allowance }) }).catch(() => undefined)
    return () => { live = false }
  }, [connected, providerId]) // eslint-disable-line react-hooks/exhaustive-deps

  const keyStatus = loading
    ? <p className="provider-status is-busy" role="status"><LoaderCircle size={14} className="animate-spin" />{local ? t('Looking for models on this computer…') : t('Checking the key…')}</p>
    : error
      ? <p className="provider-status is-error" role="alert"><CircleAlert size={14} />{error}</p>
      : connected
        ? <p className="provider-status is-ok" role="status"><Check size={14} />{local ? t('Connected. {n} models found.', { n: models.length }) : t('The key works. {n} models found.', { n: models.length })}{allowance?.note ? ` ${allowance.note}` : ''}</p>
        : null

  const groups = providerGroups.map(group => ({ ...group, items: providerPresets.filter(item => item.group === group.id).sort((a, b) => Number(Boolean(b.recommended)) - Number(Boolean(a.recommended))) }))

  return <section className="settings-section provider-settings provider-flow">
    <h2>{t('Model provider')}</h2>
    <p className="settings-lede">{t('Choose where Neru’s AI comes from. A free provider takes about a minute: make an account, copy a key, paste it here.')}</p>

    {provider.configured && <div className="provider-current">
      <span className="provider-current-dot" aria-hidden />
      <span>{t('Now using')} <strong>{providerPresets.find(item => item.id === provider.providerId)?.name ?? provider.providerId}</strong>{provider.model && <> · <span className="provider-current-model">{provider.model}</span></>}</span>
    </div>}

    <Step n={1} title="Choose a provider" done={connected}>
      {groups.map(group => {
        const collapsed = group.id === 'paid' && !morePaid && preset.group !== 'paid'
        return <div key={group.id} className="provider-group">
          <div className="provider-group-head"><strong>{t(group.title)}</strong><span>{t(group.hint)}</span></div>
          {collapsed
            ? <button type="button" className="provider-more" onClick={() => setMorePaid(true)}><ChevronDown size={14} />{t('Show {n} paid providers', { n: group.items.length })}</button>
            : <div className="provider-cards" role="radiogroup" aria-label={t(group.title)}>
              {group.items.map(item => <ProviderCard key={item.id} preset={item} selected={item.id === providerId} inUse={provider.configured && provider.providerId === item.id} hasKey={savedKeys.includes(item.id)} onPick={() => props.onProvider(item.id)} />)}
            </div>}
        </div>
      })}
    </Step>

    <Step n={2} title={local ? 'Connect' : 'Add your key'} done={connected}>
      <div className="provider-connect">
        <div className="provider-about">
          <p>{t(preset.description)}</p>
          <p className="provider-facts">
            {preset.freeLimit && <span className="provider-fact"><strong>{t('Free allowance')}</strong>{t(preset.freeLimit)}</span>}
            <span className="provider-fact"><strong>{t('Your data')}</strong><PrivacyTag privacy={preset.privacy} /><span className="provider-privacy-detail">{t(privacyLabels[preset.privacy].detail)}</span></span>
          </p>
        </div>
        {/* Ollama gets the local model library instead of written steps: download, remove, start. */}
        {preset.id === 'builtin'
          ? <BuiltinModels onChanged={() => void api.listModels(providerId, props.format, props.baseUrl, '').then(list => props.onModels(() => list)).catch(() => undefined)} />
          : preset.id === 'ollama'
          ? <LocalModels onChanged={() => void api.listModels(providerId, props.format, props.baseUrl, '').then(list => props.onModels(() => list)).catch(() => undefined)} />
          : preset.keySteps && <ol className="provider-key-steps">{preset.keySteps.map(step => <li key={step}>{t(step)}</li>)}</ol>}
        {preset.keyUrl && preset.id !== 'ollama' && <a className="button subtle provider-key-button" href={preset.keyUrl} target="_blank" rel="noreferrer">{local ? <>{t('Open {name}', { name: preset.name })}</> : <><KeyRound size={14} />{t('Get a {name} key', { name: preset.name })}</>}<ExternalLink size={12} /></a>}
        {preset.id !== 'local' && preset.group !== 'local' && <label className="provider-key-field">{preset.keyOptional ? t('API key (optional)') : t('API key')}
          <span className="provider-key-input">
            <input type={showKey ? 'text' : 'password'} autoComplete="off" spellCheck={false} value={props.apiKey} onChange={event => props.onApiKey(event.target.value)}
              placeholder={savedKey ? t('Saved on this PC. Paste a new key to replace it.') : preset.keyOptional ? t('Leave empty to use it without a key') : t('Paste your key here')} />
            <button type="button" className="icon-button" onClick={() => setShowKey(value => !value)} aria-label={showKey ? t('Hide key') : t('Show key')} title={showKey ? t('Hide key') : t('Show key')}>{showKey ? <EyeOff size={15} /> : <Eye size={15} />}</button>
          </span>
        </label>}
        {keyStatus ?? (!keyless && !savedKey && <p className="provider-status">{t('Once you paste a key, Neru checks it and tests which models it can use.')}</p>)}
      </div>
    </Step>

    <Step n={3} title="Pick a model" done={inUse}>
      <SettingsModelPicker providerId={providerId} baseUrl={props.baseUrl} apiKey={props.apiKey} models={models} onModels={props.onModels} value={props.model} onSelect={props.onModel} loading={loading}
        formatFor={model => formatForModel(providerId, model, props.format, props.baseUrl)} emptyText={keyless ? t('Models appear here once Neru reaches the provider.') : t('Models appear here once your key is added.')} />
      {connected && !ready && !loading && models.some(info => info.verified === 'unknown') && <p className="settings-note">{t('Neru is testing which models answer. You can already pick one: it is tested first.')}</p>}
      <div className="provider-save">
        <button className="button primary" onClick={props.onSave} disabled={!canSave || inUse}>{inUse ? <><Check size={14} />{t('In use')}</> : t('Use {model}', { model: props.model ? (props.model.split('/').pop() ?? props.model) : t('this model') })}</button>
        <span className="provider-save-note"><ShieldCheck size={14} />{t('Keys are encrypted with your Windows account and sent only to this provider.')}</span>
      </div>
    </Step>

    <details className="provider-advanced">
      <summary><ChevronDown size={14} aria-hidden />{t('Advanced')}</summary>
      <p className="settings-note">{t('Filled in for you. Change these only for a custom server or if the provider’s instructions say so.')}</p>
      <div className="settings-grid">
        <label>{t('API format')}<Select value={props.format} onChange={event => props.onFormat(event.target.value as ApiFormat)}><option value="openai-chat">Chat Completions</option><option value="openai-responses">Responses</option><option value="anthropic">Anthropic Messages</option></Select></label>
        <label>{t('Address (base URL)')}<input value={props.baseUrl} onChange={event => props.onBaseUrl(event.target.value)} placeholder="https://provider.example/v1" autoComplete="url" spellCheck={false} /></label>
      </div>
      <div className="settings-row"><div><strong>{t('Architect and editor')}</strong><p>{t('The chosen model plans each change and a faster model on its own quota writes it. After two failed edits the chosen model edits again. Auto turns it on for free plans.')}</p></div>
        <Select className="settings-select" value={props.engine?.architectEditor ?? 'auto'} disabled={!props.engine} onChange={event => props.onEngine(event.target.value as EngineSettings['architectEditor'])}><option value="auto">{t('Auto')}</option><option value="on">{t('On')}</option><option value="off">{t('Off')}</option></Select></div>
      <div className="settings-row"><div><strong>{t('Saved keys')}</strong><p>{t('Remove every provider and voice key from this PC. You will need to paste them again.')}</p></div><button className="button subtle" onClick={props.onForgetKeys}><KeyRound size={15} /> {t('Forget saved keys')}</button></div>
    </details>
  </section>
}
