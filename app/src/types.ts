export type Section = 'home' | 'explorer' | 'search' | 'git' | 'terminal' | 'preview' | 'settings' | 'team'

export interface ProjectInfo {
  name: string
  path: string
  git: boolean
}

export interface FileEntry {
  name: string
  path: string
  isDir: boolean
  size: number
}

export interface SearchHit {
  path: string
  line: number
  /** Match position in `preview` (characters) and its length. */
  column: number
  length: number
  preview: string
}

/** The Search view's answer: the hits, and how much of the project the index had to read. */
export interface SearchResults { hits: SearchHit[]; filesScanned: number; totalFiles: number; truncated: boolean; ms: number }

/** Where the project index stands (the Explorer status line). */
export interface IndexStatus { root: string; state: 'indexing' | 'ready' | 'error'; files: number; total: number; symbols: number; bytes: number; ms: number; scans: number; updatedAt: number }

/** What a `workspace://changed` event carries: every path that changed, and the subset found by a rescan (edits made outside Neru). */
export interface WorkspaceChange { paths: string[]; external?: string[] }

export interface GitFile {
  status: string
  path: string
  staged: boolean
}

export interface GitStatus {
  branch: string
  files: GitFile[]
}

/** A multiple-choice question from the agent's ask_user_question tool. */
export interface AgentQuestion { question: string; header: string; options: { label: string; description: string }[]; multiSelect: boolean }

export interface PendingView {
  kind: 'edit' | 'delete' | 'move' | 'mkdir' | 'task' | 'plan' | 'question'
  label: string
  /** An edit's diff, or a plan's markdown. */
  diff: string | null
  questions?: AgentQuestion[]
  /** Kept on a settled card: what the user answered or asked to change. */
  answers?: string[]
  feedback?: string
}

/** One rate limit on the API key, as the provider last reported it. */
export interface Quota { label: string; limit: number | null; remaining: number | null; resetsIn: string | null }

export interface ContextUsage {
  used: number
  /** The model's context window. */
  window: number
  /** Most one request may use: the window, or a smaller per-minute cap on this key. */
  limit?: number
  measured?: boolean
  /** The window came from the provider's model list rather than a guess by name. */
  windowReported?: boolean
  model?: string
  quotas?: Quota[]
}

export interface AgentResponse {
  sessionId: string
  content: string
  steps: string[]
  pending: PendingView | null
  sources: Source[]
  context: ContextUsage
}

export type DocumentKind = 'text' | 'image' | 'element' | 'pdf' | 'docx' | 'pptx' | 'xlsx' | 'odt' | 'odp' | 'ods' | 'rtf'

/** A document the user attached from outside the project. PDF and Word files carry extracted text; images carry a data URL. */
export interface AttachedDocument { name: string; path: string; size: number; kind: DocumentKind; text?: string; dataUrl?: string }

export interface RewindResult { snapshot: SessionSnapshot; prompt: string; contextPaths: string[]; restored: string[] }

export interface RemoteInfo { branch: string; base: string; remote: string | null; ahead: number | null; behind: number | null; github: string | null; web: string | null; ghCli: boolean }

export interface McpTool { name: string; description: string; readOnly: boolean }
export interface McpServer { name: string; command: string; args: string[]; env: Record<string, string>; url?: string; headers: Record<string, string>; enabled: boolean; status: 'connected' | 'starting' | 'error' | 'off' | 'needs_auth' | 'needs_trust'; signedIn: boolean; error: string | null; tools: McpTool[]; source?: 'user' | 'project' }
/** Whether the open project folder is trusted, and what trusting it would turn on. */
export interface TrustStatus { trusted: boolean; mcpServers: string[]; hookEvents: string[] }

export type Effort = 'auto' | 'low' | 'medium' | 'high'

/** How much Neru may do without asking. */
export type AgentMode = 'manual' | 'accept_edits' | 'plan' | 'auto' | 'bypass'

export interface SessionChange { path: string; diff: string; additions: number; deletions: number; status: 'added' | 'modified' | 'deleted' }

export interface SlashCommand { name: string; description: string; template: string; source: 'project' | 'personal' | 'built-in' | 'skill'; argumentHint?: string; model?: string; allowedTools?: string[] }

export interface CiCheck { name: string; state: 'pending' | 'success' | 'failure' | 'skipped'; url: string | null }
export interface PrStatus { number: number; url: string; title: string; state: string; checks: CiCheck[] }

export interface Source {
  id: string
  title: string
  url: string
  domain: string
}

