import type { ReactNode } from 'react'
import { BookOpen, Brain, Bug, Download, FolderOpen, Globe, GraduationCap, Lightbulb, Search } from '@/components/icons'

/**
 * The connector catalog in Settings → Connectors: well-known MCP servers, checked against each
 * vendor's own docs. Hosted ones speak Streamable HTTP (or HTTP + SSE) and sign in with OAuth through
 * dynamic client registration, which is what Neru's sign-in needs; servers that only accept
 * pre-registered apps (Asana v2, Slack's hosted server, Figma's hosted server, Google's Workspace
 * preview servers) cannot sign in from Neru, so they are offered as local programs or left out.
 * Logos come from Simple Icons' CDN, or the brand's own site / Wikimedia Commons.
 */

export type ConnectorCategory = 'research' | 'google' | 'productivity' | 'developer' | 'web' | 'design' | 'data'
export type Auth = 'oauth' | 'token' | 'none'

export const CATEGORIES: { id: ConnectorCategory; label: string }[] = [
  { id: 'research', label: 'Research' },
  { id: 'google', label: 'Google Workspace' },
  { id: 'productivity', label: 'Productivity' },
  { id: 'developer', label: 'Developer' },
  { id: 'web', label: 'Web & search' },
  { id: 'design', label: 'Design' },
  { id: 'data', label: 'Data' },
]

/** An environment variable a local connector reads. */
export interface EnvField {
  key: string
  label: string
  /** Works without it (at lower limits, or with fewer features). */
  optional?: boolean
  /** A value to fill in for the user, such as a mode switch. */
  value?: string
  /** Hide what is typed (keys and tokens). */
  secret?: boolean
}

export interface CatalogEntry {
  id: string
  name: string
  /** One plain line for the card. */
  description: string
  category: ConnectorCategory
  /** Extra words search should match, such as the products a suite covers. */
  keywords?: string[]
  /** Hosted connectors: their endpoint and how they authenticate. */
  url?: string
  auth?: Auth
  /** Set only when the server speaks the older HTTP + SSE transport. */
  transport?: 'http' | 'sse'
  /** Local connectors: the command that starts them and the settings they read. */
  command?: 'npx' | 'uvx'
  args?: string
  env?: EnvField[]
  /** What to do before it can work, shown in the add form. */
  setup?: string
  /** Where to get a key or read the setup steps. */
  setupUrl?: string
  logo?: string
  logoUrl?: string
  glyph?: ReactNode
}

/** What a local connector's command needs on this PC, with the page that installs it. */
export const RUNTIMES: Record<'npx' | 'uvx', { name: string; url: string }> = {
  npx: { name: 'Node.js', url: 'https://nodejs.org/en/download' },
  uvx: { name: 'uv', url: 'https://docs.astral.sh/uv/getting-started/installation/' },
}

