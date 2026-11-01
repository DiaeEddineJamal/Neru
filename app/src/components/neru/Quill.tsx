import { cn } from '@/lib/utils'

/** A quill that writes: shown while Neru drafts or edits a document, in the chat and in the panel. */
export function Quill({ className, size = 16 }: { className?: string; size?: number }) {
  return <span className={cn('quill', className)} style={{ width: size, height: size }} aria-hidden>
    <svg viewBox="0 0 24 24" width={size} height={size} fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round">
      <path className="quill-feather" d="M20.2 3.8c-4.6-.5-9.4 1.5-12 5.6-1.5 2.4-2.1 5.1-2 7.8l7.4-7.4M6.2 17.2c2.6.1 5.3-.5 7.6-2 4.1-2.6 6.4-7.3 6.4-11.4M9.4 14.6l4.2-.4" />
      <path className="quill-ink" d="M3.5 20.5c1.6-.9 3-1.1 4.2-.6s2.4.5 3.8-.3" />
    </svg>
  </span>
}