export interface ChatEntry {
  id: string
  /** A note is a session event (a model switch, a compaction) shown as a quiet line. */
  role: 'user' | 'assistant' | 'note'
  content: string
  steps?: string[]
  contextPaths?: string[]
  /** Images sent with a user message (data URLs). */
  images?: string[]
  sources?: Source[]
  thinking?: string
  /** Files the agent wrote in this reply (kept in memory for the open session). */
  files?: { id: string; path: string; content: string; edit?: boolean; state?: 'writing' | 'done' | 'pending' | 'error' }[]
}

export type ToolEventStatus = 'running' | 'done' | 'error' | 'pending'

export type AgentEvent = { sessionId: string } & (
  | { type: 'delta'; text: string }
  | { type: 'tool'; id: string; label: string; status: ToolEventStatus }
  | { type: 'sources'; sources: Source[] }
  | { type: 'status'; running: boolean }
  | { type: 'notice'; text: string }
  | { type: 'context'; usage: ContextUsage }
  | { type: 'provider'; providerId: string; model: string }
  | { type: 'draft'; id: string; path: string; content: string; tool: string; append?: boolean }
  | { type: 'reasoning'; chars: number; text: string }
  | { type: 'rewind'; text: string; drafts: string[]; thinking: number }
  | { type: 'todos'; todos: Todo[] }
  | { type: 'steered'; text: string }
  | ({ type: 'subagent' } & SubagentProgress))

/** Live state of one sub-agent, sent while it works and shown nested under its `task` tool call. */
export interface SubagentProgress { id: string; role: string; description: string; status: 'running' | 'done'; tools: number; rounds: number; elapsedMs: number; current: string; steps: string[]; approval?: SubagentApproval }

/** An edit or command a sub-agent waits on the user for, answered with api.answerSubagentApproval. */
export interface SubagentApproval { requestId: string; kind: string; label: string; diff?: string | null }

export interface Todo { content: string; status: 'pending' | 'in_progress' | 'completed' }

export interface VoiceView {
  baseUrl: string
  model: string
  hasKey: boolean
  usesChatProvider: boolean
}

export interface SessionSummary {
  id: string
  title: string
  /** Named by the model or the user; not replaced again. */
  titled?: boolean
  projectPath: string
  updatedAt: number
  worktree?: { path: string; branch: string; base: string }
  running?: boolean
}

export interface SessionSnapshot {
  session: SessionSummary
  messages: ChatEntry[]
  pending: PendingView | null
  pendingFromAgent: boolean
  todos?: Todo[]
}

export interface ProviderView {
  providerId: string
  apiFormat: import('./providerCatalog').ApiFormat
  baseUrl: string
  model: string
  configured: boolean
  hasKey: boolean
}

/** Where the `neru` terminal command stands, as Settings → CLI shows it. */
export interface CliStatus { onPath: boolean; path: string | null; version: string | null; bundled: string | null; platform: string; note: string | null }

/** A skill the agent can load, as Settings lists it. */
export interface SkillView { name: string; description: string; source: 'personal' | 'project' | 'claude' | 'built-in'; path: string; chars: number; enabled: boolean }

/** A custom sub-agent from .neru/agents, .claude/agents or the personal agents folder. */
export interface AgentView {
  name: string
  description: string
  source: 'project' | 'claude' | 'personal'
  /** Neru tools it gets (sub-agents only read). */
  tools: string[]
  /** Tools its file lists that a sub-agent cannot have. */
  ignoredTools: string[]
  model: string | null
  path: string
}

/** A command the agent left running in the background. */
export interface ShellView {
  id: string
  sessionId: string
  command: string
  status: 'running' | 'exited' | 'killed'
  exitCode: number | null
  /** Like "running for 42s". */
  detail: string
  elapsedSecs: number
  lines: number
}

/** One model a provider offers, classified from the provider's own metadata or, failing that, its name. */
export interface ModelInfo {
  id: string
  name?: string
  /** What the model is mostly for. */
  kind: 'chat' | 'reasoning' | 'code'
  modalities: { input: string[]; output: string[] }
  /** Reads images. */
  vision: boolean
  /** Takes tool definitions, which Neru needs to edit files and run commands. */
  tools: boolean
  reasoning: boolean
  contextWindow?: number
  free: boolean
  /** ok: a test request worked. unavailable: the provider refused the model. unknown: not checked, or the check was inconclusive. */
  verified: 'ok' | 'unavailable' | 'unknown'
  verifiedReason?: string
  /** metadata: the provider said what the model can do. name: guessed from the id. */
  source: 'metadata' | 'name'
}

export interface ProbeResult { model: string; status: 'ok' | 'unavailable' | 'unknown' | 'badKey'; reason: string; rateLimited: boolean; cached: boolean }