export const CATALOG: CatalogEntry[] = [
  // Research
  { id: 'arxiv', name: 'arXiv', description: 'Search arXiv, download papers and read them section by section', category: 'research', keywords: ['papers', 'preprints', 'citations', 'latex'], command: 'uvx', args: 'arxiv-mcp-server', env: [{ key: 'SEMANTIC_SCHOLAR_API_KEY', label: 'Semantic Scholar API key (for citation graphs)', optional: true, secret: true }], setupUrl: 'https://github.com/blazickjp/arxiv-mcp-server', logo: 'arxiv' },
  { id: 'paper-search', name: 'Paper Search', description: 'One search across arXiv, PubMed, bioRxiv, Semantic Scholar, OpenAlex, Crossref and more', category: 'research', keywords: ['papers', 'scholar', 'semantic scholar', 'openalex', 'crossref', 'biorxiv', 'medrxiv', 'doi', 'literature review'], command: 'uvx', args: 'paper-search-mcp', env: [{ key: 'PAPER_SEARCH_MCP_UNPAYWALL_EMAIL', label: 'Email for Unpaywall open-access links', optional: true }, { key: 'PAPER_SEARCH_MCP_SEMANTIC_SCHOLAR_API_KEY', label: 'Semantic Scholar API key', optional: true, secret: true }], setupUrl: 'https://github.com/openags/paper-search-mcp', glyph: <GraduationCap size={18} /> },
  { id: 'pubmed', name: 'PubMed', description: 'Search PubMed and Europe PMC, fetch full text, format citations', category: 'research', keywords: ['biomedical', 'medicine', 'papers', 'mesh', 'citations', 'europe pmc'], command: 'npx', args: '-y @cyanheads/pubmed-mcp-server', env: [{ key: 'NCBI_API_KEY', label: 'NCBI API key (raises the rate limit)', optional: true, secret: true }], setupUrl: 'https://github.com/cyanheads/pubmed-mcp-server', logo: 'pubmed' },
  { id: 'consensus', name: 'Consensus', description: 'Answers from 200M+ peer-reviewed papers', category: 'research', keywords: ['papers', 'evidence', 'literature review', 'science'], url: 'https://mcp.consensus.app/mcp', auth: 'oauth', glyph: <Lightbulb size={18} /> },
  { id: 'scholar-gateway', name: 'Scholar Gateway', description: 'Semantic search over Wiley journals and peer-reviewed research', category: 'research', keywords: ['wiley', 'papers', 'journals', 'literature'], url: 'https://connector.scholargateway.ai/mcp', auth: 'oauth', glyph: <GraduationCap size={18} /> },
  { id: 'zotero', name: 'Zotero', description: 'Search your Zotero library, notes and annotations', category: 'research', keywords: ['references', 'citations', 'bibliography', 'library', 'pdf annotations'], command: 'uvx', args: '--from zotero-mcp-server zotero-mcp', env: [{ key: 'ZOTERO_LOCAL', label: 'Use the Zotero app on this PC', value: 'true' }], setup: 'In Zotero 7 or later, open Settings → Advanced and allow other applications on this computer to communicate with Zotero.', setupUrl: 'https://github.com/54yyyu/zotero-mcp', logo: 'zotero' },
  { id: 'wikipedia', name: 'Wikipedia', description: 'Search and read Wikipedia articles, summaries and sections', category: 'research', keywords: ['encyclopedia', 'facts', 'wiki'], command: 'uvx', args: 'wikipedia-mcp', setupUrl: 'https://github.com/rudra-ravi/wikipedia-mcp', logo: 'wikipedia' },
  { id: 'huggingface', name: 'Hugging Face', description: 'Models, datasets, papers and Spaces', category: 'research', keywords: ['ml', 'ai', 'models', 'datasets', 'papers'], url: 'https://huggingface.co/mcp', auth: 'none', logo: 'huggingface' },

  // Google Workspace
  { id: 'google-workspace', name: 'Google Workspace', description: 'Gmail, Google Calendar, Drive, Docs and Sheets in one connector', category: 'google', keywords: ['gmail', 'email', 'mail', 'calendar', 'google drive', 'drive', 'docs', 'sheets', 'google'], command: 'uvx', args: 'workspace-mcp --tool-tier core', env: [{ key: 'GOOGLE_OAUTH_CLIENT_ID', label: 'OAuth client ID' }, { key: 'GOOGLE_OAUTH_CLIENT_SECRET', label: 'OAuth client secret', secret: true }], setup: 'Google has no sign-in Neru can use directly, so this needs your own Google Cloud OAuth client: create a project, enable the Gmail, Calendar and Drive APIs, then create an OAuth client of type "Desktop app" and paste its ID and secret here. The first tool call opens Google sign-in in your browser.', setupUrl: 'https://github.com/taylorwilsdon/google_workspace_mcp', logo: 'gmail' },

  // Productivity
  { id: 'notion', name: 'Notion', description: 'Search, read and update pages and databases', category: 'productivity', keywords: ['notes', 'wiki', 'docs'], url: 'https://mcp.notion.com/mcp', auth: 'oauth', logo: 'notion' },
  { id: 'linear', name: 'Linear', description: 'Issues, projects and cycles', category: 'productivity', keywords: ['issues', 'tasks', 'tickets'], url: 'https://mcp.linear.app/mcp', auth: 'oauth', logo: 'linear' },
  { id: 'atlassian', name: 'Atlassian', description: 'Jira issues and Confluence pages', category: 'productivity', keywords: ['jira', 'confluence', 'issues', 'wiki'], url: 'https://mcp.atlassian.com/v2/mcp', auth: 'oauth', logo: 'atlassian' },
  { id: 'todoist', name: 'Todoist', description: 'Tasks, projects and due dates', category: 'productivity', keywords: ['tasks', 'todo', 'to-do'], url: 'https://ai.todoist.net/mcp', auth: 'oauth', logo: 'todoist' },
  { id: 'monday', name: 'monday.com', description: 'Boards, items, workflows and dashboards', category: 'productivity', keywords: ['tasks', 'projects', 'crm'], url: 'https://mcp.monday.com/mcp', auth: 'oauth', logoUrl: 'https://upload.wikimedia.org/wikipedia/commons/c/c6/Monday_logo.svg' },
  { id: 'slack', name: 'Slack', description: 'Read channels, threads and DMs; posting stays off unless you turn it on', category: 'productivity', keywords: ['chat', 'messages', 'team'], command: 'npx', args: '-y slack-mcp-server@latest --transport stdio', env: [{ key: 'SLACK_MCP_XOXP_TOKEN', label: 'Slack user token (xoxp-…)', secret: true }], setup: 'Create a Slack app with the user scopes the server lists, install it to your workspace and paste its user OAuth token.', setupUrl: 'https://github.com/korotovsky/slack-mcp-server', logoUrl: 'https://upload.wikimedia.org/wikipedia/commons/d/d5/Slack_icon_2019.svg' },
  { id: 'obsidian', name: 'Obsidian', description: 'Read, search and edit notes in your Obsidian vault', category: 'productivity', keywords: ['notes', 'markdown', 'vault', 'zettelkasten'], command: 'npx', args: '-y obsidian-mcp-server', env: [{ key: 'OBSIDIAN_API_KEY', label: 'Local REST API key', secret: true }], setup: 'Install the "Local REST API" community plugin in Obsidian, turn on its non-encrypted (HTTP) server and copy its API key. Keep Obsidian open while you use it.', setupUrl: 'https://github.com/cyanheads/obsidian-mcp-server', logo: 'obsidian' },
  { id: 'intercom', name: 'Intercom', description: 'Conversations, contacts and help articles', category: 'productivity', keywords: ['support', 'customers'], url: 'https://mcp.intercom.com/mcp', auth: 'oauth', logo: 'intercom' },
  { id: 'zapier', name: 'Zapier', description: 'Thousands of apps through your Zapier actions', category: 'productivity', keywords: ['automation', 'apps'], url: 'https://mcp.zapier.com/api/mcp/mcp', auth: 'oauth', logo: 'zapier' },
  { id: 'paypal', name: 'PayPal', description: 'Invoices, orders and transactions', category: 'productivity', keywords: ['payments', 'invoices'], url: 'https://mcp.paypal.com/mcp', auth: 'oauth', logo: 'paypal' },
  { id: 'square', name: 'Square', description: 'Payments, catalog and customers', category: 'productivity', keywords: ['payments', 'shop'], url: 'https://mcp.squareup.com/sse', auth: 'oauth', transport: 'sse', logo: 'square' },
  { id: 'filesystem', name: 'Filesystem', description: 'Read and search files in folders you choose', category: 'productivity', keywords: ['files', 'folders', 'local'], command: 'npx', args: '-y @modelcontextprotocol/server-filesystem D:\\Neru\\projects', setup: 'The last argument is the folder it may use; change it to yours, and add more folders separated by spaces.', glyph: <FolderOpen size={18} /> },
  { id: 'memory', name: 'Memory', description: 'A small knowledge graph Neru can remember with', category: 'productivity', keywords: ['remember', 'knowledge graph'], command: 'npx', args: '-y @modelcontextprotocol/server-memory', glyph: <Brain size={18} /> },
  { id: 'sequential-thinking', name: 'Sequential Thinking', description: 'Step-by-step reasoning the model can revise and branch', category: 'productivity', keywords: ['reasoning', 'planning', 'think'], command: 'npx', args: '-y @modelcontextprotocol/server-sequential-thinking', glyph: <Lightbulb size={18} /> },

  // Developer
  { id: 'github', name: 'GitHub', description: 'Repos, issues and pull requests (personal access token)', category: 'developer', keywords: ['git', 'code', 'pull requests', 'issues'], url: 'https://api.githubcopilot.com/mcp/', auth: 'token', setupUrl: 'https://github.com/settings/personal-access-tokens', logo: 'github' },
  { id: 'gitlab', name: 'GitLab', description: 'Projects, issues and merge requests', category: 'developer', keywords: ['git', 'code', 'merge requests'], url: 'https://gitlab.com/api/v4/mcp', auth: 'oauth', logo: 'gitlab' },
  { id: 'context7', name: 'Context7', description: 'Up-to-date library docs and code examples', category: 'developer', keywords: ['docs', 'documentation', 'libraries', 'api'], url: 'https://mcp.context7.com/mcp', auth: 'none', glyph: <BookOpen size={18} /> },
  { id: 'deepwiki', name: 'DeepWiki', description: 'Ask questions about any public GitHub repository', category: 'developer', keywords: ['github', 'docs', 'repository'], url: 'https://mcp.deepwiki.com/mcp', auth: 'none', glyph: <Globe size={18} /> },
  { id: 'sentry', name: 'Sentry', description: 'Errors, issues and stack traces', category: 'developer', keywords: ['errors', 'monitoring', 'bugs'], url: 'https://mcp.sentry.dev/mcp', auth: 'oauth', logo: 'sentry' },
  { id: 'stripe', name: 'Stripe', description: 'Customers, payments and products', category: 'developer', keywords: ['payments', 'billing'], url: 'https://mcp.stripe.com', auth: 'oauth', logo: 'stripe' },
  { id: 'vercel', name: 'Vercel', description: 'Projects, deployments and logs', category: 'developer', keywords: ['deploy', 'hosting'], url: 'https://mcp.vercel.com', auth: 'oauth', logo: 'vercel' },
  { id: 'netlify', name: 'Netlify', description: 'Sites, deploys and environment variables', category: 'developer', keywords: ['deploy', 'hosting'], url: 'https://netlify-mcp.netlify.app/mcp', auth: 'oauth', logo: 'netlify' },
  { id: 'cloudflare', name: 'Cloudflare', description: 'Workers, KV, R2 and D1 in your account', category: 'developer', keywords: ['workers', 'hosting', 'edge'], url: 'https://bindings.mcp.cloudflare.com/mcp', auth: 'oauth', logo: 'cloudflare' },
  { id: 'cloudflare-docs', name: 'Cloudflare Docs', description: 'Search the Cloudflare developer docs', category: 'developer', keywords: ['docs', 'documentation'], url: 'https://docs.mcp.cloudflare.com/mcp', auth: 'none', logo: 'cloudflare' },
  { id: 'jam', name: 'Jam', description: 'Bug reports with console logs and repro steps', category: 'developer', keywords: ['bugs', 'qa'], url: 'https://mcp.jam.dev/mcp', auth: 'oauth', glyph: <Bug size={18} /> },
  { id: 'playwright', name: 'Playwright', description: 'Drive a real browser: click, type, screenshot', category: 'developer', keywords: ['browser', 'testing', 'automation', 'web'], command: 'npx', args: '-y @playwright/mcp@latest', logoUrl: 'https://playwright.dev/img/playwright-logo.svg', glyph: <Globe size={18} /> },
  { id: 'git', name: 'Git', description: 'Log, diff, blame and branches for any repository', category: 'developer', keywords: ['version control', 'diff'], command: 'uvx', args: 'mcp-server-git', logo: 'git' },

  // Web & search
  { id: 'exa', name: 'Exa', description: 'Web search and page fetching for agents, no key needed', category: 'web', keywords: ['search', 'web', 'internet'], url: 'https://mcp.exa.ai/mcp', auth: 'none', glyph: <Search size={18} /> },
  { id: 'tavily', name: 'Tavily', description: 'Web search, extract and crawl built for AI answers', category: 'web', keywords: ['search', 'web', 'crawl', 'extract'], url: 'https://mcp.tavily.com/mcp/', auth: 'oauth', glyph: <Search size={18} /> },
  { id: 'firecrawl', name: 'Firecrawl', description: 'Scrape, search and crawl websites into clean Markdown', category: 'web', keywords: ['scrape', 'crawl', 'web', 'search'], url: 'https://mcp.firecrawl.dev/v2/mcp-oauth', auth: 'oauth', glyph: <Globe size={18} /> },
  { id: 'brave', name: 'Brave Search', description: 'Web, news, image and local search with the Brave API', category: 'web', keywords: ['search', 'web', 'news'], command: 'npx', args: '-y @brave/brave-search-mcp-server --transport stdio', env: [{ key: 'BRAVE_API_KEY', label: 'Brave Search API key', secret: true }], setupUrl: 'https://brave.com/search/api/', logo: 'brave' },
  { id: 'fetch', name: 'Fetch', description: 'Fetch web pages as Markdown', category: 'web', keywords: ['web', 'download', 'url'], command: 'uvx', args: 'mcp-server-fetch', glyph: <Download size={18} /> },

  // Design
  { id: 'canva', name: 'Canva', description: 'Search, create and edit designs', category: 'design', keywords: ['slides', 'posters', 'graphics'], url: 'https://mcp.canva.com/mcp', auth: 'oauth', logo: 'canva' },
  { id: 'miro', name: 'Miro', description: 'Read and create boards, diagrams and sticky notes', category: 'design', keywords: ['whiteboard', 'diagrams', 'brainstorm'], url: 'https://mcp.miro.com/', auth: 'oauth', logo: 'miro' },
  { id: 'figma', name: 'Figma', description: 'Layout and styles from Figma files (Framelink, API key)', category: 'design', keywords: ['ui', 'design', 'frames'], command: 'npx', args: '-y figma-developer-mcp --stdio', env: [{ key: 'FIGMA_API_KEY', label: 'Figma personal access token', secret: true }], setupUrl: 'https://www.framelink.ai/docs/quickstart', logo: 'figma' },
  { id: 'webflow', name: 'Webflow', description: 'Sites, CMS collections and pages', category: 'design', keywords: ['website', 'cms'], url: 'https://mcp.webflow.com/mcp', auth: 'oauth', logo: 'webflow' },
  { id: 'wix', name: 'Wix', description: 'Sites, stores and bookings', category: 'design', keywords: ['website', 'store'], url: 'https://mcp.wix.com/mcp', auth: 'oauth', logo: 'wix' },

  // Data
  { id: 'airtable', name: 'Airtable', description: 'Bases, tables and records', category: 'data', keywords: ['spreadsheet', 'database', 'records'], url: 'https://mcp.airtable.com/mcp', auth: 'oauth', logo: 'airtable' },
  { id: 'supabase', name: 'Supabase', description: 'Tables, SQL, migrations and edge functions', category: 'data', keywords: ['postgres', 'sql', 'database'], url: 'https://mcp.supabase.com/mcp', auth: 'oauth', logo: 'supabase' },
  { id: 'neon', name: 'Neon', description: 'Postgres projects, branches and SQL', category: 'data', keywords: ['postgres', 'sql', 'database'], url: 'https://mcp.neon.tech/mcp', auth: 'oauth', logo: 'neon' },
  { id: 'prisma', name: 'Prisma Postgres', description: 'Databases, migrations and queries', category: 'data', keywords: ['postgres', 'sql', 'database', 'orm'], url: 'https://mcp.prisma.io/mcp', auth: 'oauth', logo: 'prisma' },
]

