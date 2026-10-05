/**
 * Neru Remote, protocol v1: how the phone app talks to the desktop (src-tauri/src/remote.rs).
 * Plain TypeScript with type-only imports, so the phone can import it through Metro.
 *
 * Transport: a WebSocket to ws://<host>:<port>/ on the desktop's LAN address. Off until the user turns
 * it on in Settings → Phone; the desktop tries DEFAULT_PORT first and falls back to a free port.
 *
 * Framing: every text frame is base64url(nonce || ciphertext), no padding. The cipher is
 * XChaCha20-Poly1305 with the 32-byte pairing key, a fresh random 24-byte nonce per frame and no AAD.
 * The plaintext is UTF-8 JSON: a ClientRequest from the phone, a ServerReply or ServerPush from the
 * desktop. A frame that does not decrypt (wrong key, tampered, binary, not base64url) closes the
 * connection; there is no plaintext fallback. Frames are capped at MAX_FRAME_BYTES. A connection
 * whose first frame has not decrypted within 30 seconds is closed. Pushes start after the first frame
 * decrypts, so send `hello` first.
 *
 * Resetting the pairing on the desktop rotates the key and closes every connection; turning Remote off
 * closes them too.
 */
import type { AgentEvent, AgentMode, AgentModel, SessionSnapshot, TeamAgent, TeamEvent, TeamPost, TeamTask, TeamTaskSummary } from './types'

export const PROTOCOL_VERSION = 1
export const DEFAULT_PORT = 47613
export const MAX_FRAME_BYTES = 8 * 1024 * 1024
export const NONCE_BYTES = 24
export const KEY_BYTES = 32

/**
 * The pairing payload, shown as a QR code and as text:
 * `neru://pair?v=1&h=<comma-separated LAN IPv4s>&p=<port>&k=<base64url key, no padding>&n=<url-encoded computer name>`
 * The address on the default route comes first in `h`; try them in order.
 */
export const PAIRING_SCHEME = 'neru://pair'
export interface Pairing { v: 1; hosts: string[]; port: number; key: string; name: string }

export function formatPairingUrl(pairing: Omit<Pairing, 'v'>): string {
  return `${PAIRING_SCHEME}?v=1&h=${pairing.hosts.join(',')}&p=${pairing.port}&k=${pairing.key}&n=${encodeURIComponent(pairing.name)}`
}

/** Reads a pairing link; null when it is not one this version understands. */
export function parsePairingUrl(text: string): Pairing | null {
  const trimmed = text.trim()
  if (!trimmed.startsWith(`${PAIRING_SCHEME}?`)) return null
  const params: Record<string, string> = {}
  for (const part of trimmed.slice(PAIRING_SCHEME.length + 1).split('&')) {
    const at = part.indexOf('=')
    if (at > 0) params[part.slice(0, at)] = decodeURIComponent(part.slice(at + 1).replace(/\+/g, ' '))
  }
  const port = Number(params.p)
  const hosts = (params.h ?? '').split(',').map(host => host.trim()).filter(Boolean)
  if (params.v !== '1' || !hosts.length || !Number.isInteger(port) || port <= 0 || port > 65535 || !/^[A-Za-z0-9_-]{43}$/.test(params.k ?? '')) return null
  return { v: 1, hosts, port, key: params.k, name: params.n ?? 'Neru' }
}

/** A session in the phone's list. `waiting`: it has a call waiting for approval. Loose chats have project "Chat" and an empty projectPath. */
export interface SessionSummaryForPhone {
  id: string
  title: string
  project: string
  projectPath: string
  updatedAt: number
  running: boolean
  waiting: boolean
}

/** Every command, its args and the `data` of a successful reply. */
export interface RemoteCommands {
  hello: { args: Record<string, never>; result: { name: string; version: string } }
  list_sessions: { args: Record<string, never>; result: SessionSummaryForPhone[] }
  /** The same snapshot the desktop window gets: messages, plus the pending call with its diff, command or plan text. */
  session_snapshot: { args: { sessionId: string }; result: SessionSnapshot }
  /**
   * Steers a running session (read by the agent before its next step), or starts a reply in an idle one.
   * `mode` applies to a reply it starts; default 'manual' (ask every time). Fails while a call is waiting for approval.
   */
  send_prompt: { args: { sessionId: string; text: string; mode?: AgentMode }; result: { steered: boolean } }
  /**
   * The desktop's Approve button: applies the edit, runs the command / task / connector call, or approves the plan,
   * then the agent continues (in `mode`, default 'manual'; for a plan, the mode to build in). `always` also remembers
   * the command for this project, like "Always allow" (not for file edits). Questions use `answer`.
   * Resolves once the call ran; the continued reply streams as agent events.
   */
  approve: { args: { sessionId: string; always?: boolean; mode?: AgentMode }; result: null }
  /** The desktop's Deny button. The agent is told and the run does not continue. */
  deny: { args: { sessionId: string }; result: null }
  /** Added: answers a pending ask_user_question card (one answer per question; several choices joined by ", "), then continues. */
  answer: { args: { sessionId: string; answers: string[]; mode?: AgentMode }; result: null }
  /** Added: approves or denies a sub-agent's edit or command (AgentEvent type 'subagent' with an `approval`). */
  answer_subagent: { args: { requestId: string; approve: boolean }; result: null }
  stop: { args: { sessionId: string }; result: null }
  list_teams: { args: Record<string, never>; result: TeamTaskSummary[] }
  team_snapshot: { args: { id: string }; result: TeamTask }
  /** `to`: member handles; empty sends to the whole team. */
  send_team_message: { args: { id: string; text: string; to: string[] }; result: TeamPost }
  /** Added `handle`: stops one member instead of the whole task. */
  stop_team: { args: { id: string; handle?: string }; result: null }
  /** Added: the agent CLIs on the desktop (installed ones have a `path`). Neru's own agent (kind 'neru') is always available and not listed. */
  list_team_agents: { args: Record<string, never>; result: TeamAgent[] }
  /** Added: models an agent CLI offers; empty when it keeps no list (type a model id, or leave it to the agent default). */
  list_agent_models: { args: { kind: string }; result: AgentModel[] }
  /**
   * Added: starts a team task like the desktop's New task dialog (an empty projectPath is a task with no folder).
   * A non-empty `text` goes out as the first message, to `to` (handles or 'all'; empty: whoever it mentions, else the first member).
   */
  create_team_task: { args: { title: string; projectPath: string; members: { kind: string; model?: string; mode?: string }[]; text?: string; to?: string[] }; result: TeamTask }
}
export type RemoteCommand = keyof RemoteCommands

export type ClientRequest<C extends RemoteCommand = RemoteCommand> = { id: number; cmd: C; args: RemoteCommands[C]['args'] }

export type ServerReply<C extends RemoteCommand = RemoteCommand> =
  | { id: number; ok: true; data: RemoteCommands[C]['result'] }
  | { id: number; ok: false; error: string }

/**
 * Sent without a request. `agent` and `team` carry every desktop event as is. `sessions` is the whole list,
 * sent whenever a run starts or stops (so after it settles into waiting) and after the phone approves, denies or answers.
 */
export type ServerPush =
  | { event: 'agent'; payload: AgentEvent }
  | { event: 'team'; payload: TeamEvent }
  | { event: 'sessions'; payload: SessionSummaryForPhone[] }

export type ServerMessage = ServerReply | ServerPush
export const isPush = (message: ServerMessage): message is ServerPush => 'event' in message
