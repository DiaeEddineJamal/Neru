# Neru and Claude Code: what matches and what doesn't

Audited from the source for 0.5.0; updated after the sandbox, sub-agent and free-model work that follows 0.6.0. Neru aims to work like Claude Code's desktop app and CLI, with any model provider.

## Matches Claude Code

- **Tools**: read, write, multi-edit, delete, move; glob, grep, symbols and a project map; shell commands in the foreground or background (`shell_output`, `kill_shell`); web search and fetch with citations; a to-do list; sub-agents in parallel, including custom agents from `.neru/agents` and `.claude/agents` that may edit and run commands under the session's permissions; `ask_user_question`; plan mode that ends with `exit_plan_mode` and a plan to approve; images in the app.
- **Permissions**: review, accept edits, plan, auto and bypass modes; always-allow; an OS sandbox for commands (bubblewrap, sandbox-exec, Windows job objects) behind a rules gate; `permissions.allow/ask/deny` rules from `.claude/settings.json`, `.claude/settings.local.json`, `.neru/settings.json` and `~/.claude/settings.json` (a project's allow rules only after the folder is trusted).
- **Hooks**: `.neru/hooks.json`, the personal `hooks.json`, and Claude's `hooks` block in project settings (after trust). Events: PreToolUse, PostToolUse, UserPromptSubmit, SessionStart, SessionEnd, Stop, SubagentStop, PreCompact, Notification; exit code 2 or JSON decisions.
- **Instructions**: `~/.claude/CLAUDE.md`, parent folders, `AGENTS.md`, `CLAUDE.md`, `.claude/CLAUDE.md`, `CLAUDE.local.md`, nested files as Neru reads them, `@path` imports, `#` memory in the CLI.
- **Sessions**: resume, fork, rename, rewind with code restore, automatic and manual compaction, export, a context meter, parallel sessions in Git worktrees.
- **Commands**: custom commands from `.claude/commands` and `.neru/commands` with front matter (`model`, `allowed-tools`), `$ARGUMENTS`, `$1…$9`, `@file`, `` !`cmd` `` and folder namespaces; skills; output styles.
- **MCP**: stdio and Streamable HTTP, OAuth, per-project `.mcp.json` (after trust).
- **Desktop**: review pane, pull requests and CI checks, live preview with annotations, terminals, file tree, notifications, command palette, attachments, voice, auto-updates.
- **CLI**: `-p`, `-c`, `-r`, `--permission-mode`, `--dangerously-skip-permissions`, `--model`, `--effort`, `--output-format text|json|stream-json` (Claude Code's message shapes), `--append-system-prompt`, `--system-prompt`, `--add-dir`, `--session-id`, `--allowedTools`, `--disallowedTools`, `--max-turns`, `--fallback-model`, `--cwd`, each also as `--flag=value`; `NERU_API_KEY` for CI; `login`, `logout`, `models`, `mcp` (with `--transport`, `--scope`, `add-json`), `sessions`, `doctor`, `config`, `update`; `!` shell, `#` memory, `@` files, shift+tab, esc esc, ctrl+r, `?`; 43 built-in slash commands, including `/compact <instructions>`, `/output-style`, `/statusline`, `/trust` and `/untrust`. Installs with npm or the official scripts, on Windows, macOS and Linux (x64 and arm64).

- **Team (beyond Claude Code)**: Claude Code, Codex, OpenCode, Gemini CLI and Cursor Agent share one thread on their own subscriptions, hand work to each other, and route around limits; imports bring over chats, skills, MCP servers and instructions from those tools, the Claude app, Cursor and VS Code. Every feature Traycer documents that runs locally is either done or tracked as ❌ in TRAYCER_AUDIT.md; none is left half-built. 15 coding CLIs, a message queue, Auto access, browser tools over MCP, smart execution, per-task terminals and PR checks.

## Not yet

| Gap | Size |
| --- | --- |
| MCP resources and prompts (only tools are used); legacy SSE transport | M |
| `--settings`, `--mcp-config`, `--input-format stream-json`, `--include-partial-messages`; token usage and cost in `-p` output | S–M each |
| winget and a Homebrew tap (manifests are generated; the first submission and the tap repository are not done) | S |
| `/vim` mode, image paste in the CLI; `/output-style` in the app | S–M |
| Notebook editing | M |
| Customizable keyboard shortcuts | M |
| The CLI on Linux without WebKitGTK (it runs headless through Xvfb today) | L |
| Long shell commands moved to the background after the 3-minute limit | M |
| Team: cloud sync and multiplayer, remote hosts | L |
| A sandbox that limits writes on Windows (today: a job object; per-folder limits need AppContainer or a low-integrity token) | L |
