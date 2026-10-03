# Neru vs Claude — feature audit (updated 2026-10-02)

Compared against Claude Desktop (Chat + Code tab) and the Claude Code CLI.
Status: **Done** = implemented and wired end to end · **Partial** = works but narrower than Claude · **Missing** = not in Neru.
"Done" is based on reading the code and unit tests, not a full manual test of every feature.

## 1. Agent core

| Feature | Claude | Neru | Notes |
|---|---|---|---|
| Agent loop with streaming text | ✓ | **Done** | Streaming for Chat Completions, Responses and Anthropic formats |
| Live view of files as they are written | ✓ | **Done** | VS Code Dark+/Light+ colors, line numbers, auto-scroll |
| Read / search / find / list tools | ✓ | **Done** | `read_file` (windowed), `search_text`, `find_files`, `list_directory` |
| Edit / write / delete / move / mkdir | ✓ | **Done** | Every change is shown as a diff, with a checkpoint for undo |
| Shell commands | ✓ | **Done** | PowerShell, 3-minute limit, dev-server commands blocked; a rules gate refuses drive and home wipes outright |
| Permission modes | ✓ | **Done** | Review, Plan, Accept edits, Auto, Bypass |
| Always-allow rules, allow/deny lists | ✓ | **Done** | Per project; `.neru/allow.txt` and `.neru/deny.txt`; destructive commands and writes outside the project always ask; Auto runs a command alone only when every part is allowlisted |
| Several tool calls in one turn | ✓ | **Done** | Several calls per turn; sub-agents run in parallel; queued approvals keep generated files |
| Sub-agents (Task / Agent tool) | ✓ | **Done** | `task` tool, up to eight in parallel; custom agents that list Edit/Write/Bash edit and run commands under the session's mode, and their approvals show in the parent session |
| To-do list tool with visible progress | ✓ | **Done** | `update_todos`, pinned panel above the composer, `/todos` |
| Steering the agent mid-reply | ✓ | **Done** | Type while it works; read before its next step (app and CLI) |
| Hooks | ✓ many events | **Done** | preToolUse (exit 2 blocks), postToolUse, postEdit, userPromptSubmit, sessionStart, stop; matchers |
| OS-level sandbox | ✓ | **Partial** | Linux bubblewrap and macOS sandbox-exec limit writes to the project, temp and caches and hide cloud credentials; Windows uses a job object (tree, process and memory caps, UI limits) without per-folder write limits. Verified by a unit test on Windows only; the Linux and macOS paths run in the release smoke test |
| Extended-thinking display | ✓ | **Done** | Collapsible "Thought process" block |
| Round limit and continue | ✓ | **Done** | Pauses after 40 rounds |

## 2. Context, sessions and memory

| Feature | Claude | Neru | Notes |
|---|---|---|---|
| Saved sessions, rename, delete | ✓ | **Done** | |
| Parallel sessions | ✓ | **Done** | Each session has its own runtime |
| Git worktree sessions | ✓ | **Done** | `/worktree` |
| Rewind conversation and code | ✓ | **Done** | Checkpoints restore files |
| Auto titles | ✓ | **Done** | |
| Auto-compaction and `/compact` | ✓ | **Done** | Also trims old tool output and learns each model's limit |
| Context and quota meter | ✓ | **Done** | Includes provider rate-limit headers |
| AGENTS.md / CLAUDE.md loading | ✓ | **Done** | Root files, plus nested per-folder files when the agent reads there |
| Persistent memory across sessions | ✓ | **Done** | `save_memory`, project and user memory files, `/memory` |
| Resume or continue a session from the CLI | ✓ | **Done** | `neru -c`, `neru -r`, `/resume` |
| Projects / knowledge bases (Chat) | ✓ | **Missing** | |

## 3. Extensibility

| Feature | Claude | Neru | Notes |
|---|---|---|---|
| MCP servers (stdio and HTTP) | ✓ | **Done** | OAuth sign-in; large tool catalogs load on demand |
| Skills (`SKILL.md`) | ✓ | **Done** | Reads `.neru/skills` and `.claude/skills` |
| Custom slash commands | ✓ | **Done** | `.neru/commands` and `.claude/commands`, with `$ARGUMENTS`, `model`, `allowed-tools` and `` !`cmd` `` (trusted projects) |
| Built-in slash commands | ~40 | **Done** | 21 in the app, 19 in the CLI: /model /mode /clear /resume /cost /status /todos /memory /permissions /hooks /export /doctor and more |
| Plugins and marketplace | ✓ | **Missing** | |
| Output styles | ✓ | **Partial** | `/output-style` in the CLI and `outputStyle` in settings (default, explanatory, learning, style files); no picker in the app yet |
| Headless / SDK mode | ✓ | **Partial** | `neru -p` prints one answer and exits (reads stdin); no SDK library |

## 4. Models and providers

