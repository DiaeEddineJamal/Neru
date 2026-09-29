# Neru

Neru is a local desktop coding workspace built with Tauri 2, Rust, React, TypeScript, and Monaco. Its visual identity follows the supplied Neru brand board: charcoal and paper themes, moss accents, and the blinking shadowed mascot.

## Run on this Windows workstation

From PowerShell:

```powershell
cd D:\Neru
. .\Use-NeruStorage.ps1
.\Run-Neru.ps1
```

The storage script puts temporary files, npm and Cargo caches, Rust build output, and Neru data under `D:\Neru\.local`. The Windows app also uses that D: data location when launched directly from its executable. Development needs the installed Visual Studio C++ tools; the Windows SDK and Rust toolchain used here are under `D:\Neru\.local`.

For a production Windows executable and NSIS installer, run `D:\Neru\tools\windows-toolchain.cmd build` after dot-sourcing the storage script. Build artifacts are under `D:\Neru\.local\build\rust\release`.

## Use

On first launch, Neru opens a four-step guide for the workspace, project folder, model connection, and review flow. The guide can be skipped and replayed later from Settings. Its transitions respect the system's reduced-motion preference.

Open a local project or clone a Git repository into a D: folder. The workspace includes a file explorer and editor, project search, terminal, Git status and diffs, staging and commits, and an agent chat. Chats are saved under `D:\Neru\.local\data\sessions` and restored with the last project when the app reopens. Use the sidebar to switch, rename, or delete chats.

The composer can attach up to five text files from the open project; type `@` or use the paperclip to find them. Plan mode reads the project and proposes an approach. Review mode lets the agent propose file edits, npm tasks, and exact PowerShell commands for approval. Project-level `AGENTS.md`, `CLAUDE.md`, and `.neru/instructions.md` are loaded into a new chat when present. Stop a model request with the square button or Escape. File edits and shell commands proposed by the agent require approval. Neru creates a checkpoint for an approved edit and offers undo in the status bar. The Review code action asks the agent to inspect current Git changes.

Configure an OpenAI-compatible endpoint, model, and optional API key in Settings. FreeLLMAPI can be used through its compatible endpoint. The key remains in the process memory and is cleared when Neru exits. The model service itself must be available for chat to work.

The `http://127.0.0.1:1420/` browser view is a visual preview during development. Filesystem, terminal, Git, and agent commands run in the native Tauri window.

## Assets

- `D:\Neru\assets\neru-app-icon.png`: transparent, large moss Windows app icon source
- `D:\Neru\assets\neru-app-icon-tile.png`: previous tile version retained for comparison
- `D:\Neru\assets\neru-mascot-cutout.png`: transparent mascot and ground shadow
- `D:\Neru\app\src-tauri\icons`: generated Windows and other platform icon sizes

The light and dark themes use the same transparent mascot art. Its eyes are drawn separately so they stay light when the theme changes. Click or press Enter on the home or sidebar mascot to trigger a random look, twinkle, hop, or sway. There is no hover reaction or selected frame. The eyes blink occasionally. Reduced-motion preference disables the animation.
