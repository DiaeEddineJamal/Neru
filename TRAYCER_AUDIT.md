# Neru vs Traycer: feature audit

*2026-10-03 (updated). Traycer as documented at docs.traycer.ai: all 95 pages, Desktop/Host/CLI v1.4.2, extension v2.16.9. Neru at the current working tree, after the Team and Imports work.*

**How to read it.**

| Mark | Meaning |
|---|---|
| ✅ | Neru does this |
| 🟡 | Neru does part of it; the note says what is missing |
| ❌ | Not built yet |
| ⛔ | Needs Traycer's cloud, account or hosted service; out of scope for a local app |

**Built in the second pass (every partial feature finished):** the message queue (edit, reorder, pause), Auto access with each CLI's own reviewer, Neru's browser as an MCP server for members, a combined Changes tab with split diffs, a pull-request tab with Fix in chat, the Activity tab with smart execution, a usage dashboard with CSV export and a header reading, map replay, task groups, icons, colours, filters and unread marks, per-task and embedded CLI terminals, setup cards, graded deletes, a rich artifact editor and PDF export, model pickers, each agent's own slash commands, CLI versions and paths, ten more coding CLIs, import progress and by-project view, a getting-started checklist and templates, Homebrew and Linux arm64. Gemini CLI now runs on a free API key (Google retired its free personal sign-in).

**Built in the first pass:**
- per-turn change cards with **Undo**
- **side chats** (`/btw`, `/side`)
- **forked members**
- **open a member in its own terminal**, same session
- **worktree setup/teardown scripts**: reads `.neru/environment.json` and Traycer's `.traycer/environment.json`
- **Sweep** of landed worktrees
- **custom CLI agents** (`cli-agents/`)
- **pinned and labelled tasks**, and **message search** across tasks
- **typed artifacts** with status: spec, ticket, story, review
- **Mermaid diagrams** and **live HTML wireframes**
- **Markdown export**
- **queue indicators**
- seven more team skills: `/phases`, `/verify`, `/critique`, `/revise`, `/autobuild`, `/housekeeping`, `/btw`
- the terminal, files, changes and browser **side panes inside Team**
- a redesigned **@-mention menu**
- **spotlight walkthroughs** (app, Team, task) and a **Team step in onboarding**

## Summary

| Area | ✅ | 🟡 | ❌ | ⛔ |
|---|---|---|---|---|
| Install, first run, sign-in, import | 11 | 0 | 0 | 2 |
| Tasks and panels | 12 | 0 | 3 | 2 |
| Agents and composer | 19 | 0 | 16 | 0 |
| Agent-to-agent | 6 | 0 | 2 | 1 |
| Models, providers, usage, routing | 9 | 0 | 7 | 2 |
| Artifacts | 9 | 0 | 7 | 0 |
| Terminals, browser, git, PRs, files, layout | 5 | 0 | 3 | 2 |
| Worktrees and Sweep | 5 | 0 | 5 | 0 |
| Hosts | 0 | 0 | 2 | 2 |
| History, search, navigation | 7 | 0 | 3 | 0 |
| Notifications | 2 | 0 | 3 | 1 |
| Sync, teams, mobile | 0 | 0 | 0 | 6 |
| IDE-extension modes | 10 | 0 | 3 | 0 |
| **Total** | **95** | **0** | **54** | **18** |

**What remains falls into three groups.**
- **Cloud and account features (⛔)** need a hosted service: sync, sharing, teams, mobile, remote hosts, Traycer credits.
- **Larger local features (❌)** that would each be a project of their own:
  - multiple accounts per agent
  - artifact comments and hierarchy
  - a canvas with splits and tabs
  - a notification center with sounds and hooks
- **Composer conveniences (❌)** Neru's solo sessions already have but Team doesn't yet: attachments, voice, effort, context meter, steering.

## 0. Install, first run, sign-in, import

