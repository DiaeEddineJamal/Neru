import { useEffect, useState } from 'react'
import { api } from '../../api'
import { providerPresets } from '../../providerCatalog'
import type { ProviderCapacity } from '../../types'
import { cn } from '@/lib/utils'
import { t } from '../../i18n'

const count = (value: number) => value.toLocaleString()
const nameOf = (id: string) => providerPresets.find(preset => preset.id === id)?.name ?? id

/** Today's free capacity on every saved free key, in the context panel: free plans run out of requests
 *  per day before anything else, so this is what tells you how much work is left. `refresh` changes
 *  after every reply. */
export function FreeCapacity({ refresh }: { refresh: unknown }) {
  const [plans, setPlans] = useState<ProviderCapacity[]>([])
  useEffect(() => {
    let live = true
    void api.freeCapacity().then(list => { if (live) setPlans(list) }).catch(() => undefined)
    return () => { live = false }
  }, [refresh])
  return <FreeCapacityList plans={plans} />
}

export function FreeCapacityList({ plans }: { plans: ProviderCapacity[] }) {
  if (!plans.length) return null
  const known = plans.filter(plan => plan.left !== null)
  const left = known.reduce((sum, plan) => sum + (plan.left ?? 0), 0)
  const summary = known.length ? t(known.length < plans.length ? '{n}+ requests left' : '{n} requests left', { n: count(left) }) : t('{n} free plans', { n: plans.length })
  return <span className="context-quotas free-capacity">
    <span className="context-row"><span>{t('Free plans today')}</span><b>{summary}</b></span>
    {plans.map(plan => {
      const share = plan.limit && plan.left !== null ? Math.max(0, Math.min(1, plan.left / plan.limit)) : null
      return <span key={plan.providerId} className="context-quota">
        <span className="context-row"><span className={cn(plan.current && 'is-current')}>{nameOf(plan.providerId)}</span><b>{plan.left !== null ? t('{used} used · {left} left', { used: count(plan.usedToday), left: count(plan.left) }) : t('{used} used', { used: count(plan.usedToday) })}</b></span>
        {share !== null && <i className={cn(share < 0.1 && 'low')} style={{ ['--left' as string]: `${share * 100}%` }} />}
        {plan.restingModels > 0 && <small>{t(plan.restingModels === 1 ? '1 model resting after a limit, back in {time}' : '{n} models resting after a limit, first back in {time}', { n: plan.restingModels, time: plan.backIn ?? '' })}</small>}
        {plan.current && <small>{t('Its day starts over in {time}', { time: plan.resetsIn })}</small>}
      </span>
    })}
  </span>
}
