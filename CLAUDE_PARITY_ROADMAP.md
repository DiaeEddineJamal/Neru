# Neru and Claude Code: what matches and what doesn't

Audited from the source for 0.5.0. Neru aims to work like Claude Code's desktop app and CLI, with any model provider.

## Matches Claude Code

- **Tools**: read, write, multi-edit, delete, move; glob, grep, symbols and a project map; shell commands in the foreground or background (`shell_output`, `kill_shell`); web search and fetch with citations; a to-do list; sub-agents in parallel, including custom agents from `.neru/agents` and `.claude/agents`; `ask_user_question`; plan mode that ends with `exit_plan_mode` and a plan to approve; images in the app.
- **Permissions**: review, accept edits, plan, auto and bypass modes; always-allow; `permissions.allow/ask/deny` rules from `.claude/settings.json`, `.claude/settings.local.json`, `.neru/settings.json` and `~/.claude/settings.json` (a project's allow rules only after the folder is trusted).
- **Hooks**: `.neru/hooks.json`, the personal `hooks.json`, and Claude's `hooks` block in project settings (after trust). Events: PreToolUse, PostToolUse, UserPromptSubmit, SessionStart, SessionEnd, Stop, SubagentStop, PreCompact, Notification; exit code 2 or JSON decisions.
- **Instructions**: `~/.claude/CLAUDE.md`, parent folders, `AGENTS.md`, `CLAUDE.md`, `.claude/CLAUDE.md`, `CLAUDE.local.md`, nested files as Neru reads them, `@path` imports, `#` memory in the CLI.
- **Sessions**: resume, fork, rename, rewind with code restore, automatic and manual compaction, export, a context meter, parallel sessions in Git worktrees.
- **Commands**: custom commands from `.claude/commands` and `.neru/commands` with front matter, `$ARGUMENTS`, `$1…$9`, `@file` and folder namespaces; skills.
- **MCP**: stdio and Streamable HTTP, OAuth, per-project `.mcp.json` (after trust).
- **Desktop**: review pane, pull requests and CI checks, live preview with annotations, terminals, file tree, notifications, command palette, attachments, voice, auto-updates.
- **CLI**: `-p`, `-c`, `-r`, `--permission-mode`, `--dangerously-skip-permissions`, `--model`, `--effort`, `--output-format text|json|stream-json`, `--append-system-prompt`, `--allowedTools`, `--disallowedTools`, `--max-turns`, `--cwd`; `login`, `logout`, `models`, `mcp`, `sessions`, `doctor`, `config`, `update`; `!` shell, `#` memory, `@` files, shift+tab, esc esc, ctrl+r, `?`; 39 built-in slash commands. Installs with npm, winget or the official scripts.

## Not yet

| Gap | Size |
| --- | --- |
| MCP resources and prompts (only tools are used); legacy SSE transport | M |
| `--add-dir`, `--session-id`, `--settings`, `--mcp-config`, `--input-format stream-json`, `--system-prompt`, `--fallback-model` | S–M each |
| `/output-style`, `/statusline` commands, `/vim` mode, image paste in the CLI | S–M |
| Custom sub-agents that edit or run commands (they are read-only: sub-agents can't ask for approval) | M |
| Custom commands' `model` and `allowed-tools`, and `` !`cmd` `` expansion | S |
| Un-trusting a folder from the UI | S |
| Notebook editing | M |
| Customizable keyboard shortcuts | M |
| The CLI on Linux without a desktop session (it needs WebKitGTK today) | L |
