export type Section = 'home' | 'explorer' | 'search' | 'git' | 'terminal' | 'preview' | 'settings' | 'team' | 'board'

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
  /** A second model's review of an edit, from a different model family. */
  review?: Review
  /** Auto mode's risk verdict on a command it did not run on its own. */
  risk?: { risk: 'low' | 'medium' | 'high'; reason: string }
}

/**
 * A second opinion on a proposed change, from a model of another family than the one that wrote it.
 * `running` while it reads; `agree` or `concerns` with its notes; `skipped` when no other model is available.
 */
export interface Review {
  status: 'running' | 'agree' | 'concerns' | 'skipped' | 'failed'
  /** The reviewing model, e.g. "gemini-3.8-flash". */
  reviewer: string
  /** One line: what the reviewer thinks. */
  summary: string
  notes: string[]
}

/** Tokens Neru used: on free keys, local and on-device models (`free`) and on paid keys (`paid`), since `since`. */
export interface UsageTotals {
  /** `cached`: input tokens the provider served from its prompt cache (part of `input`). */
  free: { input: number; output: number; replies: number; cached?: number }
  paid: { input: number; output: number; replies: number; cached?: number }
  since: number
}

/** How Neru stretches free quotas. `architectEditor`: a strong model plans each change and a faster one writes it. */
export interface EngineSettings { architectEditor: 'auto' | 'on' | 'off' }

/** A task waiting to run on the desktop, or one that ran. `at` is when it starts (epoch ms). */
export interface QueuedTask {
  id: string
  title: string
  prompt: string
  projectPath: string
  at: number
  createdAt: number
  mode: string
  /** `waiting`: it ran and stopped at a change or command for you to approve. */
  status: 'queued' | 'running' | 'waiting' | 'done' | 'failed' | 'canceled'
  sessionId?: string
  error?: string
  /** Photos sent with it. */
  images: number
  /** Runs from the operating system's scheduler, even when Neru is closed. */
  offline?: boolean
  repeat?: 'daily' | 'weekly'
}

/** One `.neru/checks` file's outcome. */
export interface CheckResult {
  name: string
  description: string
  status: 'pass' | 'fail' | 'skipped' | 'error' | 'idle'
  findings: string[]
}

/** One session's work while you were away, for the morning summary. */
export interface DigestItem {
  sessionId: string
  title: string
  project: string
  projectPath: string
  status: 'done' | 'waiting' | 'failed' | 'running'
  /** The agent's last reply, shortened. */
  summary: string
  /** Changes applied, commands run, and commands that failed since `Digest.since`. */
  edits: number
  commands: number
  failedCommands: number
  /** Started from the task queue. */
  queued: boolean
  updatedAt: number
}

export interface Digest { since: number; items: DigestItem[]; usage: UsageTotals }

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
export interface McpResource { uri: string; name: string; description: string; mimeType?: string | null }
export interface McpPromptArgument { name: string; description: string; required: boolean }
export interface McpPrompt { name: string; description: string; arguments: McpPromptArgument[] }
/** A resource of a running connector, for @-mentions. */
export interface ServerResource extends McpResource { server: string }
/** A prompt of a running connector, run as the slash command `command`. */
export interface ServerPrompt extends McpPrompt { server: string; command: string }
export interface McpServer { name: string; command: string; args: string[]; env: Record<string, string>; url?: string; headers: Record<string, string>; enabled: boolean; transport?: 'sse' | null; resources?: McpResource[]; prompts?: McpPrompt[]; status: 'connected' | 'starting' | 'error' | 'off' | 'needs_auth' | 'needs_trust'; signedIn: boolean; error: string | null; tools: McpTool[]; source?: 'user' | 'project' }
/** Whether the open project folder is trusted, and what trusting it would turn on. */
export interface TrustStatus { trusted: boolean; mcpServers: string[]; hookEvents: string[] }

export type Effort = 'auto' | 'low' | 'medium' | 'high'

/** How much Neru may do without asking. */
export type AgentMode = 'manual' | 'accept_edits' | 'plan' | 'spec' | 'auto' | 'bypass'

