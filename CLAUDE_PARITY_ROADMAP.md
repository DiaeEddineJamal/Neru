# Neru capability roadmap

Neru is a local coding agent. This tracks the implementation work identified in the Claude Desktop Code comparison, without treating every Claude ecosystem feature as part of the first release.

## Implemented in the first pass

- Saved project conversations under `D:\Neru\.local\data\sessions`, with restoration after app restart, switching, rename, and delete.
- Pending edit/task approvals are saved with their conversation.
- Stop a model response while its provider request is running.
- Attach up to five project text files to a prompt through a searchable picker or `@` entry point.
- Load top-level `AGENTS.md`, `CLAUDE.md`, and `.neru/instructions.md` into a new agent conversation.
- Review and Plan modes; Plan exposes only read tools.
- Agent can propose an exact PowerShell command for approval, with Neru storage variables sourced from D: before execution.
- Read Git diffs in the agent and request a focused code review from the app toolbar.

## Next implementation order

1. Isolated parallel sessions using Git worktrees, then session grouping and background progress.
2. Streaming agent events, queued steering, cancellable tools, and a visible task timeline.
3. Multi-file patch proposals, file-by-file visual diff review, and inline feedback.
4. Image/PDF prompt attachments and context usage controls.
5. Integrated app preview and automated browser verification.
6. GitHub pull-request creation, checks, review feedback, and CI status.
7. Skills, plugins, and MCP connectors with scoped permissions.
8. Resizable panes, multiple terminal tabs, and workspace layouts.
9. Optional SSH/cloud execution, scheduling, and cross-device continuation.

## Verification notes

- The first pass has a React production build, lint run, Rust check, and a Rust persistence test.
- Native interaction and model-provider behavior still need end-to-end checks in the rebuilt desktop executable.
- Features in the next implementation order are not yet available in Neru.
