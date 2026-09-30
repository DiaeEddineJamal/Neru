import type { Terminal } from '@xterm/xterm'

/** Neru's terminal welcome: the moss mascot in block characters, drawn in and blinking once or twice. */

const MOSS = '\x1b[38;2;122;154;128m'
const MOSS_DEEP = '\x1b[38;2;92;120;98m'
const SHADOW = '\x1b[38;2;58;61;55m'
const CREAM = '\x1b[38;2;242;240;233m'
const EYE_CLOSED = '\x1b[38;2;242;240;233;48;2;122;154;128m'
const DIM = '\x1b[38;2;140;144;136m'
const BORDER = '\x1b[38;2;100;128;106m'
const BOLD = '\x1b[1m'
const RESET = '\x1b[0m'

// M body, D shaded edge, E eye, S shadow. Leans right like the mascot, with its ground shadow.
const ART = [
  '            ▗▄▄▄▖    ',
  '         ▗▟██████▙   ',
  '       ▗▟█████████▌  ',
  '     ▗▟███EE███EE██▙ ',
  '   ▗▟███████████████▖',
  ' ▗▟█████████████████▌',
  ' ▝DDDDDDDDDDDDDDDDDD▘',
  '   SSSSSSSSSSSSSSSS  ',
]
const EYE_ROW = 3
export const ART_WIDTH = ART[0].length

function paint(line: string, closed = false) {
  let out = ''
  for (const char of line) {
    if (char === 'E') out += closed ? `${EYE_CLOSED}▁${RESET}` : `${CREAM}█${RESET}`
    else if (char === 'D') out += `${MOSS_DEEP}▀${RESET}`
    else if (char === 'S') out += `${SHADOW}░${RESET}`
    else if (char === ' ') out += ' '
    else out += `${MOSS}${char}${RESET}`
  }
  return out
}

// Terminal cells: CJK characters such as 練る take two.
const visible = (text: string) => [...text.replace(/\x1b\[[0-9;]*m/g, '')].reduce((cells, char) => cells + (/[　-鿿豈-﫿]/.test(char) ? 2 : 1), 0)
const pad = (text: string, width: number) => text + ' '.repeat(Math.max(0, width - visible(text)))
const wait = (ms: number) => new Promise(resolve => window.setTimeout(resolve, ms))

function shorten(path: string, width: number) {
  return path.length <= width ? path : `…${path.slice(path.length - width + 1)}`
}

/** Text lines beside the mascot. */
function details(project: string, width: number) {
  return [
    '',
    `${MOSS}✻${RESET} ${BOLD}${CREAM}Welcome to Neru${RESET}`,
    `${DIM}練る · think, build, refine${RESET}`,
    '',
    `${DIM}cwd:${RESET} ${shorten(project || '~', width - 5)}`,
    `${DIM}shell:${RESET} PowerShell · Neru's agent commands appear here`,
    '',
    '',
  ]
}

/**
 * Writes the banner. Returns when the draw-in (and blinks) finish, so shell output can follow.
 * Narrow terminals get the mascot and a title without the box.
 */
export async function playBanner(terminal: Terminal, project: string, animate: boolean) {
  const cols = terminal.cols
  const step = animate ? 28 : 0
  const write = (text: string) => new Promise<void>(resolve => terminal.write(text, resolve))

  if (cols < ART_WIDTH + 34) {
    for (const line of ART) { await write(` ${paint(line)}\r\n`); if (step) await wait(step) }
    await write(`\r\n ${MOSS}✻${RESET} ${BOLD}${CREAM}Welcome to Neru${RESET}\r\n\r\n`)
    return
  }

  const inner = Math.min(cols - 4, 92)
  const textWidth = inner - ART_WIDTH - 4
  const side = details(project, textWidth)
  await write(`${BORDER}╭${'─'.repeat(inner)}╮${RESET}\r\n`)
  const row = (art: string, text: string) => `${BORDER}│${RESET} ${paint(art)}   ${pad(text, textWidth)}${BORDER}│${RESET}`
  for (let index = 0; index < ART.length; index += 1) {
    await write(`${row(ART[index], side[index] ?? '')}\r\n`)
    if (step) await wait(step)
  }
  await write(`${BORDER}╰${'─'.repeat(inner)}╯${RESET}\r\n`)

  const tips = [
    `${DIM}Tips for getting started:${RESET}`,
    ` ${MOSS}1.${RESET} Ask Neru to build or fix something from the Code view`,
    ` ${MOSS}2.${RESET} Commands it runs, and their output, stream into this terminal`,
    ` ${MOSS}3.${RESET} Type here as in any PowerShell session`,
  ]
  for (const tip of tips) { await write(`  ${tip}\r\n`); if (step) await wait(step * 1.5) }
  await write('\r\n')

  if (!animate) return
  // Blink: rewrite the eye row in place, counted up from the cursor.
  const up = ART.length - EYE_ROW + 1 + tips.length + 1
  const eyes = (closed: boolean) => write(`\x1b7\x1b[${up}A\r${BORDER}│${RESET} ${paint(ART[EYE_ROW], closed)}\x1b8`)
  for (const pause of [260, 1200]) {
    await wait(pause)
    await eyes(true); await wait(120); await eyes(false)
  }
}