| Traycer | Neru | Notes |
|---|---|---|
| Installers for macOS, Windows, Linux | ✅ | NSIS/MSI, macOS and Linux builds from the release workflow |
| Homebrew / WSL | ✅ | Homebrew cask and formula rendered and attached to each release; Linux arm64 builds for WSL on ARM; README covers WSL |
| App updates (check, install) | ✅ | Built-in updater |
| Account sign-in, uninstall from account | ⛔ | Neru has no account |
| First-launch host install | ⛔ | Neru has no separate host service; everything runs in the app |
| Introduction tour | ✅ | Onboarding has a Team step that detects installed agents; spotlight walkthroughs of the whole app, Team and an open task play once each, can be skipped, and replay from the sidebar menu, palette, Help menu or Settings |
| **Import past sessions** (Claude Code, Codex, OpenCode) | ✅ | Plus Gemini CLI, Cursor, VS Code Copilot; time range, per-source filter, Show imported, Imported label; continued sessions resume |
| Import: By project view, background progress, unreadable flags | ✅ | By-project grouping with group checkboxes, a live progress bar while importing, unreadable sessions listed with the reason; duplicate Codex logs listed once |
| Getting-started checklist | ✅ | A checklist on Team’s start page that ticks itself off: an agent signed in, history imported, first task, the tour |
| First-task guide / quickstart from the start page | ✅ | Templates on Team’s start page: plan a feature, review changes, fix a bug together, debate an approach; each sends the first message |
| Sign-in per provider from the intro | ✅ | Settings → Agents opens each CLI's own sign-in |
| Skills, MCP and rules import | ✅ | Neru goes further: skills from `~/.claude`, `~/.codex`, `~/.agents`, `~/.cursor`, Gemini, OpenCode; MCP from every tool; instructions |
| Unreadable sessions flagged | ✅ | Listed, greyed out, with the reason |

## 1. Tasks and panels

| Traycer | Neru | Notes |
|---|---|---|
| Task as the container (agents, artifacts, files) | ✅ | Team task: members, thread, artifacts, task folder |
| Task folder on disk | ✅ | `data/teams/<id>`: `thread.md`, `artifacts/` |
| Several workspace folders per task | ❌ | One project per task; members can have their own worktree |
| Agents panel | ✅ | Members tab |
| Artifacts panel | ✅ | Kinds, status, filter, versions, export |
| Terminals panel | ✅ | Each task has its own terminal tabs, opened in its folder and restored with the task |
| Browsers panel | ✅ | Members drive Neru’s browser through its MCP server: start the dev server, open pages in the pane, read a page’s text and errors; browser feedback goes to the Team composer |
| Git Diff panel | ✅ | Changes tab: every file the task changed across the project and worktrees, unified or split; edit a file in place from the review pane |
| Pull Requests panel | ✅ | Pull request tab per task: the project’s or a member’s branch, its checks, Fix in chat to a chosen member |
| File Tree panel | ✅ | Files pane with editor, next to Team |
| Sharing panel | ⛔ | |
| Comments panel | ❌ | |
| Usage button, Sweep button | ✅ | Usage tab (Ctrl+Shift+U) and Sweep in the task header |
| Labels | ✅ | Per task; shown in the rail and searchable |
| Label colours, manage labels | ❌ | |
| Groups, task icon and colour | ✅ | Groups with collapsible headings, 12 icons, 7 colours |
| Sync indicator | ⛔ | |

## 2. Agents and composer

