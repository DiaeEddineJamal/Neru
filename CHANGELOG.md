# Changelog

Every release of Neru. The app shows the same notes in *What's new* after it updates. Downloads are on the [releases page](https://github.com/DiaeEddineJamal/Neru/releases).

## 0.8.0 · Team you can watch work, with your models and your projects

### Team

- **Watch members work.** A member's turn shows at once, as in Code mode: the thinking orb with what it is doing ("Thinking…", "Exploring the project…", "Composing…"), its steps as they happen, and Claude Code's reply streaming in word by word. Before, nothing showed until the whole reply arrived, because the window was never told a member had started.
- **The same message box as Code mode.** Dictate with the microphone (and pick the microphone), attach files with **+**, or paste a screenshot. Attachments are copied into the task folder, which every member can read, and their paths go with the message.
- **Model and effort per member.** The Members panel has a dropdown of each agent's models and one of its effort levels: Claude Code's Fable, Opus, Sonnet and Haiku (by alias or full id, plus any newer model your Claude Code has used) with low to max, and every model your Codex account offers (GPT-6 Astra, Sol and Luna, GPT-5.6 and more) with that model's own levels. The old text field hid every model but the one already typed.
- **Pick the project when starting a task:** the one open now, a recent one, another folder, a **new project** (Neru creates the folder and starts a Git repository there), or none for planning only.
- Turn changes leave out `.omc/`, the state folder of the oh-my-claudecode plugin, which is never the agent's work.

## 0.7.1 · No more flashing console windows

### Fixes