export interface CheckProgress { runId: string; done: number; total: number; result: ProbeResult | null; finished: boolean; note: string | null }

export interface CheckSummary { checked: number; total: number; ok: number; unavailable: number; unknown: number; cancelled: boolean; note: string | null }

/** A coding-agent CLI Neru can run as a Team member on the user's own subscription. */
export interface TeamAgent { kind: string; name: string; path: string | null; version: string | null; signedIn: boolean; login: string; install: string; resumes: boolean; customPath?: boolean; npm?: string | null }
export interface TeamLimit { label: string; used: number; resetsAt: number | null }
export interface TeamSpend { at: number; input: number; output: number; cost: number }
export interface TeamUsage { input: number; output: number; cost: number; turns: number; limits: TeamLimit[]; history?: TeamSpend[] }
export type TeamMemberStatus = 'idle' | 'working' | 'failed' | 'stopped' | ''
export interface TeamMember { handle: string; kind: string; model: string; mode: string; upstream: string | null; seen: number; status: TeamMemberStatus; error: string | null; usage: TeamUsage; worktree: { path: string; branch: string; base: string } | null; forkNext?: boolean }
export interface TeamChanges { root: string; before: string; after: string; files: { path: string; status: string }[]; undone: boolean }
export interface TeamPost { id: string; author: string; to: string[]; text: string; steps: string[]; at: number; kind: 'message' | 'notice' | 'error' | 'side' | 'setup' | 'queued'; changes?: TeamChanges; status?: 'running' | 'ok' | 'failed' }
export interface TeamQueued { id: string; text: string; to: string[]; at: number }
export interface TeamExecution { executor: string; reviewer: string; maxRounds: number; running: boolean }
export interface TeamTask { id: string; title: string; projectPath: string; createdAt: number; updatedAt: number; members: TeamMember[]; posts: TeamPost[]; routeOnLimit: boolean; pinned: boolean; labels: string[]; folder: string; group: string; icon: string; color: string; queue: TeamQueued[]; queuePaused: boolean; execution: TeamExecution }
export interface TeamTaskSummary { id: string; title: string; projectPath: string; updatedAt: number; createdAt: number; running: boolean; pinned: boolean; labels: string[]; members: { handle: string; kind: string; status: TeamMemberStatus }[]; group: string; icon: string; color: string; failed: boolean; posts: number; queued: number }
export interface TeamTaskFile { path: string; diff: string; additions: number; deletions: number; status: string; place: string }
export interface TeamTreeInfo { handle: string; path: string; branch: string; dirty: number; ahead: number }
export interface AgentCommand { agent: string; name: string; description: string; body: string }
export interface TerminalLaunch { cwd: string; program: string; args: string[]; env: [string, string][]; title: string }
export interface TeamArtifact { path: string; size: number; modified: number; versions: string[]; kind: string | null; status: string | null; title: string | null }
export interface TeamSearchHit { taskId: string; taskTitle: string; postId: string; author: string; snippet: string; at: number }
export interface CustomAgent { kind: string; name: string; path: string }
export type TeamEvent = { taskId: string } & (
  | { type: 'member'; member: TeamMember }
  | { type: 'live'; handle: string; text: string; steps: string[] }
  | { type: 'post'; post: TeamPost }
  | { type: 'routing'; from: string; to: string; seconds: number; moved?: boolean }
  | { type: 'error'; error: string }
  | { type: 'queued'; handle: string; queued: boolean }
  | { type: 'idle'; summary: TeamTaskSummary }
  | { type: 'queue'; queue: TeamQueued[]; paused: boolean }
  | { type: 'setup'; post: TeamPost }
  | { type: 'execution'; running: boolean }
)
export interface ImportSource { id: string; name: string; kind: 'agent' | 'app' | 'editor'; found: boolean; path: string; chats: number; skills: number; servers: number; rules: number; note: string | null }
export interface ImportChat { key: string; source: string; cwd: string; title: string; updated: number; messages: number; imported: boolean; unreadable: boolean; reason: string | null }
export interface ImportProgress { done: number; total: number; title: string }
export interface ImportSkill { source: string; name: string; path: string; description: string; conflict: boolean }
export interface ImportServer { key: string; source: string; name: string; target: string; conflict: boolean }
export interface ImportRules { source: string; path: string; scope: 'personal' | 'project'; size: number; imported: boolean }
export interface ImportScan { sources: ImportSource[]; chats: ImportChat[]; skills: ImportSkill[]; servers: ImportServer[]; rules: ImportRules[] }