export interface SessionChange { path: string; diff: string; additions: number; deletions: number; status: 'added' | 'modified' | 'deleted' }

export interface SlashCommand { name: string; description: string; template: string; source: 'project' | 'personal' | 'built-in' | 'skill' | 'mcp'; argumentHint?: string; model?: string; allowedTools?: string[] }

export interface CiCheck { name: string; state: 'pending' | 'success' | 'failure' | 'skipped'; url: string | null }
export interface PrStatus { number: number; url: string; title: string; state: string; checks: CiCheck[] }

export interface Source {
  id: string
  title: string
  url: string
  domain: string
  /** Scholarly sources (Research mode) also carry what a bibliography needs. */
  authors?: string[]
  year?: number
  venue?: string
  doi?: string
}

export type CitationStyle = 'apa' | 'mla' | 'chicago' | 'ieee' | 'harvard'
/** A step of Research mode's plan, as research.rs reports it. */
export interface ResearchStep { label: string; status: 'pending' | 'running' | 'done' | 'error'; detail: string }

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
  | { type: 'research'; steps: ResearchStep[] }
  | { type: 'steered'; text: string }
  | ({ type: 'subagent' } & SubagentProgress)
  /** Added: a second model's review of the edit waiting for approval. */
  | { type: 'review'; review: Review }
  /** The agent saved a reviewable document (plan, walkthrough, spec). */
  | { type: 'document'; document: SessionDocument })

/** A Markdown document the agent wrote for review in this session. */
export interface SessionDocument { name: string; title: string; size: number; modified: number; done: number; total: number }
/** An answer from beside the main conversation: /btw, or one column of a compare. */
export interface SideAnswer { text: string; providerId: string; model: string; ms: number }
export interface OutputStyle { name: string; description: string; source: string; current: boolean }

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
  /** Where it stands on the session board; filled in when listing. */
  status?: SessionStatus
  /** The pull request for a worktree session's branch, when the board looked it up. */
  pr?: SessionPr
}

export type SessionStatus = 'running' | 'needs-you' | 'done' | 'merged'

export interface SessionPr {
  number: number
  url: string
  /** OPEN, MERGED or CLOSED. */
  state: string
  checks: '' | 'passing' | 'failing' | 'pending'
}

export interface BoardCard {
  session: SessionSummary
  branch: string
  base: string
  dirty: number
  ahead: number
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
/** Who the user is, for every model's system prompt. The About you part can be switched off without losing it. */
export interface Profile { name: string; gender: '' | 'male' | 'female'; occupation: string; about: string; style: string; personalize: boolean }

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
  /** 'text' when the model has no native tool calls and Neru describes the tools in its prompt. */
  protocol?: 'native' | 'text'
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
/** A model a Team member's CLI offers, with the reasoning efforts it takes. */
export interface AgentModel { id: string; label: string; efforts: string[]; defaultEffort: string | null }
export interface TeamMember { handle: string; kind: string; model: string; effort?: string; mode: string; upstream: string | null; seen: number; status: TeamMemberStatus; error: string | null; usage: TeamUsage; worktree: { path: string; branch: string; base: string } | null; forkNext?: boolean }
export interface TeamChanges { root: string; before: string; after: string; files: { path: string; status: string }[]; undone: boolean }
export interface TeamPost { id: string; author: string; to: string[]; text: string; steps: string[]; at: number; kind: 'message' | 'notice' | 'error' | 'side' | 'setup' | 'queued'; changes?: TeamChanges; status?: 'running' | 'ok' | 'failed'; /** Generated images: file names in the task's images/ folder. */ images?: string[] }
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
  | { type: 'live'; handle: string; text: string; steps: string[]; /** Generating an image right now. */ drawing?: boolean }
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

/** Settings → Phone (Neru Remote). `pairing` and `qrSvg` are empty while it is off. */
export interface RemoteStatus { enabled: boolean; port: number; hosts: string[]; name: string; pairing: string; qrSvg: string; clients: number; internet: boolean; endpoint: string; internetError: string }
