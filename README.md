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

Neru updates itself: it checks this repository's releases when it starts and every few hours, and offers *Restart to update*. After updating, *What's new* shows the changes. The full history is in [CHANGELOG.md](CHANGELOG.md).

## Install the CLI

The desktop app already puts `neru` on your PATH. To install only the terminal CLI:

```sh
# macOS, Linux
curl -fsSL https://raw.githubusercontent.com/DiaeEddineJamal/Neru/main/install.sh | bash

# Windows (PowerShell)
irm https://raw.githubusercontent.com/DiaeEddineJamal/Neru/main/install.ps1 | iex

# Any platform with Node.js 18+
npm install -g neru-cli

# Windows
winget install Luziv.Neru.CLI
```

Then run `neru` in any project folder. It shares settings, keys and sessions with the app.

The scripts download the release archive for your platform, check its SHA256, and install to `~/.neru/cli` (linked from `~/.local/bin/neru`) or `%LOCALAPPDATA%\Neru\cli` (added to your user PATH). Pin a version with `bash -s -- 0.4.0`, or `$env:NERU_VERSION = '0.4.0'` on Windows.

On Linux, `neru` needs WebKitGTK 4.1, the same library the app uses. The script tells you if it is missing: `sudo apt install libwebkit2gtk-4.1-0`, `sudo dnf install webkit2gtk4.1`, or `sudo pacman -S webkit2gtk-4.1`.

**Update:** `neru update`, or `npm install -g neru-cli@latest`, or `winget upgrade Luziv.Neru.CLI`. Running the install script again also updates.

**Uninstall:**

| Installed with | Remove with |
| --- | --- |
| `install.sh` | `rm -rf ~/.neru/cli ~/.local/bin/neru` |
| `install.ps1` | `Remove-Item -Recurse "$env:LOCALAPPDATA\Neru\cli"`, then remove that folder from your user PATH |
| npm | `npm uninstall -g neru-cli` |
| winget | `winget uninstall Luziv.Neru.CLI` |

Settings and sessions stay in place, so the app keeps them.

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

### Releasing

1. Bump the version in `app/package.json`, `app/src-tauri/tauri.conf.json` and `app/src-tauri/Cargo.toml`.
2. Add the release to `app/src/changelog.json`. It feeds the in-app *What's new*, the GitHub release notes, and `CHANGELOG.md` (`node app/scripts/release-notes.mjs --changelog`).
3. Push a tag such as `v0.2.0`. The release workflow builds every platform, signs the update files with the `TAURI_SIGNING_PRIVATE_KEY` secret, and publishes the release, which installed copies then pick up.
4. The same run uploads the standalone CLI archives, publishes `neru-cli` to npm (with the `NPM_TOKEN` secret) and opens winget update PRs (with `WINGET_TOKEN`). The first winget submission is manual: see [packaging/winget](packaging/winget/README.md).

The installer artwork is generated from the app's design tokens: `python tools/build_installer_art.py`.

## Credits

Kneaded into shape by [Luziv](https://github.com/DiaeEddineJamal/Neru).
