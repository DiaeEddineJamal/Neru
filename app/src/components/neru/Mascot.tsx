import { useCallback, useEffect, useRef, useState } from 'react'

type Reaction = 'still' | 'look-left' | 'look-right' | 'twinkle' | 'hop' | 'sway'
type MascotProps = { size?: number; className?: string; interactive?: boolean }

const clickReactions: Exclude<Reaction, 'still'>[] = ['look-left', 'look-right', 'twinkle', 'hop', 'sway']

export function Mascot({ size = 42, className = '', interactive = false }: MascotProps) {
  const [reaction, setReaction] = useState<Reaction>('still')
  const lastReaction = useRef<Reaction>('still')
  const resetTimer = useRef<number | undefined>(undefined)

  const animate = useCallback(() => {
    const available = clickReactions.filter(choice => choice !== lastReaction.current)
    const next = available[Math.floor(Math.random() * available.length)] ?? clickReactions[0]
    lastReaction.current = next
    window.clearTimeout(resetTimer.current)
    setReaction(next)
    resetTimer.current = window.setTimeout(() => setReaction('still'), 1450)
  }, [])

  useEffect(() => () => window.clearTimeout(resetTimer.current), [])

  const figure = (
    <span className={`mascot is-${reaction} ${className}`} style={{ width: size, height: size }} role="img" aria-label="Neru spirit">
      <span className="mascot-figure">
        <img className="mascot-art" src="/neru-mascot-cutout.png" alt="" aria-hidden="true" />
        <svg className="mascot-face" viewBox="0 0 1254 1254" aria-hidden="true">
          <ellipse className="mascot-eye" cx="718" cy="668" rx="37" ry="88" />
          <ellipse className="mascot-eye" cx="881" cy="656" rx="37" ry="83" />
          <path className="mascot-sparkle" d="M 1010 402 L 1017 422 L 1037 429 L 1017 436 L 1010 456 L 1003 436 L 983 429 L 1003 422 Z" />
        </svg>
      </span>
    </span>
  )

  if (!interactive) return figure

  return (
    <button
      type="button"
      className="mascot-control"
      style={{ width: size, height: size }}
      onClick={animate}
      aria-label="Animate Neru mascot"
    >
      {figure}
    </button>
  )
}
