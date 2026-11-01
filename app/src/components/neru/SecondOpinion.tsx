import { CircleCheck, LoaderCircle, ShieldAlert, ShieldQuestion } from 'lucide-react'
import type { Review } from '../../types'
import { cn } from '@/lib/utils'
import { t } from '../../i18n'

/** A second model's opinion on the change waiting for approval: agrees, or has concerns with its notes. */
export function SecondOpinion({ review }: { review: Review }) {
  const title = review.status === 'running' ? t('{reviewer} is checking this change…', { reviewer: review.reviewer })
    : review.status === 'agree' ? t('2 models agree · {reviewer} checked it', { reviewer: review.reviewer })
    : review.status === 'concerns' ? t('{reviewer} has concerns', { reviewer: review.reviewer })
    : review.status === 'skipped' ? t('No second opinion')
    : t('The second opinion did not finish')
  const Icon = review.status === 'running' ? LoaderCircle : review.status === 'agree' ? CircleCheck : review.status === 'concerns' ? ShieldAlert : ShieldQuestion
  return <div className={cn('second-opinion', review.status)} role="status" aria-live="polite">
    <div className="second-opinion-head"><Icon size={15} className={review.status === 'running' ? 'animate-spin' : undefined} aria-hidden /><strong>{title}</strong></div>
    {review.summary && <p>{review.summary}</p>}
    {review.notes.length > 0 && <ul>{review.notes.map(note => <li key={note}>{note}</li>)}</ul>}
  </div>
}
