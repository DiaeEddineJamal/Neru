# Neru 練る

*To knead, to refine through repeated work.*

Neru is a local-first coding agent for Windows, macOS and Linux. It explores your project, proposes changes you read line by line, and never acts without asking. It is a [Tauri](https://tauri.app) app: a Rust core with a React interface, using the web engine your system already has.

## Download

Installers are on the [Releases](https://github.com/DiaeEddineJamal/Neru/releases) page:

| Platform | Files |
| --- | --- |
| Windows 10/11 | `Neru_x.y.z_x64-setup.exe` (recommended) or `.msi` |
| macOS 11+ | `.dmg` for Apple silicon (`aarch64`) or Intel (`x64`) |
| Linux | `.AppImage` or `.deb` |

The builds are not code-signed yet. Windows SmartScreen shows "More info → Run anyway"; on macOS, right-click the app and choose Open the first time.

## What it does

- **Works on real projects.** Reads, searches and edits files; creates, renames, moves and deletes files and folders. Every change shows a diff first and saves a checkpoint, so you can undo or rewind.
- **Permission modes.** Review every step, accept edits automatically, plan read-only, or run routine work on its own.
- **Any model.** Free keys from NVIDIA NIM, ModelScope, Google Gemini, Cerebras, Mistral and OpenRouter, local models through Ollama, or your own OpenAI, Anthropic, DeepSeek and other keys. When a free model hits its limit, Neru switches to the next best one.
- **Context that fits.** A live context meter shows the model's window and your key's remaining quota. Large files are read in ranges, older tool output is trimmed, and long sessions are summarized.
- **Parallel sessions** with Git worktrees, a review pane, pull requests and CI checks, MCP connectors, slash commands, on-device dictation, and an integrated terminal and preview.

## Build from source

Requirements: Node.js 22, Rust (stable), and the [Tauri prerequisites](https://tauri.app/start/prerequisites/) for your platform.

```sh
cd app
npm ci
npm run desktop                      # run in development
npm run tauri -- build               # installers for this platform, in src-tauri/target/release/bundle
```

Pushing a tag such as `v0.1.0` runs `.github/workflows/release.yml`, which builds every platform and attaches the installers to a draft release.

The installer artwork is generated from the app's design tokens: `python tools/build_installer_art.py`.

## Credits

Kneaded into shape by [Luziv](https://github.com/DiaeEddineJamal/Neru).