/** Lowercase and strip accents, so "zotéro" finds Zotero. */
const fold = (text: string) => text.normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLowerCase()

/** The entries in a category (or all) whose name, description or keywords contain every search word. */
export function filterCatalog(entries: CatalogEntry[], query: string, category: ConnectorCategory | 'all' = 'all') {
  const words = fold(query).split(/\s+/).filter(Boolean)
  return entries.filter(entry => {
    if (category !== 'all' && entry.category !== category) return false
    const haystack = fold([entry.name, entry.description, entry.id, ...(entry.keywords ?? [])].join(' '))
    return words.every(word => haystack.includes(word))
  })
}

/** Turns a connector's raw error into a plain sentence, keeping the original for the details. */
export function explainConnectorError(raw: string): { message: string; link?: { label: string; url: string } } {
  const text = raw.toLowerCase()
  const runtime = (key: 'npx' | 'uvx') => ({ label: `Install ${RUNTIMES[key].name}`, url: RUNTIMES[key].url })
  // mcp.rs already words a missing program plainly and names the page that installs it.
  if (/\b(npx|npm|node)\b.*(not found|not recognized|is not recognised)/.test(text) || text.includes('nodejs.org'))
    return { message: 'Node.js is not installed on this PC, so npx cannot start this connector. Install Node.js, restart Neru, then reconnect.', link: runtime('npx') }
  if (/\b(uvx|uv)\b.*(not found|not recognized|is not recognised)/.test(text) || text.includes('astral.sh/uv'))
    return { message: 'uv is not installed on this PC, so uvx cannot start this connector. Install uv, restart Neru, then reconnect.', link: runtime('uvx') }
  if (text.includes('not found on this pc') || text.includes('is not recognized as an internal or external command'))
    return { message: 'The program this connector runs is not installed on this PC. Check the command, or install the program it needs.' }
  if (text.includes('does not allow automatic app registration'))
    return { message: 'This service only lets pre-approved apps sign in, so Neru cannot sign in to it. Use an access token in the Headers field if the service offers one.' }
  if (text.includes('could not reach') || text.includes('dns') || text.includes('connection refused') || text.includes('error sending request'))
    return { message: 'Neru could not reach this connector. Check your internet connection and the server URL.' }
  if (text.includes('timed out'))
    return { message: 'The connector took too long to answer. The first start can be slow while packages download; try Reconnect.' }
  if (/answered 40[13]\b/.test(text) || text.includes('sign in again'))
    return { message: 'The connector refused the sign-in or key. Sign in again, or check the token you entered.' }
  if (/answered 404\b/.test(text))
    return { message: 'Nothing answers at this URL. The server address may have changed; check it in Edit.' }
  if (/answered 5\d\d\b/.test(text))
    return { message: 'The connector’s server had a problem. Try again in a few minutes.' }
  if (text.includes('the server stopped'))
    return { message: 'The connector stopped while starting. Check its settings, such as a missing or wrong API key, then reconnect.' }
  return { message: 'The connector could not start.' }
}
