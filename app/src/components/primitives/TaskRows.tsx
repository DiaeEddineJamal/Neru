/* ─────────────────────────────────────────────────────────
 * TASK ROWS
 *
 * Adapted from Beautiful UI's Task Rows primitive
 * (https://www.beautifului.dev/#task-rows, MIT License, © 2026 Beautiful UI).
 * Kept: the numbered spinner ring, the pop-in check badge, status pills,
 * staggered fade-up rows and the list card. Changed: rows are driven by live
 * agent state instead of a scripted demo, and the foundation tokens map onto
 * Neru's theme (see the .task-rows rules in App.css).
 * ───────────────────────────────────────────────────────── */

import type { ReactNode } from 'react'

export type TaskRowStatus = 'pending' | 'running' | 'done' | 'failed'

export type TaskRow = {
  key: string
  label: string
  status: TaskRowStatus
  /** Shown inside the ring while the task waits or runs; usually its position in the plan. */
  step?: number
  /** Small right-aligned note, like "3 files". */
  amount?: string
}

export type TaskRowsLabels = { completed: string; failed: string; running: string }

const DEFAULT_LABELS: TaskRowsLabels = { completed: 'Completed', failed: 'Failed', running: 'In progress' }

export function SpinnerRing({ active, size = 24, progress, children }: { active?: boolean; size?: number; progress?: number; children?: ReactNode }) {
  const stroke = 2
  const r = (size - stroke) / 2
  const c = 2 * Math.PI * r
  return (
    <span className="task-ring" style={{ width: size, height: size }}>
      <svg width={size} height={size} className={active ? 'task-ring-spin' : undefined} aria-hidden>
        <circle cx={size / 2} cy={size / 2} r={r} fill="none" className="task-ring-track" strokeWidth={stroke} />
        {active && <circle cx={size / 2} cy={size / 2} r={r} fill="none" className="task-ring-head" strokeWidth={stroke} strokeLinecap="round" strokeDasharray={`${c * 0.28} ${c * 0.72}`} />}
        {progress !== undefined && !active && <circle cx={size / 2} cy={size / 2} r={r} fill="none" className="task-ring-fill" strokeWidth={stroke} strokeLinecap="round" strokeDasharray={c} strokeDashoffset={c * (1 - Math.max(0, Math.min(1, progress)))} transform={`rotate(-90 ${size / 2} ${size / 2})`} />}
      </svg>
      <span className="task-ring-label">{children}</span>
    </span>
  )
}

export function TaskBadge({ tone }: { tone: 'green' | 'red' }) {
  return <span className={`task-badge ${tone}`} aria-hidden>{tone === 'red' ? XIcon : CheckIcon}</span>
}

const XIcon = <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3.5" strokeLinecap="round"><path d="M18 6L6 18M6 6l12 12" /></svg>
const CheckIcon = <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3.5" strokeLinecap="round" strokeLinejoin="round"><path d="M20 6L9 17l-5-5" /></svg>

export default function TaskRows({ rows, labels, className }: { rows: TaskRow[]; labels?: Partial<TaskRowsLabels>; className?: string }) {
  const copy = { ...DEFAULT_LABELS, ...labels }
  return (
    <ol className={`task-rows${className ? ` ${className}` : ''}`}>
      {rows.map((row, i) => (
        <li key={row.key} className="task-row" data-status={row.status} style={{ animationDelay: `${Math.min(i, 10) * 60}ms` }}>
          {/* Keying the marker by status replays its entrance when the task changes state. */}
          <span className="task-row-mark" key={row.status}>
            {row.status === 'done' ? <TaskBadge tone="green" />
              : row.status === 'failed' ? <TaskBadge tone="red" />
              : <SpinnerRing active={row.status === 'running'}>{row.step}</SpinnerRing>}
          </span>
          <span className="task-row-label">{row.label}</span>
          {row.amount && <span className="task-row-amount">{row.amount}</span>}
          {row.status === 'done' && <span className="task-pill green">{copy.completed}</span>}
          {row.status === 'running' && <span className="task-pill live">{copy.running}</span>}
          {row.status === 'failed' && <span className="task-pill red">{copy.failed}</span>}
        </li>
      ))}
    </ol>
  )
}