| Feature | Claude | Neru | Notes |
|---|---|---|---|
| Model picker | ✓ | **Done** | Scrollable, searchable, handles hundreds of models |
| Multiple providers | Anthropic only | **Ahead** | 20 providers, including free tiers, local Ollama and custom |
| Automatic switch when rate-limited | n/a | **Ahead** | Falls back to another free model or provider on 429, 402 (no credits) and 403 (no access); `--fallback-model` sets the order |
| Model health check | n/a | **Done** | At app start (cached a day), after `neru login`, when a provider is saved, and in `neru doctor`; a free tier locked to the provider's own app (OpenCode Zen) is named as such |
| Reasoning-effort control | ✓ | **Done** | Only for models that support it |
| Image input | ✓ | **Done** | |
| PDF and Word attachments | ✓ | **Done** | |
| Subscription sign-in (claude.ai / ChatGPT) | ✓ | **Partial** | Team runs Claude Code, Codex, OpenCode, Gemini CLI and Cursor Agent on their own sign-ins; Neru's own agent still uses API keys |

## 5. Workspace UI

| Feature | Claude | Neru | Notes |
|---|---|---|---|
| Diff review with inline comments | ✓ | **Done** | Comments can be sent back to the agent |
| Git: stage, commit, branch, push, PR, CI checks | ✓ | **Done** | Includes merge/rebase/stash and failed-check logs |
| Live app preview | ✓ | **Done** | Real dev-server URL detection, built-in static server, "Open preview" offer |
| Browser automation by the agent | ✓ | **Partial** | `check_preview` loads the app in a hidden browser: title, text, console errors, failed loads. No clicking yet |
| Integrated terminal | ✓ | **Done** | Tabs (Ctrl+Shift+T), shells survive view switches, `neru` on its PATH |
| File explorer and editor | Code tab: no editor | **Ahead** | Monaco editor with tabs and save |
| Project-wide search panel | ✓ | **Done** | As-you-type, case/word/regex, grouped by file |
| Voice dictation | ✓ | **Done** | Cloud or on-device speech models |
| Notifications, onboarding, auto-update, themes | ✓ | **Done** | |
| Command palette and shortcuts | ✓ | **Done** | Ctrl+K, Ctrl+N, Ctrl+B, Esc |
| Artifacts (rendered pages) | ✓ | **Missing** | |
| Web search with citations | ✓ | **Done** | |
| Scheduled / background / cloud tasks | ✓ | **Missing** | |
| Sharing conversations | ✓ | **Missing** | |
| Multi-agent workspace (Traycer-style Team) | n/a | **Ahead** | One thread shared by Claude Code, Codex, OpenCode, Gemini, Cursor and Neru; @-hand-offs, routing on limits, artifacts with versions, usage and hand-off map |
| Import from other tools | n/a | **Ahead** | Chats (Claude Code, Codex, OpenCode, Gemini, Cursor, VS Code Copilot), skills, MCP servers and instructions |

## 6. CLI

| Feature | Claude Code | Neru | Notes |
|---|---|---|---|
| Terminal command (`claude` / `neru`) | ✓ | **Done** | `neru-cli` beside the app; installer adds `neru` to PATH on Windows |
| Terminal agent UI with slash commands and approvals | ✓ | **Done** | Welcome box, streamed Markdown, tool rows, diffs to approve, pickers, completion |
| Non-interactive mode (`-p`) for scripts | ✓ | **Done** | Calls that need approval are refused and the run carries on (listed in `permission_denials`); `json`/`stream-json` output shaped like Claude Code's (`result` subtypes, `assistant`/`user` messages with `tool_use`/`tool_result`); `--max-turns` ends with `error_max_turns`; `/init`, `/review` and custom commands expand |
| Arguments like Claude Code's | ✓ | **Done** | `--flag=value`, options after the prompt, `--` before a prompt; mistakes go to stderr with exit 1; `--model` lasts one run; `--append-system-prompt` goes in the system prompt; `-p -r` errors when nothing matches |
| `mcp add` syntax | ✓ | **Done** | `--transport stdio|http`, `-s user|project` (project writes `.mcp.json`), `-e`, `-H`, `add-json`; exit codes for missing connectors and unknown subcommands |
| Login without a terminal prompt | `ANTHROPIC_API_KEY` | **Done** | `NERU_API_KEY` with `NERU_PROVIDER`, `NERU_MODEL`, `NERU_BASE_URL`; nothing is saved |
| Ctrl-C, `NO_COLOR`, headless Linux | ✓ | **Done** | Ctrl-C stops the reply, then exits cleanly; plain output with `NO_COLOR`; without a display the CLI runs itself in Xvfb |
| Installers | ✓ | **Done** | `install.sh` (macOS arm64/x64, Linux x64/arm64, Rosetta-aware, glibc check, `--uninstall`), `install.ps1` (with `NERU_UNINSTALL`), npm. Settings → CLI lists them |
| Custom status line | ✓ | **Done** | `/statusline` and `statusLine` in settings (a project's runs once trusted) |
| End-to-end smoke test | n/a | **Done** | `app/scripts/smoke.mjs` runs on every release build against a mock model |

Team is compared feature by feature with Traycer in `TRAYCER_AUDIT.md`.

## Still missing

- Per-folder write limits for commands on Windows (Linux and macOS have them)
- Plugins and marketplace; an output-style picker in the app
- Artifacts, sharing, Chat projects/knowledge
- Scheduled, background or cloud tasks
- Neru's own agent on a subscription (Team members already use theirs)
- Team: cloud sync, multiplayer and comments, remote hosts
- Clicking and typing in the page during browser checks
