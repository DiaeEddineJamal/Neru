import { useEffect, useLayoutEffect, useRef } from 'react'
import { getAudioContext } from 'voice-glow'
import { cn } from '@/lib/utils'

type Props = {
  /** Live microphone; the bars follow its loudness. */
  stream?: MediaStream | null
  /** Synthetic 0..1 level, used when there is no stream (onboarding demo). */
  level?: () => number
  /** Freezes the bars where they are. */
  paused?: boolean
  bars?: number
  className?: string
}

const SAMPLE_MS = 70

/**
 * A scrolling row of rounded bars, one per recent moment of speech, newest on the right, as in the Claude
 * desktop app's dictation. Heights are written straight to the DOM each frame so React never re-renders.
 */
export function SoundBars({ stream, level, paused = false, bars = 28, className }: Props) {
  const host = useRef<HTMLSpanElement>(null)
  const pausedRef = useRef(paused)
  const levelRef = useRef(level)
  useLayoutEffect(() => { pausedRef.current = paused; levelRef.current = level })

  useEffect(() => {
    const el = host.current
    if (!el) return
    const nodes = Array.from(el.children) as HTMLElement[]
    const history = new Array<number>(bars).fill(0)
    let analyser: AnalyserNode | null = null
    let source: MediaStreamAudioSourceNode | null = null
    const samples = new Float32Array(1024)
    if (stream) {
      const context = getAudioContext()
      if (context) {
        source = context.createMediaStreamSource(stream)
        analyser = context.createAnalyser()
        analyser.fftSize = 1024
        source.connect(analyser)
      }
    }
    const read = () => {
      if (analyser) {
        analyser.getFloatTimeDomainData(samples)
        let sum = 0
        for (const s of samples) sum += s * s
        // Speech RMS sits around 0.01-0.2; a square-root curve lifts quiet syllables without clipping loud ones.
        return Math.min(1, Math.sqrt(Math.sqrt(sum / samples.length) * 6))
      }
      return Math.min(1, Math.max(0, levelRef.current?.() ?? 0))
    }
    let frame = 0
    let last = 0
    let smooth = 0
    const tick = (now: number) => {
      frame = requestAnimationFrame(tick)
      if (pausedRef.current || now - last < SAMPLE_MS) return
      last = now
      const value = read()
      smooth = value > smooth ? value : smooth * 0.55 + value * 0.45
      history.shift()
      history.push(smooth)
      for (let i = 0; i < nodes.length; i++) nodes[i].style.transform = `scaleY(${Math.max(0.12, history[i]).toFixed(3)})`
    }
    frame = requestAnimationFrame(tick)
    return () => {
      cancelAnimationFrame(frame)
      source?.disconnect()
      analyser?.disconnect()
    }
  }, [stream, bars])

  return <span ref={host} className={cn('sound-bars', paused && 'paused', className)} aria-hidden>
    {Array.from({ length: bars }, (_, i) => <i key={i} />)}
  </span>
}
