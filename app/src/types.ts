export type Section = 'home' | 'explorer' | 'search' | 'git' | 'terminal' | 'preview' | 'settings'

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

export interface GitFile {
  status: string
  path: string
  staged: boolean
}

export interface GitStatus {
  branch: string
  files: GitFile[]
}

export interface PendingView {
  kind: 'edit' | 'delete' | 'move' | 'mkdir' | 'task'
  label: string
  diff: string | null
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

export type DocumentKind = 'text' | 'image' | 'pdf' | 'docx' | 'pptx' | 'xlsx' | 'odt' | 'odp' | 'ods' | 'rtf'

/** A document the user attached from outside the project. PDF and Word files carry extracted text; images carry a data URL. */
export interface AttachedDocument { name: string; path: string; size: number; kind: DocumentKind; text?: string; dataUrl?: string }

export interface RewindResult { snapshot: SessionSnapshot; prompt: string; contextPaths: string[]; restored: string[] }

export interface RemoteInfo { branch: string; base: string; remote: string | null; ahead: number | null; behind: number | null; github: string | null; web: string | null; ghCli: boolean }

export interface McpTool { name: string; description: string; readOnly: boolean }
export interface McpServer { name: string; command: string; args: string[]; env: Record<string, string>; url?: string; headers: Record<string, string>; enabled: boolean; status: 'connected' | 'starting' | 'error' | 'off' | 'needs_auth'; signedIn: boolean; error: string | null; tools: McpTool[] }

export type Effort = 'auto' | 'low' | 'medium' | 'high'

/** How much Neru may do without asking. */
export type AgentMode = 'manual' | 'accept_edits' | 'plan' | 'auto' | 'bypass'

export interface SessionChange { path: string; diff: string; additions: number; deletions: number; status: 'added' | 'modified' | 'deleted' }

export interface SlashCommand { name: string; description: string; template: string; source: 'project' | 'personal' | 'built-in' | 'skill' }

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
  role: 'user' | 'assistant'
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
  | { type: 'draft'; id: string; path: string; content: string; tool: string }
  | { type: 'reasoning'; chars: number; text: string }
  | { type: 'todos'; todos: Todo[] }
  | { type: 'steered'; text: string })

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

/** A skill the agent can load, as Settings lists it. */
export interface SkillView { name: string; description: string; source: 'personal' | 'project' | 'claude' | 'built-in'; path: string; chars: number; enabled: boolean }
