# Local models: study and plan

*November 1, 2026. Neru on `main` after the free-model work (f854837). Checked against the code; paths are relative to `app/`.*

## The position

On a typical PC, local models are a **helper next to free cloud models**, not a replacement for them.

- A 1–4B model runs fast on any CPU but cannot carry multi-step tool use.
- The main agent loop needs at least a 20–30B-class model. That means 32 GB of RAM (mixture-of-experts models with about 3B active parameters, such as `qwen3-coder:30b`, run on a CPU at a usable speed) or a graphics card with 16 GB or more.
- What small local models do well costs free plans real requests: session titles, commit messages, summaries, repairing tool calls, embeddings and tab completion. Moving those to the machine saves cloud quota for the work that needs it, and keeps it private.

So the plan makes local models easy to get and puts them to work as helpers first. The main loop runs locally only where the hardware allows it.

## What Neru has today

| Area | Where | State |
| --- | --- | --- |
| Ollama as a main provider, native `/api/chat` with a stepped `num_ctx`, `keep_alive: 30m` | `src-tauri/src/providers.rs:251` | Works |
| Installed models' capabilities and context size (`/api/show`) | `src-tauri/src/models.rs:1030` | Works |
| Lean prompt and tool set for local and small models; text tool protocol | `src-tauri/src/toolproto.rs` | Works |
| Doctor check for flash attention and a quantized KV cache | `src-tauri/src/extras.rs:584` | Advice only |
| On-device dictation and embeddings (Transformers.js, ONNX) | `src/lib/speech`, `src/lib/semantic` | Works |
| Getting a model: install Ollama, run `ollama pull` in a terminal | `src/providerCatalog.ts:99` | Manual |

## Gaps

1. **No hardware awareness.** Nothing measures RAM, the graphics card or free disk space, so nothing can say which model fits or whether a local model can drive the agent.
2. **No model library.** Downloading a model means a terminal, a tag the user has to know, and no size or progress shown.
3. **Local models never help unless they are the main model.** The helper lane (`fallback::small_model`) only considers the main model and saved keys, and Ollama has no key. With Ollama running beside a free cloud model, titles, summaries and repairs still spend cloud requests.
4. **Tool-call repair is unconstrained.** On Ollama, the repair request could use `format` with the tool's JSON schema so the answer is always valid JSON.
5. **No speed check.** Nobody learns whether their machine reads a prompt at 50 or 500 tokens a second until a reply crawls.

## Plan

Sizes: S = 1–2 days, M = 3–5 days, L = more.

### L0. Hardware profile and recommendations (S) ✅ done

`hardware.rs` with `local_advice`, a "Local models" line in `neru doctor`, and `localModels` in the registry (now version 2). On the test machine it reports: 32 GB memory, no graphics card with its own memory; `qwen3-coder:30b` for the agent "slowly, about 14 tokens a second"; helper `gemma3:4b`; completion `qwen2.5-coder:1.5b`; embeddings `nomic-embed-text`. Speeds are estimates from memory bandwidth (28 GB/s for a CPU with AVX2, 250 for a graphics card, 100 for Apple silicon) until L4 measures them. The catalog's sizes are approximate and live in the registry, so they can be corrected without a release.

Original plan:
- Detect RAM, CPU (threads, AVX2/AVX-512), graphics cards with their dedicated memory, Apple-silicon unified memory, and free space on the data drive. On Windows this uses the registry's 64-bit `HardwareInformation.qwMemorySize`, because WMI's `AdapterRAM` stops at 4 GB.
- A curated local catalog in the model registry (`registry/models.json`, so it updates without a release): Ollama tag, download size, roles (helper, code, completion, embeddings) and the memory each needs.
- `local_recommendations`: for each role, the best model that fits, plus a plain verdict (helpers only, or the agent runs locally).

### L1. Model library in the app (M)
- Ollama status (installed, running, version), installed models with sizes, and the recommended models not yet installed.
- Download with progress (`/api/pull` streams it), cancel and remove (`/api/delete`), with a disk-space check before each download.
- When Ollama is missing, a link to ollama.com/download and a check that finds it once it is installed. Neru does not run the installer itself.

### L2. Local helpers (S)
- When Ollama runs and has a suitable small model, the helper lane uses it first, before any cloud model, for titles, summaries, commit messages and repairs. It stays within the privacy rule, which local always satisfies.
- Eval: a scenario with a local helper available shows its chore requests leaving the cloud count.

### L3. Constrained repair and JSON chores on Ollama (S)
- The repair request sends the tool's parameter schema as `format`, so the answer always parses.

### L4. Speed check (S)
- After a download, a short benchmark through Ollama's own timings (`prompt_eval_duration`, `eval_duration`) gives prompt and generation speed and a verdict: fine for the agent, helpers only, or too slow.

### L5. Bundled engine (L, decide later)
- A `llama-server` sidecar downloaded on demand would remove the Ollama dependency and give control over KV-cache reuse, speculative decoding and grammars. Decide once L1–L4 show how people use local models.

## Order

L0 → L2 → L1 → L3 → L4. L0 first, because recommendations need it. L2 next: it is small and saves cloud requests for everyone who already runs Ollama. Then the library UI.

## Test machine

The development machine has an i7-1185G7 (4 cores, AVX-512), 32 GB of RAM and Intel Iris Xe integrated graphics: no dedicated graphics memory. That is a good stand-in for the low-end case. Ollama is not installed on it, so Ollama behaviour is tested against a scripted server until it is.