- **No more console windows flashing on Windows.** Opening the app no longer pops terminal windows open and shut. Git, GitHub CLI, PowerShell, hooks, version checks, dev servers and the other console programs Neru runs in the background now start without a window. Terminals you open yourself (a member's own terminal) still show.

## 0.7.0 · Team, a CLI that works like Claude Code's, and free models that last longer

### The neru CLI, like Claude Code's

- **Arguments.** `--flag=value` works, options may follow the prompt, and `--` ends them. A mistake prints to stderr and exits 1 instead of showing the help with exit 0. `--version` prints `0.7.0 (Neru)`.
- **`--model` lasts one run**, as in Claude Code, and so does a switch after a rate limit: the saved default stays. A name your provider doesn't list gets a warning. `/model` and `neru login` still change the default.
- **`--append-system-prompt`** now goes in the system prompt instead of being repeated in every message.
- **Print mode (`-p`)**:
  - A call that needs approval is refused and the model carries on, as `claude -p` does, instead of the run stopping with exit 3. Refusals are listed in `permission_denials`.
  - `--output-format json` and `stream-json` have Claude Code's shapes: `result` with `success`, `error_during_execution` or `error_max_turns`, and `num_turns`; `assistant` and `user` messages with `tool_use` and `tool_result`; Claude's names for permission modes.
  - `--max-turns` ends with `error_max_turns` and exit 1, and 0 is refused.
  - `/init`, `/review`, `/security-review` and your own commands expand.
  - A refused tool shows ✗.
  - `-p -r <title or id>` finds a session by title too, and says so with exit 1 when nothing matches instead of starting a new one.
- **`neru mcp add`** takes `claude mcp add`'s syntax:
  - `--transport stdio|http`, `-s user|project` (project writes the project's `.mcp.json`), `-e`, and `-H`, also after a URL;
  - the command's own flags stay with the command;
  - `neru mcp add-json` is new;
  - removing a connector that doesn't exist, or an unknown subcommand, now fails with an exit code.
- **CI and scripts:** `NERU_API_KEY`, with `NERU_PROVIDER`, `NERU_MODEL` and `NERU_BASE_URL`, connects a model without `neru login`. Nothing is saved.
- **Ctrl-C** stops a running reply, then exits cleanly (background commands stopped, session-end hooks run), even while a command runs after an approval. `NO_COLOR` turns colors off, and errors on a piped stderr are plain text.
- **`/compact <instructions>`** tells the summary what to keep, in the CLI and the app.
- **Linux without a display** (SSH, CI, containers): `neru` runs itself inside Xvfb when it is installed, and says what to install when it isn't. No Dock icon on macOS.
- **`neru update`** knows Linux arm64 and Homebrew installs, and cleans up what an update leaves behind on Windows. The project path no longer reaches the model as `\\?\D:\…`.
- **Install:**
  - `install.sh` installs the native build under Rosetta and checks glibc (2.35 or later) and musl before downloading;
  - it finds WebKitGTK on arm64 and suggests Xvfb on machines without a display;
  - `install.sh --uninstall` and `NERU_UNINSTALL=1` for `install.ps1` remove the CLI and keep your settings;
  - a prerelease tag no longer becomes the latest release.
- **Settings → CLI** is back: the install, update and uninstall commands for your system (only ones that work today: the scripts and npm), and every command, flag, environment variable and shortcut.

### Settings and session events, the Claude way

- **Session events in the thread.** A model switch, a compaction, pacing, a rewind or plan mode now show as a quiet line in the conversation, where they happened ("Compacted conversation · saved 12.4k tokens"), instead of a bar across the top. A line still in progress has a spinner and is replaced by the outcome. Finished lines are saved in the session itself, so they are there when you reopen it, after a restart, for a session that ran in the background, and in `neru` exports. They are never sent to the model, and rewind and `/copy` skip them.
- **Settings as a dialog.** Settings open over what you were doing, with a search box and a grouped list on the left (Settings, This computer, Customize), icons, a close button and Esc. On/off choices are switches.

### Free models last longer

- **A smaller working context on free plans.** Free quotas count tokens per minute and per day, and a model with a 1M-token window used to be sent its whole conversation every request. On a free provider or a free model, Neru now works within 64k tokens and summarizes beyond that. Tool results are cut at 12,000 characters, results older than the last eight shrink to a short excerpt, and the first message's project snapshot is half as big. This applies in Code, Chat and to Neru as a Team member.
- **Pacing.** When a response says a model's per-minute requests or tokens are used up, Neru waits for the reset before sending, instead of spending a request on a refusal.
- **Sub-agents on free plans.** At most four per turn. Reading sub-agents run on a quick sibling model of the same key, so they draw on a separate per-minute quota from the main conversation, and they get four rounds and 90 seconds. Older results shrink as they work. A sub-agent that hits a daily cap or a dead model switches to the next best model instead of failing, and each sub-agent's row names the model it ran on.
- **A better next model.** Every saved key is considered at once, with model lists fetched in parallel and cached for ten minutes, so a switch takes moments instead of seconds per provider. Candidates are ranked by coding strength, recent failures and known quotas: a model whose daily quota is spent is left out, and the same key wins a tie. A failed model rests for as long as the provider asked, longer for each failure in a row, up to six hours. Rests and failure streaks are remembered across restarts and shared with the `neru` CLI.

### Resizable Team columns

- Drag the edge of the task list or the task panel (Members, Artifacts and the rest) to resize them; double-click resets. Arrow keys work when the edge has focus. Widths are remembered, and the thread always keeps its room.

### Team, finished

- **A message queue.** A message for a member that is still working waits in a queue above the message box. Edit, reorder, send now, remove, or pause the whole queue; it goes out in order as members finish.
- **Auto access.** Between Edit files and Full access: Claude Code's auto mode and Codex's automatic reviewer approve or block each action. Gemini, Copilot and OpenCode may run safe read, build and test commands only.
- **Members drive Neru's browser.** Claude Code, Codex, Gemini and OpenCode members get three tools from a local, token-protected MCP server: start the dev server, open a page in Neru's browser pane, and read a page's text and errors. Browser annotations and picked elements go to the Team message box while you are in Team.
- **New task tabs.**
  - **Changes**: every file the task changed, across the project and each member's worktree, as one diff in unified or split view, with "Review everything with @member".
  - **Pull request**: the PR for the project's branch or a member's, its checks, and **Fix** to hand the failing log to a member.
  - **Activity**: who is working and for how long, routing, the queue, side chats waiting, and **smart execution**: one member implements each open ticket in order and another reviews it, with a round limit.
  - **Usage**: per-turn history, bar charts and CSV export. Codex's 5-hour and weekly windows now show too. The task header shows the most-used window; Ctrl+Shift+U opens it as a popover.
  - **Map**: replay the hand-offs in order with play, pause and a timeline.
- **Find and organise tasks.** Groups with collapsible headings, an icon and a colour per task, filters by repository, agent, label and unread (all or any), four sorts, and unread, failed and queued marks.
- **Terminals.** Each task has its own terminal tabs, opened in its folder and reopened with it. A member's own CLI can open inside Neru's terminal on its session, or in a separate window.
- **Setup cards** show each worktree setup run as it happens, with its output and Run again. **Deleting a task** lists what each worktree would lose and asks for the task's name before losing uncommitted work.
- **Artifacts** get a formatting toolbar with shortcuts, list continuation, write/split/preview and PDF export.
- **Models and commands.** Each member's model field offers its CLI's own models; `/model @codex …` switches it. `/` also lists each agent's own slash commands (Claude, Codex prompts, Gemini, OpenCode, Cursor); Neru fills in the arguments and sends them to that member. Each member's transcript is kept in the task folder, and the thread can show one member only.
- **More agents.** Qwen Code, GitHub Copilot CLI, Amp, Factory Droid, Goose, Crush, Aider, Auggie, Kiro and Continue join Claude Code, Codex, OpenCode, Gemini and Cursor.
- **Settings → Agents** shows each CLI's version and the newest one, updates or installs a chosen version, and lets you point Neru at a CLI somewhere else.
- **Imports** group chats by project, show progress while importing, list sessions that could not be read with the reason, and list a resumed Codex session once.
- **Getting started**: Team's start page has a checklist that ticks itself off and templates for a first task.
- **Gemini CLI** works again. Google retired its free personal sign-in, so Neru gives it a free AI Studio key from Settings → Agents, kept with your other keys. Neru also tells it to trust the project folder and continues its sessions.
- **Install**: a Homebrew cask and formula come with each release, Linux arm64 builds (WSL on ARM), and `install.sh` accepts Linux arm64.

### Guided walkthroughs

- A **spotlight walkthrough** points at one control at a time, explains it, and glides to the next. It covers the whole app the first time you open it (sessions, Team, chat or code, palette, terminal, files, changes, review, live preview, every message-box control and Settings), Team before your first task, and a task the first time you open one. Steps for controls that aren't on screen are left out. Skip it any time with Esc or **Skip tour**, and replay it from the sidebar's ⋯ menu, the palette, Help or Settings.
- Onboarding has a new **Team** step that shows which agents are installed and signed in on this computer, and the feature tour has a Team tab.

### Fixes

- Switching between model providers no longer forgets their keys. Each saved key is reused when you pick that provider again, in Settings and onboarding, including when its base URL has changed.
- A custom CLI agent that works in its own worktree now finds its script in the project's `.neru/cli-agents`, even when that folder isn't committed.
- Onboarding's scroll bar is a slim rounded thumb in the app's colours instead of the default white one.

### Team: your agents, one thread

- A new **Team** section runs the coding agents you already pay for side by side: Claude Code, Codex, OpenCode, Gemini CLI and Cursor Agent, each on its own sign-in, plus Neru's own agent on the model set up in Neru. Neru finds the installed CLIs and whether they are signed in. Settings → Agents opens each one's own sign-in in a terminal; Neru never reads their credentials.
- Every member of a task sees one shared thread. A member's turn continues its own CLI session and starts with everything teammates posted since its last turn. The whole thread is also kept as `thread.md` in the task folder, for any agent to read.
- Pick who answers with `@claude`, `@codex` or `@all`, or the To chips. Agents hand work to each other by starting a line with a teammate's handle; add `(fyi)` when no answer is needed. A message may cause at most six hand-offs before the team waits for you.
- When a member hits its subscription limit, a card counts down and hands the request to a teammate with the whole thread. Keep it where it is with one click, or turn this off per task.
- Team skills `/plan`, `/tickets`, `/execute`, `/review`, `/debate`, `/explain` and `/walkthrough` write specs, tickets, reviews and debates to the task's artifacts. The Artifacts tab shows them, lets you edit them, and keeps every earlier version to restore.
- Each member's access is set per task. **Read-only** members can still write the task's artifacts but nothing in the project. **Edit files** and **Full access** go further. A member can also work in its own Git worktree, so parallel edits never collide.
- The Usage tab (Ctrl+Shift+U) shows tokens, reported cost, and Claude Code's 5-hour and weekly limits. The Map tab draws who handed work to whom. Search finds past tasks. When a team finishes while Neru is in the background, you get a notification.
- Every agent shows its own logo: Claude, Codex, OpenCode, Gemini CLI, Cursor, GitHub Copilot and VS Code.
- As in Claude, inline code and file paths in replies show as coloured chips, links are underlined, and `@claude`-style mentions stand out from the text.

### Team, closer to Traycer

- **Undo a turn.** Each reply that changed files in a Git project shows a Changes card listing them, and Undo puts those files back as they were. Untracked files are included.
- **Side chats.** `/btw @codex question` asks one member on the side. It answers in a copy of its session, read-only, and the thread does not move on.
- **Fork a member.** It gets a copy that continues from the same session, so you can try another direction.
- **Open a member in its own terminal**, continuing the same session.
- **Worktree scripts.** A member's worktree runs the repository's setup script when it is created and its teardown script before it is removed. Scripts come from `.neru/environment.json`, or Traycer's `.traycer/environment.json`.
- **Sweep** removes members' worktrees whose work has landed and that have nothing uncommitted.
- **Your own agents.** A script in `cli-agents/` gets the prompt in `NERU_PROMPT` and joins a team like any CLI.
- **Find and organise tasks.** Pin tasks, label them, and search every task's messages from the task list.
- **Typed artifacts.** Artifacts are specs, tickets, stories or reviews, and tickets have a status you can click through. Filter them by type, and export them as Markdown.
- **Diagrams and wireframes.** Mermaid diagrams and live HTML wireframes render in artifacts and replies, with fullscreen and zoom.
- **More team skills:** `/phases`, `/verify`, `/critique`, `/revise`, `/autobuild` and `/housekeeping`.
- **Small things.** Messages for a busy member show as queued. The terminal, files, changes and browser panes open next to a Team task. The `@` menu shows each teammate's logo, name and whether it is free.
- `TRAYCER_AUDIT.md` compares every Traycer feature with Neru.

### Import your chats, skills and settings

- Settings → Imports scans the machine for Claude Code, the Claude app, Codex (CLI, desktop app and IDE extension), Cursor, VS Code, OpenCode and Gemini CLI. **Import everything** brings over what it finds; or pick item by item:
  - **Chats** from Claude Code, Codex, OpenCode, Gemini CLI, Cursor and VS Code Copilot become Team tasks, one per project. Claude Code, Codex and OpenCode chats can be continued, since the member resumes that very session.
  - **Skills** from `~/.claude/skills`, `~/.codex/skills`, `~/.agents/skills`, `~/.cursor/skills`, `~/.gemini/skills` and OpenCode's skills folder.
  - **MCP servers** from Claude Code, the Claude app, Cursor, VS Code, Codex, Gemini CLI and OpenCode become connectors.
  - **Instructions** from Codex, Gemini and OpenCode join Neru's own `AGENTS.md`. A project's Cursor rules, `GEMINI.md` and Copilot instructions go to its `.neru/instructions.md`.
- Imported chats read as they were written. The tags tools wrap messages in (`<user_query>`, time stamps, reminders, rules, command echoes) are removed, and messages the tool generated itself, such as Cursor's task notifications, are left out.
- Neru only reads these tools' files and never changes them. Anything that would replace something of yours with the same name is skipped by **Import everything** and marked in the lists.

### Free models that fail clearly, then keep going

- A model refused with HTTP 402 (no credits) or 403 (no access) now switches to the next working model, like a 429 already did. A 403 about the key itself still stops, since another model would fail the same way.
- OpenCode Zen's free models answer 403 "free tier can only be used from within OpenCode". Neru now says so in plain words, sets aside that provider's free models for the day, and moves on. A 402 sets aside its paid models the same way, instead of retrying them every 90 seconds.
- `--fallback-model qwen3-coder,glm-4.6` names the models to switch to first, in order, before Neru picks one itself.
- Health check: the app checks the model in use when it starts, and `neru login` checks the model you pick. `neru doctor` sends the model one small request, so a model that lists fine but refuses requests shows up there. Results are kept for a day, so starting up sends nothing new most of the time.

### Commands run in a sandbox

- The agent's shell commands now run inside the operating system's sandbox. On Linux (with bubblewrap) and macOS they can write only to the project, temporary folders and package caches, and cannot read cloud or GPG credentials. On Windows each command runs in a job object: everything it starts is ended with it, process count and memory are capped, and it cannot touch other apps' windows, read the clipboard or shut Windows down.
- A command that must write elsewhere (a global install) can ask to run outside the sandbox; you are always asked first, except in Bypass mode.
- A rules gate in front of the shell now refuses outright the commands that wipe a drive, your home folder or a system folder, format disks or fork-bomb, whatever the mode. `git reset --hard`, `git clean`, and deleting, moving or writing outside the project always ask, even in Bypass.
- Auto mode runs a command on its own only when every part of it is allowlisted: `git status && curl …` now asks.
- `neru doctor` shows which sandbox is in use. Turn it off with `"sandbox": {"enabled": false}` in `~/.claude/settings.json` or `NERU_SANDBOX=off`.

### Sub-agents that edit

- A custom agent in `.neru/agents` or `.claude/agents` whose `tools` line lists Edit, Write or Bash can now change files and run commands. It follows the session's permission mode and rules; anything that would need your approval shows under the agent's row (or as a prompt in the CLI) and waits for your answer. Agents without those tools stay read-only.
- Rewinding a turn also takes back the edits its sub-agents made.

### Smaller additions

- Custom commands use their `model` (when the provider offers it) and `allowed-tools`, and fill in `` !`command` `` lines with the command's output, as in Claude Code. A project's commands do this only once the folder is trusted.
- New CLI flags: `--system-prompt`, `--add-dir` and `--session-id`.
- `/output-style` picks how Neru writes (default, explanatory, learning, or your own files in `output-styles`), and `/statusline <command>` shows a command's output in the CLI's status line. Both use the same settings keys as Claude Code.
- Stop trusting a folder from Settings → Connectors, or with `/untrust` in the CLI.
- Every release now runs a scripted first session of the CLI on Windows, macOS and Linux: a new project, an edit, a command and a resumed session.

## 0.6.0 · Free models that keep going, 23 new skills, and your choice of microphone
*2026-10-01*

Free plans no longer stop after a few quick messages: Neru waits out per-minute limits instead of giving up, and a limit learned on one provider no longer shrinks the same model everywhere. Models get 23 new built-in skills, you can pick which microphone to dictate with, and the in-app browser's drawing tools fit small windows.

### Free models that keep going

- When a free plan's per-minute limit is hit, Neru waits as long as the provider asks (or about 20 seconds) and sends again, instead of failing with “usage limit reached”. Daily caps still switch to another model.
- Limits Neru learns are now kept per provider. Groq's free 8K tokens a minute for gpt-oss no longer squeezes that model on Hugging Face, OpenRouter or Cerebras. Limits saved by earlier versions are cleared once.
- A provider reporting a token limit of 0 no longer leaves a 128K model with no room for a single message.
- The skills list sent with every request is about half the size, which leaves more of a small per-minute budget for your conversation. The full skill still loads when the model needs it.

### 23 new built-in skills

- From Addy Osmani's agent skills: API and interface design, performance optimization, code simplification, CI/CD, documentation and ADRs, git workflow, spec-driven development, deprecation and migration, and observability.
- From Vercel: React and Next.js best practices, composition patterns, React Native, and the Web Interface Guidelines review.
- From Superpowers: dispatching parallel agents, subagent-driven development and git worktrees. From Anthropic: webapp testing with Playwright and the skill creator.
- From Trail of Bits: modern Python tooling, mutation testing and Semgrep. Cloudflare's security audit, and humanizer for prose that doesn't read as AI-written.
- 42 skills now ship with Neru. Each keeps its license and source, listed in the skills NOTICE.

### Voice

- An arrow beside the microphone picks the input to dictate with. Neru remembers it and falls back to the Windows default when that device is unplugged.
- Hold to record: hold the microphone button to dictate and let go to stop. Turn it on in the same menu.

### Browser and settings

- The drawing toolbar wraps onto a second row in a small window instead of cutting off Add to chat.
- The element inspector, console, network and annotations panels have a close button.
- The CLI section is gone from Settings. Install the neru command with npm, winget or the install scripts as before.

## 0.5.0 · The neru CLI everywhere, only models that work, and Claude Code parity
*2026-09-30*

Install the neru command with npm, winget or a one-line script and use it with the same commands as Claude Code. NVIDIA NIM and OpenRouter now list only the models your key can actually call, and Neru picks up Claude Code project setups: .mcp.json, CLAUDE.md imports, settings permissions, hooks, custom agents and commands.

### Install the CLI

- `npm install -g neru-cli`, `winget install Luziv.Neru.CLI`, or the official scripts: `irm https://raw.githubusercontent.com/DiaeEddineJamal/Neru/main/install.ps1 | iex` on Windows and `curl -fsSL https://raw.githubusercontent.com/DiaeEddineJamal/Neru/main/install.sh | bash` on macOS and Linux.
- `neru update` updates the CLI the same way it was installed. Every release now ships standalone CLI archives with checksums.
- Settings has a new CLI section with the install commands for your system, an Add neru to PATH button, and a reference of commands, flags and shortcuts.

### The terminal, like Claude Code

- A redesigned welcome with Neru's mascot, tips and recent sessions, and a status line showing the permission mode, the model and how much context is left.
- `neru login` connects a provider from the terminal. New commands: `logout`, `models`, `mcp list|get|add|remove`, `sessions`, `doctor`, `config` and `update`.
- New flags: `--permission-mode`, `--dangerously-skip-permissions`, `--effort`, `--output-format json|stream-json`, `--append-system-prompt`, `--allowedTools`, `--disallowedTools`, `--max-turns` and `--cwd`.
- `!` runs a shell command and hands its output to Neru, `#` saves a memory, shift+tab cycles the permission mode, esc esc rewinds, ctrl+r searches your history and `?` shows every shortcut.
- New slash commands: /login, /logout, /effort, /fork, /rename, /rewind, /mcp, /skills, /agents, /bashes, /security-review, /pr-comments, /diff, /copy, /release-notes, /update, /bug and /terminal-setup.

### Only models that work

- NVIDIA NIM: every model in the catalog was checked with a free key. The 40 that answer “Function not found for account” are gone; the 14 that work are marked confirmed. Models NVIDIA adds later are checked quietly in the background.
- OpenRouter: Neru now asks for the models your account can use (its privacy settings, provider preferences and guardrails), keeps only models that can call tools, and hides paid models on a free-tier key.
- A model a provider refuses is no longer offered anywhere.

### Claude Code parity

- Plan mode ends with a plan to approve: approve and let Neru edit, approve and review each change, or keep planning with feedback.
- Neru can ask you multiple-choice questions when a decision is yours, in the app and in the terminal.
- Dev servers and watchers run in the background; Neru reads their output and stops them when asked.
- Custom sub-agents from .neru/agents and .claude/agents, and custom commands with arguments ($1, $ARGUMENTS), @file includes and folders as namespaces.
- Claude Code project setups work unchanged: .mcp.json servers, CLAUDE.md hierarchy with @imports, permission rules and hooks from .claude/settings.json. Project servers and hooks run only after you trust the folder once.

### Fixes

- The onboarding fits small laptop screens: the window can be moved, resized and maximized, and Get started is always visible.
- Session times in the terminal no longer all read “just now”.
- Replies from providers that don't stream now show up in the terminal.

## 0.4.0 · Replies that don't drop, draw on your app, and a faster agent
*2026-09-30*

Neru now retries a dropped connection instead of failing the reply, reads small projects in one step, and lets you draw on your running app and point at elements the way Claude does.

### Reliable replies

- When a provider drops the connection mid-answer (OpenRouter's “Network connection lost”), Neru retries the same request after 1, 2, 4, 8 and 15 seconds and says so, instead of failing the whole reply. Text from the failed attempt is taken back, so nothing shows twice.
- If the connection keeps dropping, Neru moves to the next best model, as it already does for rate limits.
- Sub-agents and conversation summaries retry the same way.
- Requests reuse one kept-alive connection, so there is no new handshake for every message and long thinking pauses are less likely to be cut off.

### A faster agent

- Prompt caching for Claude models, directly and through OpenRouter: each step pays only for what is new, like Claude Code. Other models reuse the unchanged start of the conversation on their own.
- Small projects (up to 24 files) go with your first message, like the open files Cursor sends, so the model starts working instead of spending steps reading.
- A fast project index: a background, .gitignore-aware index with trigram search and a map of files and symbols, so the agent finds code in large repositories without opening folder after folder.
- Sub-agents research several areas at once, share the index and report back with file:line references.
- Say “use the skills you need” and the model picks the best-fitting skills from Neru's set, loads them first, and names the ones it used.

### Your app, in Neru's browser

- Draw on the page: pen, line, arrow, rectangle, ellipse and text in five colors, with undo, redo and clear, then add the marked-up picture to the chat.
- Pick an element and it joins your message as a small chip with its tag and text, as in the Claude app, with its details sent to the model instead of pasted into the box.
- Browser tabs, element picking, annotations, and console and network capture work on dev servers and plain HTML sites alike, hot reload included.
- Terminal, files and changes stack in one column beside the conversation, and the browser gets its own, like Claude Code's workspace.

### Models and messages

- A new model picker: search, filter by images, reasoning, code and tools, and check which models your key can really use.
- Queue messages while Neru works, or send one in a forked session, from a menu on the send button.
- The session title in the title bar opens the session and project menus, and renames in place.

### Design

- The to-do list uses Beautiful UI's Task Rows: numbered steps, a spinner on the task in progress, and check badges as tasks finish.
- Your messages sit in a calm charcoal bubble in dark mode, and rise into place with a softer send animation.
- The floating sidebar keeps its green light-mode colors instead of turning dark.

### Fixes

- The rewind menu is no longer see-through over the messages below it.
- Opening a local file such as index.html is no longer counted as a web lookup.

## 0.3.0 · Built-in skills, right-click menus and a working preview
*2026-09-30*

Neru now ships with 19 expert skills for design, engineering and security, so any model plans, tests, reviews and designs with care instead of producing generic AI output.

### Built-in skills

- 19 skills come with Neru, curated from the most used open skill collections: Anthropic's frontend-design and mcp-builder, Impeccable for design polish, Superpowers for planning, test-driven development, systematic debugging, verification and code review, and Trail of Bits for security review, sharp edges, supply-chain risk and property-based testing.
- Two written for Neru: neru-writing keeps replies, docs and UI copy plain and specific, and neru-web-security is an OWASP checklist with concrete fixes.
- Like Claude, the model sees each skill's name and description and loads the full instructions, and any reference file, when a task matches. UI work always follows the design skills.
- Settings → Skills groups them by topic, credits their authors, and lets you switch any of them off. A project or personal skill with the same name replaces the built-in one.

### Right-click menus

- Files and folders: open, open in your editor, attach, ask Neru about it, rename in place (F2), duplicate, new file or folder, copy the path, Open File Location, and delete (Del) with a checkpoint.
- Editor tabs, sessions and projects have their own menus, and a project can be removed from the sidebar without touching its files.

### Fixes

- The preview no longer shows “127.0.0.1 refused to connect”: pages load inside Neru's browser for plain HTML sites and dev servers alike.
- Tool steps name the skill reference file being read.

## 0.2.0 · Faster, smarter, and in your terminal
*2026-09-30*

Coding works reliably on NVIDIA, OpenRouter and other free models, files appear as they are written, and Neru now runs in your terminal too.

### Coding that finishes

- Whole files no longer get cut off: Neru asks providers for room to write them, and recovers when a model sends broken or truncated tool calls instead of failing the reply.
- Several files are written in one step, and in Review mode the rest wait in line behind your approval instead of being generated again.
- Longer tasks (up to 40 steps), fixes for models that repeat tool names, and support for models that write tool calls as text.
- The agent checks the app it built in a real browser and fixes the errors it finds.

### Watch it work

- Files appear as they are written, as compact Write and Edit rows with the newest lines streaming in VS Code colors.
- Code in replies and in the editor uses VS Code's Dark+ and Light+ colors, with line numbers and a copy button.
- A to-do list shows the plan for multi-step work, and a collapsible thought process shows what reasoning models are thinking.
- Type while Neru works: it reads your message before its next step.

### Smarter agent

- Sub-agents research several parts of a big codebase in parallel.
- Memory: Neru remembers your preferences and project conventions across sessions (/memory).
- When a model is slow, down, missing or rate-limited, Neru switches to the next best model on the same provider or another one, and tells you in plain words.
- Hooks for every step: preToolUse (can block), postToolUse, userPromptSubmit, sessionStart and stop, and AGENTS.md files in subfolders are read when the agent works there.

### neru in the terminal

- A full terminal agent: type neru in any terminal (the installer adds it to your PATH) or in Neru's own terminal.
- Streaming replies with highlighted code, diffs to approve, slash commands, @ file mentions, history, and neru -p for scripts. It shares sessions and settings with the app.
- The integrated terminal has tabs, keeps shells running in the background, and greets you with the mascot.

### Everyday polish

- Preview: when Neru builds something, Open preview starts the dev server and loads it in Neru's browser, for any framework or plain HTML.
- 21 slash commands, including /model, /mode, /resume, /cost, /status, /memory, /permissions, /hooks, /export and /doctor.
- Images you send show in the conversation, and PowerPoint, Excel, OpenDocument and RTF files can be attached alongside PDFs and Word documents.
- A searchable, scrollable model picker, project search as you type with case, word and regex options, and a refresh icon in the file tree.
- A redesigned onboarding, friendlier error messages with one-click fixes, a larger mascot, new message bubbles and send animation, and a new paper-cut installer.

## 0.1.0 · The first release
*2026-09-29*

Neru is a local-first coding agent: it explores your project, proposes changes you read line by line, and never acts without asking.

### An agent that works on your project

- Reads, searches, and edits files, and creates, renames, moves, and deletes files and folders — scaffolding whole features file by file.
- Every change shows a diff first and saves a checkpoint, so Undo and Rewind put files and folders back exactly.
- A project tree beside the conversation marks the files Neru just touched.
- Permission modes: review every step, accept edits, plan read-only, auto for routine work, or bypass.

### Free models that can really code

- NVIDIA NIM, ModelScope, Google Gemini, Cerebras, Mistral, and OpenRouter free plans, each with a link to get a key and its daily allowance.
- When a model hits its limit, Neru switches to the next best free coding model and keeps going.
- Model lists put the strongest coding models first. Local models through Ollama, and OpenAI, Anthropic, DeepSeek, and other keys work too.

### Context that fits

- A live context meter shows the model's real window, the per-request limit of your key, and requests or tokens left today.
- Large files are read in ranges, stale file reads are dropped, long tool output is trimmed, and long sessions are summarized.
- Big connector catalogs load on demand instead of filling every request.

### Calmer, clearer thinking

- Thinking orbs cross-fade between states, and the status text glides with a soft shimmer and a timer.
- Steps appear on a timeline and settle with a check; finished replies recap what was read, created, edited, moved, or run.
- Sessions get a short title that says what you are working on.

### Everything else

- Skills: add Markdown skill files or folders in Settings → Skills. Neru loads a skill when a task matches its description, and /name runs it directly.
- Parallel sessions with Git worktrees, a review pane, pull requests and CI checks, MCP connectors, slash commands, on-device dictation, a terminal, and a preview pane.
- A redesigned onboarding where every control fits on small windows, with arrow-key navigation.
- Branded installers for Windows, macOS, and Linux, and automatic updates from GitHub Releases.