| Traycer | Neru | Notes |
|---|---|---|
| Chat agents | ✅ | Every member is a chat in the shared thread |
| Terminal agents (the CLI's own TUI) | ✅ | A member’s own CLI opens embedded in Neru’s terminal pane on its session, or in a separate window |
| New agent dialog | ✅ | Add agent menu: CLIs, Neru, your scripts |
| Child agents, nested tree, drag to re-parent | ❌ | Flat list; forks are noted in the thread |
| Search / filter / sort agents | ❌ | |
| Rename, archive, copy ID | ❌ | Remove only |
| Status icons (working, failed, …) | ✅ | |
| Agent roles (experimental) | ❌ | |
| Enter sends, Shift+Enter newline | ✅ | |
| Message queue while an agent works | ✅ | Messages to busy members queue up; edit, reorder, send now, remove, pause and resume |
| Steer into a running turn | ❌ | In Team. Neru's solo sessions can steer |
| Model picker per agent | ✅ | A picker filled from each CLI (Codex’s model list, Claude and Gemini aliases, OpenCode/Cursor `models`), plus `/model @member name` |
| Thinking effort, fast mode | ❌ | In Team. Solo sessions have effort |
| Mid-turn settings rules | ✅ | Access and model apply from the next turn |
| `@` mention members, @all | ✅ | Logo, name, handle, live status; leading mentions pick who answers |
| `@` files, folders, git, PRs, tasks, artifacts, terminals, browser | ❌ | In Team. Solo sessions mention files |
| `/` skills and agent commands | ✅ | 14 team skills plus each agent’s own commands (Claude, Codex prompts, Gemini/Qwen TOML, OpenCode, Cursor), expanded and sent to that member |
| Side chats (`/btw`, `/side`) | ✅ | Answered in a fork of the member's session, read-only; the thread does not move on |
| Attachments, images | ❌ | In Team. Solo sessions have them |
| Voice input | ❌ | In Team. Solo sessions have on-device dictation |
| Drafts | ❌ | |
| Quote a reply | ❌ | |
| Context usage chip | ❌ | In Team. Solo sessions have it |
| Supervised mode with approval cards | ❌ | Headless CLIs can't stop to ask. Read-only / Edit files / Full access instead; Neru's own member uses its normal approvals |
| Auto-accept edits, Full access | ✅ | "Edit files" and "Full access" |
| Auto mode with a judge model | ✅ | Auto access: Claude Code’s auto mode and Codex’s automatic reviewer judge each action; Gemini, Copilot and OpenCode get safe commands only |
| Agent questions | ❌ | Not possible headless |
| Message actions: copy | ✅ | |
| Message actions: edit, delete | ❌ | |
| Fork an agent | ✅ | A copy that continues a fork of the member's session (Claude `--fork-session`, Codex `exec fork`, OpenCode `--fork`) |
| **Undo a turn** | ✅ | Change card per turn (a Git snapshot of the whole working tree, untracked included), Undo restores; earlier artifact versions are kept automatically |
| Files changed / review all | ✅ | Changes tab with one combined diff and “Review everything with @member” |
| Active / background work panels | ✅ | Activity tab: who is working and for how long, routing, the queue, side chats waiting, smart execution |
| Long chats: lazy loading, minimap | ❌ | |
| Per-chat usage | ✅ | Usage tab per member |

## 3. Agent-to-agent

| Traycer | Neru | Notes |
|---|---|---|
| Hand work to another agent | ✅ | A reply line starting `@handle` |
| Fire-and-forget messages | ✅ | `(fyi)` on the line |
| Read another agent's transcript | ✅ | `transcripts/<handle>.md` per member in the task folder, named in every prompt; a transcript filter in the thread |
| Loop protection | ✅ | Six hand-offs per message |
| Agents create or stop other agents | ❌ | |
| Agent roles | ❌ | |
| Cross-host messages | ⛔ | |
| Agent office (office view, replay timeline) | ✅ | Map with a replay timeline: play, pause and scrub through hand-offs, highlighting each edge and its message |
| Shared context across providers | ✅ | Catch-up of what each member missed plus its own resumed session |

## 4. Models, providers, usage, routing

| Traycer | Neru | Notes |
|---|---|---|
| Coding agents | ✅ | 15 CLIs: Claude Code, Codex, OpenCode, Gemini CLI, Cursor Agent, Qwen Code, GitHub Copilot CLI, Amp, Factory Droid, Goose, Crush, Aider, Auggie, Kiro, Continue; plus Neru and your own scripts |
| Sign in per provider | ✅ | Opens the CLI's own login |
| Custom agents (scripts) | ✅ | `cli-agents/*.cmd|ps1|sh` with `NERU_PROMPT` etc. |
| Several accounts (profiles) per agent | ❌ | |
| CLI version management, custom paths | ✅ | Settings → Agents: installed and newest version, update or install a chosen version, choose a CLI path or go back to PATH |
| Usage limits per provider | ✅ | Claude Code and Codex 5-hour and weekly windows (Codex from its session log); tokens and cost for the rest |
| Usage popover (Ctrl+Shift+U), status-bar readings | ✅ | A limit reading in the task header and a popover on Ctrl+Shift+U |
| Low-usage banner, switch account | ❌ | |
| Cost and token dashboards | ✅ | Per-turn history, bar charts per member, CSV export |
| Automatic routing on limits | ✅ | Countdown card hands the request to a teammate with the whole thread; Keep it here; toggle per task |
| Routing plan: equivalent models, wait for reset, switch back | ❌ | |
| Routing settings page | ❌ | |
| Traycer Inference credits, plans, teams billing | ⛔ | |
| Plugins | ❌ | |
| OpenCode model providers UI | ❌ | |
| Model routing overrides per problem | ❌ | |
| Neru's own free models as a member | ✅ | Neru goes further here |
| Account sessions/devices | ⛔ | |

## 5. Artifacts

| Traycer | Neru | Notes |
|---|---|---|
| Types: spec, ticket, story, review; status on tickets/stories | ✅ | Front matter; status pill cycles To do → In progress → Done |
| Filter by type | ✅ | |
| Agents create artifacts by writing files | ✅ | Read-only members can still write artifacts |
| Built-in planning/building/review skills | ✅ | `/plan`, `/phases`, `/tickets`, `/execute`, `/review`, `/verify`, `/critique`, `/revise`, `/autobuild`, `/debate`, `/explain`, `/walkthrough`, `/housekeeping` |
| Edit artifacts | ✅ | Formatting toolbar, shortcuts, list continuation, write/split/preview, own undo history (live co-editing needs the cloud) |
| Version history and restore | ✅ | Every save and every agent edit keeps the previous copy |
| Version diff view, version labels | ❌ | |
| Mermaid diagrams | ✅ | Rendered, copy source, fullscreen with zoom; also in chat replies |
| Wireframes (live HTML) | ✅ | Sandboxed preview, resize, fullscreen, copy |
| Export | ✅ | Markdown files to a folder, or PDF through the system print dialog |
| Hierarchy and nesting | ❌ | Folders only |
| Search artifacts | ❌ | |
| Unread markers | ❌ | |
| Deleted-artifact recovery | ❌ | |
| Comments on artifacts | ❌ | |
| Send selection to chat | ❌ | |

## 6. Terminals, browser, git, PRs, files, layout

| Traycer | Neru | Notes |
|---|---|---|
| Terminals per task | ✅ | Per-task terminal tabs in the task’s folder, saved and restored |
| In-app browser, annotate, send to chat | ✅ | Annotations and picked elements go to the Team composer while in Team; members open and read pages through Neru’s MCP server |
| Git diff with split view | ✅ | Unified and split views in the review pane and the Changes tab |
| Edit inside the diff | ❌ | |
| PR panel with checks and "Fix in chat" | ✅ | Per task, per branch, with Fix in chat to a member |
| File tree with previews and open-in-editor | ✅ | |
| Canvas splits, tab groups, pane opener, find in tile | ❌ | |
| Window split view, multiple windows | ❌ | |
| Remote browser streaming | ⛔ | |
| Sharing roles | ⛔ | |

## 7. Worktrees and Sweep

| Traycer | Neru | Notes |
|---|---|---|
| Run in a new worktree | ✅ | Per member, switch in the member card |
| Setup and teardown scripts | ✅ | Reads `.neru/environment.json` or `.traycer/environment.json`, per platform |
| Sweep landed worktrees | ✅ | Removes members' worktrees that are clean and landed (or have no commits); keeps branches |
| Choose source branch, name, existing worktree | ❌ | Branch is `neru/<id>` from HEAD |
| Repository settings dialog for scripts | ❌ | Edit the JSON file |
| Setup status card | ✅ | A live card per setup run: running, done or failed, with the output and Run again |
| Branch prefix setting | ❌ | |
| Worktree inventory, automatic cleanup | ❌ | |
| Graded delete confirmations | ✅ | Deleting a task lists each worktree’s uncommitted files and unmerged commits; losing them needs the task’s name typed |
| Remove worktrees when a task is deleted | ❌ | |

## 8. Hosts

| Traycer | Neru | Notes |
|---|---|---|
| Local host service, doctor | ⛔ | Neru runs in-process; `neru doctor` covers the agent |
| Remote hosts, port forwarding | ⛔ | |
| Resource monitor | ❌ | |
| Prevent sleep while running | ❌ | |

## 9. History, search, navigation

| Traycer | Neru | Notes |
|---|---|---|
| Task history with search | ✅ | Task rail: search titles, folders, labels and members |
| Pin tasks | ✅ | |
| Search inside messages | ✅ | Across every task; opens the task at the message |
| Command palette | ✅ | Includes Team and Imports |
| Back / forward | ✅ | Neru's navigation history |
| Text search in code | ✅ | Search section |
| History filters (owner, repo, AND/OR), sorts | ✅ | Repository, agent, label and unread filters, all (AND) or any (OR), four sorts |
| Reopen closed tab, several windows, deep links | ❌ | |
| Home tab (needs you / running) | ❌ | |
| Report issue dialog | ❌ | |

## 10. Notifications

| Traycer | Neru | Notes |
|---|---|---|
| OS notification when work finishes | ✅ | When a team finishes while Neru is in the background |
| Notification center | ❌ | |
| Sounds per severity | ❌ | |
| Notification hooks (script / webhook) | ❌ | |
| Tab indicators | ✅ | Unread dot, failed mark, queued count and running spinner on every task |
| Phone push | ⛔ | |

## 11. Sync, teams, mobile

All ⛔: cloud sync and offline states, sharing a task, teams and seats, mobile app, web dashboard, account sessions.

## 12. IDE-extension modes

| Traycer | Neru | Notes |
|---|---|---|
| Plan | ✅ | `/plan` |
| Phases | ✅ | `/phases` |
| Review | ✅ | `/review` |
| Epic (specs and tickets) | ✅ | Typed artifacts + `/tickets` |
| Verification by severity | ✅ | `/verify` |
| YOLO / smart execution | ✅ | Smart execution: an executor implements each open ticket in order, a reviewer checks it and can send it back, with a round limit; stop any time |
| Handoff to other agents | ✅ | @ hand-offs between members |
| Custom CLI agents | ✅ | |
| Mermaid in plans | ✅ | |
| AGENTS.md | ✅ | Neru and each CLI read their own |
| Custom workflows | ❌ | |
| Templates | ❌ | |
| Model profiles / cost calculator | ❌ | |

## Where Neru goes further

- Imports from Gemini CLI, Cursor (its chat database) and VS Code Copilot. Imports skills, MCP servers and instruction files, not only sessions.
- Neru's own agent joins a team on free or API models from 20 providers.
- Per-turn Undo for every member, from a Git snapshot that includes untracked files.
- Claude-style rendering: code and path chips, `@mention` chips, underlined links.

## Suggested next steps (local, highest value first)

1. **Composer parity in Team:** attachments, voice, effort, context meter, steering into a running turn.
2. **`@` mentions** for files, artifacts and other tasks, inserted as references the member reads.
3. **Profiles:** several accounts per agent via `CLAUDE_CONFIG_DIR` / `CODEX_HOME`, plus a low-usage banner.
4. **Agent tools:** let members create, message and stop teammates through a small `neru team` CLI.
5. **Artifacts:** comments, hierarchy, search, deleted-artifact recovery, version diff, PDF export.
6. **Worktree options:** source branch, branch name and prefix, an inventory with auto-cleanup, removal when a task is deleted.
7. **Notification center** with sounds and webhooks.
