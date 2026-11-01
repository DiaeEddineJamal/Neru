import { useEffect, useState } from 'react'
import { isTauri } from '@tauri-apps/api/core'
import { Brain, Lock, PiggyBank, ShieldAlert, ShieldCheck, ShieldQuestion, Trash2 } from 'lucide-react'
import { api, type PrivacyPrefs } from '../../api'
import { privacyLabels, providerPresets, type Privacy, type PrivacyRule } from '../../providerCatalog'
import type { UsageTotals } from '../../types'
import { cn } from '@/lib/utils'
import { dollars, referenceCost, REFERENCE, tokens } from '@/lib/savings'
import { t } from '../../i18n'

const RULES: { value: PrivacyRule; label: string }[] = [
  { value: 'any', label: 'Any provider' },
  { value: 'no-train', label: 'Not trained on' },
  { value: 'local', label: 'Only this computer' },
]
const icons: Record<Privacy, typeof Lock> = { local: Lock, 'no-training': ShieldCheck, 'may-train': ShieldAlert, unknown: ShieldQuestion }

/** A provider's privacy level as a small tag, from the catalog's terms summary. */
export function PrivacyTag({ privacy }: { privacy: Privacy }) {
  const Icon = icons[privacy]
  return <span className={cn('privacy-tag', privacy)} title={t(privacyLabels[privacy].detail)}><Icon size={12} aria-hidden />{t(privacyLabels[privacy].label)}</span>
}

/**
 * Settings → Privacy: what Neru has cost (nothing, on free models) against a paid model, which providers may
 * see your code, secret hiding, second opinions, and the rules Neru learned from changes you denied.
 */
