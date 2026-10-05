// Connects to Neru Remote like the phone does: node scripts/remote-smoke.mjs "neru://pair?v=1&h=...&p=...&k=...&n=..."
// Copy the link from Settings → Phone. Sends hello and list_sessions, prints the replies, and exits 1 on failure.
import { webcrypto } from 'node:crypto'

const cipherModule = async () => {
  try { return await import('@noble/ciphers/chacha.js') } catch {
    // The app does not depend on @noble/ciphers; the phone app does.
    return import('file:///D:/Neru/mobile/node_modules/@noble/ciphers/chacha.js')
  }
}
const { xchacha20poly1305 } = await cipherModule()

const link = process.argv[2]
if (!link?.startsWith('neru://pair?')) {
  console.error('Usage: node scripts/remote-smoke.mjs "<pairing link from Settings → Phone>"')
  process.exit(2)
}
const query = new URLSearchParams(link.slice('neru://pair?'.length))
const key = Buffer.from(query.get('k') ?? '', 'base64url')
const hosts = (query.get('h') ?? '').split(',').filter(Boolean)
const port = Number(query.get('p'))
if (query.get('v') !== '1' || key.length !== 32 || !hosts.length || !port) throw new Error('Not a complete v1 pairing link')
console.log(`Pairing with ${query.get('n')} at ${hosts.join(', ')} port ${port}`)

const seal = json => {
  const nonce = webcrypto.getRandomValues(new Uint8Array(24))
  return Buffer.concat([nonce, xchacha20poly1305(key, nonce).encrypt(new TextEncoder().encode(json))]).toString('base64url')
}
const open = frame => {
  const bytes = Buffer.from(frame, 'base64url')
  return JSON.parse(new TextDecoder().decode(xchacha20poly1305(key, bytes.subarray(0, 24)).decrypt(bytes.subarray(24))))
}

/** The first host that accepts a WebSocket within 4 seconds, like the phone tries them in order. */
async function connect() {
  for (const host of hosts) {
    const socket = new WebSocket(`ws://${host}:${port}/`)
    const opened = await new Promise(resolve => {
      const timer = setTimeout(() => { socket.close(); resolve(false) }, 4000)
      socket.onopen = () => { clearTimeout(timer); resolve(true) }
      socket.onerror = () => { clearTimeout(timer); resolve(false) }
    })
    if (opened) { console.log(`Connected to ${host}`); return socket }
    console.log(`No answer from ${host}`)
  }
  throw new Error('No address answered. Is Neru Remote on, and is Neru allowed through the firewall on private networks?')
}

const socket = await connect()
const waiting = new Map()
socket.onmessage = event => {
  const message = open(String(event.data))
  if ('event' in message) return console.log(`push: ${message.event}`)
  waiting.get(message.id)?.(message)
}
socket.onclose = event => {
  for (const settle of waiting.values()) settle({ ok: false, error: `Connection closed (${event.code})` })
}
let next = 0
const call = (cmd, args = {}) => new Promise((resolve, reject) => {
  const id = ++next
  const timer = setTimeout(() => reject(new Error(`${cmd} timed out`)), 10000)
  waiting.set(id, reply => { clearTimeout(timer); waiting.delete(id); reply.ok ? resolve(reply.data) : reject(new Error(`${cmd}: ${reply.error}`)) })
  socket.send(seal(JSON.stringify({ id, cmd, args })))
})

try {
  console.log('hello:', await call('hello'))
  const sessions = await call('list_sessions')
  console.log(`list_sessions: ${sessions.length} sessions`)
  for (const session of sessions.slice(0, 5)) console.log(`  ${session.project} · ${session.title}${session.running ? ' (running)' : ''}${session.waiting ? ' (waiting)' : ''}`)
  socket.close()
} catch (error) {
  console.error(String(error))
  socket.close()
  process.exit(1)
}
