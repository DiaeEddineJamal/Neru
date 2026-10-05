# Changelog

Every release of Neru. The app shows the same notes in *What's new* after it updates. Downloads are on the [releases page](https://github.com/DiaeEddineJamal/Neru/releases).

## 0.10.0 · Neru knows your name
*2026-10-05*

Tell Neru your name and how to address you, and every model you work with uses it. Desktop notifications now reach the Windows notification panel, and the desktop app goes back to API models only.

### You

- Setup asks what Neru should call you and whether to address you as a man or a woman. Models use your name now and then, and the right masculine or feminine forms in languages such as French or Arabic.
- Change your name or gender any time in Settings → General → You.

### Fixes

- Desktop notifications show up on Windows again, with Neru's name and icon in the notification panel, even when no Start menu shortcut was created.

### Changes

- Pocket Lab is no longer in the desktop app. Neru on your computer works with API models; offline models stay in the phone app.

## 0.9.0 · Neru on your phone
*2026-10-05*

Pair your phone with Neru and keep up with your sessions and Team from anywhere: read replies as they stream, look over diffs, approve or deny, send the next prompt, and start new Team tasks. The phone talks to this computer over your own network, or over the internet when you turn that on, encrypted with a key only the two of them know.

### Neru Remote

- Pair a phone by scanning the QR code in Settings → Phone with the Neru app, or paste the pairing link into it. Remote is off until you turn it on.
- Follow every session from the phone: replies stream in as they are written, and diffs, commands and plans waiting for approval show in full.
- Approve or deny, answer the agent's questions, reply, or stop a run, from the phone. A reply started on the phone streams in the desktop window too.
- Team on the phone: follow a task's thread, message the whole team or one member, and stop a member or the task.
- The connection stays on your network and every message is encrypted with the pairing key. Reset pairing makes a new code and disconnects every phone.
- Connect over the internet: turn it on in Settings → Phone to reach this computer from mobile data or another Wi-Fi network. Traffic goes through an encrypted tunnel, and the relay only ever sees ciphertext.
- Start a Team task from the phone: pick the agents and models installed on this computer, choose a project and access mode, and send the first message. @-mention members while you write, just like on the desktop.

### Pocket Lab

- Settings → Pocket Lab downloads open models and runs them offline on this computer, with their own settings for thinking, context length and speed.
- Magic Touch cuts an object out of a photo with one click, entirely on your machine.

### Polish

- Settings has a slim scrollbar that follows the light and dark themes.
- Errors raised while Settings is open now show at the top of Settings instead of hiding behind it.
- When the window is newer than the app behind it, Neru says it needs a restart instead of showing a raw "command not found".

## 0.8.1 · Images open, and no more .omc
*2026-10-03*

Pictures open in the file viewer instead of failing, and Team members no longer leave .omc folders in your projects.

### Fixes

- Images in Files show as a preview instead of “stream did not contain valid UTF-8”. Other binary files say they open in another app, with a button to do it.
- Claude Code members run without the oh-my-claudecode plugin, so no .omc folder appears in your projects. Your own Claude Code keeps the plugin, and Neru hides .omc from files, search and changes.
- A project created with a new task is listed once in the sidebar, not twice.

## 0.8.0 · Team you can watch work, with your models and your projects
*2026-10-03*

Team members now show their work as it happens, with the thinking orb and streamed replies from Code mode. The message box gains dictation and attachments, each member gets a model and effort dropdown with the newest Claude and Codex models, and a new task can start in any project, including a brand new one.

### Team

- A member's turn shows at once: the thinking orb with what it is doing, its steps, and Claude Code's reply streaming in word by word.
- The message box is Code mode's: dictation with your choice of microphone, attachments with +, and pasted screenshots. Attached files go to the task folder, where every member can read them.
- Each member has a model and an effort dropdown: Claude Code's Fable, Opus, Sonnet and Haiku with low to max, and every model your Codex account offers with its own levels.
- A new task can work in the open project, a recent one, another folder, a new project that Neru creates with a Git repository, or none.
- Turn changes no longer list the oh-my-claudecode plugin's .omc state files.

## 0.7.1 · No more flashing console windows
*2026-10-03*

On Windows, opening Neru no longer pops terminal windows open and shut.

### Fixes

- Git, GitHub CLI, PowerShell, hooks, version checks, dev servers and the other console programs Neru runs in the background now start without a window. Terminals you open yourself still show.

## 0.7.0 · Team, a CLI that works like Claude Code's, and free models that last longer
*2026-10-03*

Team runs the coding agents you already pay for (Claude Code, Codex, Gemini CLI, OpenCode, Cursor and ten more) in one shared thread, and imports your chats, skills and connectors from them. The neru CLI now takes Claude Code's flags and output formats, and installs on Windows, macOS and Linux, x64 and arm64. Free models use far fewer tokens per request, and switching to the next working model is quicker and smarter.

### Team: your agents, one thread

- A new Team section runs Claude Code, Codex, OpenCode, Gemini CLI, Cursor Agent and ten more coding CLIs side by side, each on its own sign-in, plus Neru's own agent. They share one thread, hand work to each other with @mentions, and route around a member's usage limit.
- Team skills write specs, tickets, reviews and debates to the task's artifacts, with versions, diagrams and wireframes. Members can work in their own Git worktrees, and Undo takes back a turn's file changes.
- A message queue, side chats, forks, per-task terminals, a usage tab, and resizable task list and panel.

### The neru CLI, like Claude Code's

- Claude Code's flags and habits: --flag=value, options anywhere, usage errors on stderr with exit 1, --model for one run, --append-system-prompt in the system prompt, /compact with instructions.
- Print mode refuses calls that need approval and carries on, and its json and stream-json output have Claude Code's shapes. --max-turns ends with error_max_turns.
- neru mcp add takes claude mcp add's syntax (--transport, --scope, -e, -H) and there is neru mcp add-json.
- NERU_API_KEY connects a model in CI without neru login. Ctrl-C stops a reply cleanly, NO_COLOR is respected, and Linux servers without a display run neru inside Xvfb.
- Install and uninstall with one line on Windows, macOS and Linux (x64 and arm64), or with npm. Settings → CLI lists every command.

### Free models last longer

- On free plans Neru works within 64k tokens, shortens old tool output and paces requests to per-minute limits, so a free quota lasts much longer.
- Sub-agents on free plans run on a quick sibling model with its own quota, and switch models instead of failing.
- The next working model is chosen from every saved key at once, ranked by strength, recent failures and quotas. Failed models rest as long as the provider asked, remembered across restarts.

### Settings and session events

- Model switches, compactions and similar events show as quiet lines in the conversation, saved with the session, instead of a bar across the top.
- Settings open as a dialog with search and a grouped list, like Claude's.

### Imports, safety and more

- Settings → Imports brings over chats, skills, MCP servers and instructions from Claude Code, the Claude app, Codex, Cursor, VS Code, OpenCode and Gemini CLI.
- The agent's commands run in the operating system's sandbox, behind a rules gate that refuses destructive commands.
- Custom agents can edit files and run commands under the session's permissions.
- A guided walkthrough of every control, with a skip button.

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