export function PrivacySettings({ projectOpen, onError, onNotice }: { projectOpen: boolean; onError: (message: string) => void; onNotice: (message: string) => void }) {
  const [prefs, setPrefs] = useState<PrivacyPrefs | null>(null)
  const [projectRule, setProjectRule] = useState<PrivacyRule | null>(null)
  const [usage, setUsage] = useState<UsageTotals | null>(null)
  const [facts, setFacts] = useState<string[]>([])
  const [fact, setFact] = useState('')
  const desktop = isTauri()

  useEffect(() => {
    if (!desktop) return
    void api.privacyPrefs().then(setPrefs).catch(cause => onError(String(cause)))
    void api.usageTotals().then(setUsage).catch(() => undefined)
    if (projectOpen) {
      void api.projectPrivacy().then(value => setProjectRule(value.rule)).catch(() => undefined)
      void api.projectMemoryFacts().then(setFacts).catch(() => undefined)
    }
  }, [desktop, projectOpen, onError])

  const save = async (next: PrivacyPrefs) => {
    setPrefs(next)
    try { setPrefs(await api.setPrivacyPrefs(next)) } catch (cause) { onError(String(cause)) }
  }
  const saved = usage ? referenceCost(usage.free) : 0
  const learned = facts.filter(item => item.startsWith('Learned from a rejected change'))

  return <section className="settings-section privacy-settings"><h2>{t('Privacy and savings')}</h2>
    <p className="settings-lede">{t('Free models cost nothing, and what you send stays under your rules.')}</p>

    <div className="savings-card">
      <span className="savings-eyebrow"><PiggyBank size={15} aria-hidden /> {t('Spent')}</span>
      <strong className="savings-total">$0.00</strong>
      <p>{usage && usage.free.replies
        ? t('{tokens} tokens on free and local models. On {model} the same work would have cost about {cost}.', { tokens: tokens(usage.free.input + usage.free.output), model: REFERENCE.name, cost: dollars(saved) })
        : t('Replies on free keys and local models cost nothing. Neru counts what the same tokens would cost on {model}.', { model: REFERENCE.name })}</p>
      {usage && usage.paid.replies > 0 && <p className="savings-paid">{usage.paid.replies === 1 ? t('1 reply used your paid keys ({tokens} tokens), billed by those providers.', { tokens: tokens(usage.paid.input + usage.paid.output) }) : t('{n} replies used your paid keys ({tokens} tokens), billed by those providers.', { n: usage.paid.replies, tokens: tokens(usage.paid.input + usage.paid.output) })}</p>}
      <button type="button" className="button subtle small" onClick={() => void api.resetUsage().then(setUsage).catch(cause => onError(String(cause)))}>{t('Reset counter')}</button>
    </div>

    <h3 className="cli-heading">{t('Who sees your code')}</h3>
    <div className="settings-row"><div><strong>{t('Providers Neru may use')}</strong><p>{t('For chats and for projects without their own rule. Model switching follows it too.')}</p></div>
      <select className="settings-select" aria-label={t('Providers Neru may use')} value={prefs?.privacy ?? 'any'} disabled={!prefs} onChange={event => prefs && void save({ ...prefs, privacy: event.target.value as PrivacyRule })}>{RULES.map(rule => <option key={rule.value} value={rule.value}>{t(rule.label)}</option>)}</select></div>
    {projectOpen && <div className="settings-row"><div><strong>{t('This project')}</strong><p>{t('Saved in .neru/privacy.json, so a team can commit it: "this repo only uses local models".')}</p></div>
      <select className="settings-select" aria-label={t('Privacy rule for this project')} value={projectRule ?? ''} onChange={event => { const value = (event.target.value || null) as PrivacyRule | null; void api.setProjectPrivacy(value).then(result => { setProjectRule(result.rule); onNotice(result.rule ? t('This project now has its own privacy rule.') : t('This project follows the global rule again.')) }).catch(cause => onError(String(cause))) }}>
        <option value="">{t('Same as global')}</option>{RULES.map(rule => <option key={rule.value} value={rule.value}>{t(rule.label)}</option>)}</select></div>}
    <div className="settings-row"><div><strong>{t('Hide secrets before sending')}</strong><p>{t('API keys, tokens, passwords and private keys leave as placeholders like «secret:OpenAI key:1a2b3c». Neru puts the real values back in the edits and commands it runs.')}</p></div>
      <button type="button" className={cn('switch', prefs?.redact && 'on')} role="switch" aria-checked={Boolean(prefs?.redact)} aria-label={t('Hide secrets before sending')} disabled={!prefs} onClick={() => prefs && void save({ ...prefs, redact: !prefs.redact })} /></div>
    <ul className="privacy-providers">
      {providerPresets.filter(item => item.id !== 'custom').map(item => <li key={item.id}><span>{item.name}</span><PrivacyTag privacy={item.privacy} /></li>)}
    </ul>
    <p className="cli-note">{t("From each provider's published terms, free tiers included. Terms change, so check before relying on them.")}</p>

    <h3 className="cli-heading">{t('Second opinion')}</h3>
    <div className="settings-row"><div><strong>{t('Have another model check each change')}</strong><p>{t('A model of a different family (Gemini checking Qwen, say) reads every proposed edit while it waits for you, and says if it agrees. Uses free models only.')}</p></div>
      <button type="button" className={cn('switch', prefs?.secondOpinion && 'on')} role="switch" aria-checked={Boolean(prefs?.secondOpinion)} aria-label={t('Second opinion')} disabled={!prefs} onClick={() => prefs && void save({ ...prefs, secondOpinion: !prefs.secondOpinion })} /></div>

    {projectOpen && <>
      <h3 className="cli-heading"><Brain size={15} aria-hidden /> {t('What Neru learned about this project')}</h3>
      <p className="cli-note">{learned.length ? (learned.length === 1 ? t('1 rule from changes you denied, and {other} other notes.', { other: facts.length - learned.length }) : t('{n} rules from changes you denied, and {other} other notes.', { n: learned.length, other: facts.length - learned.length })) : t('When you deny a change and say why, Neru keeps the reason here and follows it in later sessions.')}</p>
      <ul className="learned-list">
        {facts.map(item => <li key={item}><span>{item}</span><button type="button" className="icon-button small" aria-label={t('Forget: {item}', { item })} title={t('Forget')} onClick={() => void api.forgetProjectFact(item).then(setFacts).catch(cause => onError(String(cause)))}><Trash2 size={13} /></button></li>)}
      </ul>
      <form className="learned-add" onSubmit={event => { event.preventDefault(); if (fact.trim()) void api.rememberProjectFact(fact.trim()).then(list => { setFacts(list); setFact('') }).catch(cause => onError(String(cause))) }}>
        <input value={fact} onChange={event => setFact(event.target.value)} placeholder={t('Add a rule, e.g. Use pnpm, not npm')} aria-label={t('New rule')} />
        <button type="submit" className="button subtle" disabled={!fact.trim()}>{t('Add')}</button>
      </form>
    </>}
  </section>
}
