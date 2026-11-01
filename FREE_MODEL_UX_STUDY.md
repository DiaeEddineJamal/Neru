# Free-model UX: study and plan

*November 1, 2026. Neru v0.12.2. Checked against the code; paths are relative to `app/`.*

On free tiers, the limit people hit is **requests per day** and **requests per minute**, not tokens or model quality. A great free experience therefore comes down to three things:

1. A first answer within a minute of installing.
2. As few model requests as possible per finished task.
3. Waits and switches that the user understands instead of ones that look broken.

## What Neru already does (don't rebuild)

| Area | Where | State |
| --- | --- | --- |
| Switching between providers, health saved to disk, circuit breaker | `src-tauri/src/fallback.rs` | Strong |
| Quota headers, daily/monthly blocks, preflight wait-or-switch | `src-tauri/src/limits.rs:317` | Strong |
| Text tool protocol, tool-name repair, per-model profile and demotion | `src-tauri/src/toolproto.rs` | Strong |
| Lean prompt and tool set for small or local models | `toolproto.rs:361` | Works, chosen by name guessing |
| Architect/editor split on free plans | `src-tauri/src/engine.rs` | Works |
| Small projects: every file sent with the first request | `src-tauri/src/agent.rs:390` | Works |
| Project map in the system prompt, stable so it stays cached | `agent.rs:2208` | Only when budget ≥ 32K and not lean |
| Edits tolerant of whitespace and indentation, closest-line hint on failure | `src-tauri/src/tools.rs:406`, `:476` | Works |
| Prompt caching (Anthropic marks, `prompt_cache_key`) | `src-tauri/src/providers.rs:235` | Works |
| Quotas left per key | `src/components/neru/SessionPanel.tsx:51` | Only inside the Session panel |

Two corrections to the earlier brainstorm:
- **No real zero-key path exists.** LLM7 now needs a token, so OVHcloud is the only keyless provider, at 2 requests/min per model. A Code task takes 10–20 requests, so that is 5–10 minutes: fine for a first Chat message, too slow for Code.
- **Fuzzy edit apply already exists.** What remains is making a failed edit cost no extra read (see 3b).

## Gaps found

1. **Nothing measures requests per task.** `usage.rs` counts tokens and replies, but not tool rounds, failed edits, parse failures, waits or switches per user message. Every item below claims to save requests, and today there is no way to check that.
2. **Getting a key is the whole onboarding.** The Model step (`Onboarding.tsx:217`) means visiting a site, signing up and pasting a key. Skipping it leaves Neru unable to answer.
3. **Medium and large projects get only the map.** The model spends its first 3–6 requests on `project_map`, `search_text` and `read_files` before it starts work. Lean (small) models get no map at all, though they are the worst at exploring.
4. **A failed edit usually costs a re-read.** `not_found_message` shows at most 4 lines starting from the first line that matches, so the model often calls `read_file` before retrying.
5. **Capacity is invisible.** Users can't see how much free work is left today across all keys, or what a task is likely to cost, until a limit is hit.
6. **Model knowledge ships in the binary.** `coding_score` (`fallback.rs:587`), `lean()`, the presets and free limits change monthly but update only with a release.

## Plan

Sizes: S = 1–2 days, M = 3–5 days.

### P0. Measure requests per task (S) ✅ done

Built in `src-tauri/src/taskstats.rs`. Every request goes through `providers::chat_request`, and a tokio task-local ties it to the user's message, so editor, repair and sub-agent requests count too. Results appear in Session panel → Task cost and as `task` in `neru -p --output-format json`. `app/scripts/eval-requests.mjs` runs 10 scripted scenarios against the CLI, as a lean (local) and a full model, and fails when any of them costs more requests than `eval-requests.baseline.json`. It runs in the release workflow after the smoke test. Baseline: 48 requests across the 20 runs. Not counted: chores outside a task, such as session titles and learning memories after a chat.

Found while building it: a tool call with a misnamed argument (`search_text` with `pattern` instead of `query`) costs a whole repair request. Mapping common aliases without a model call belongs in 3b.

Original plan:
- Record per user message: requests, tool rounds, failed edits, parse failures, waits (seconds), switches, tokens in/out/cached. Store locally next to `usage.json`; it never leaves the machine.
- Show it in the Session panel ("This task: 9 requests, 1 failed edit, 2 switches").
- Add a developer eval: about 10 scripted tasks run against a recorded or fake provider in tests, failing if the request count regresses.
- **Done when:** every later item can say "saves N requests on the eval set".

### P1. A first answer in under a minute (M) ✅ done

- **Starter model.** `fallback::needs_starter` and `starter_model`: when nothing is really connected (a provider without its key, or the fresh install's default local gateway with nothing listening on port 3001), Neru answers with the best keyless OVHcloud model and says so: about 2 requests a minute, plus how to add a free key. Checked for real: a fresh data folder answered in 2.4 s with `gpt-oss-120b`. It falls back to the old error when the privacy rule keeps work off cloud models.
- **Found on the way:** a fresh install's default (local gateway, `localhost:3001`) counted as "connected". Skipping the Model step meant the first message failed with "connection refused", and onboarding showed "Connected to Local gateway". Onboarding now starts from the first recommended free provider.
- **NVIDIA (the first recommended provider) has no default model,** so connecting failed with "Enter a model name". Onboarding now picks the best coding model the key can call from the provider's list.
- **Key paste** instead of reading the clipboard (no permission needed, nothing read in the background). Opening a provider's key page and returning to the window puts the cursor in the key field. Pasting connects at once, and a key with a known prefix (`nvapi-`, `AIza`, `sk-or-`, `gsk_`, `sk-ant-`, `hf_`, `xai-`) switches to its provider first.
- **More keys hint:** when a model runs out with nothing to switch to, the error now lists the providers that really are free (Cerebras had moved to paid) and explains that each has its own daily allowance.
- **Also fixed:** `coding_score` matched sizes as substrings, so Qwen3.8-27B ranked as a 7B model and DeepSeek 671B as a 1B one. Sizes now match as whole words.

Original plan:
- If the Model step is skipped, OVHcloud keyless answers Chat right away, labelled honestly as slow.
- Make the Model step one recommended path (Gemini or NVIDIA): one button opens the key page. When the window regains focus, Neru checks the clipboard **once** for a known key prefix (`AIza`, `nvapi-`, `sk-or-`, …), offers to use it, probes it and moves on. Never silent, never in the background.
- At the first daily limit, show: "Add a second free key to roughly double your daily work". The nudge comes when the user feels the limit, with a real number from P0.
- **Done when:** a new user goes from install to first Code answer in under 2 minutes with one free key.

### P2. Show capacity and waits (S–M)
- A small meter by the composer showing today's free capacity across all keys (requests left, next reset), built from the quota ledger that already exists.
- Wait and switch states inline in the conversation with an ETA: "Gemini resets in 4 min; continuing on NVIDIA Qwen3 Coder."
- Once P0 has data: an estimate before a large task ("about 15 requests; 140 left today").

### P3. Fewer requests per task
- **3a. Context pack for medium and large projects (M).** ✅ done
  - `contextpack.rs`: from the session's first request, in projects too large for the whole-project snapshot, it takes paths, identifiers (`snake_case`, `camelCase`, quoted code) and distinctive words. It looks them up in the project index: named files whole, symbol definitions with about 40 lines, and text matches for terms found in 12 files or fewer. The best six files go in as whole files or `read_file`-style ranges, within 2K tokens (lean), 4K (free plan) or 8K (paid). `.env` files, lockfiles and attached files are left out. Lean runs also get a short folder outline, since their prompt has no project map.
  - It uses the index rather than semantic search, which embeds in the window: unavailable in the CLI and slow on first use.
  - Eval: new `find-edit` scenario (a 31-file project, a request naming a function) went from 4 to 2 requests in both profiles; total 66→62.
  - Cost: the pack travels with every request until compaction. It is capped, and it is empty when nothing in the request matches the project.
  - Not yet: follow-up messages in a session get no pack; semantic search could add matches once its index is warm.
- **3b. Failed edits and tool calls that need no extra request (S).** ✅ done
  - `toolproto::fill_aliases` renames misnamed arguments (`pattern`→`query`, `filePath`→`path`, `old`→`old_string`, …) before asking for a repair request. It only renames when exactly one argument fits and the name is not one of the tool's own parameters.
  - `tools::not_found_message` scores whole blocks, anchored on their first, last and longest lines, and returns the closest block with two lines either side, numbered, telling the model it can copy from there without reading the file.
  - Eval: two new scenarios (`alias-args`, `edit-miss-block`) each cost one request less: 3→2 and 4→3, in both profiles. Across the 24 runs: 62→58.
- **3c. File summaries cached by content hash (M).** Store a one-line purpose and key exports per file, written on the small-model lane or from tree-sitter, keyed by hash and reused across sessions. These feed the map and the context pack.

### P4. Model registry updated without a release (M)
- `models.json` in the Neru repo, signed with the existing updater key, fetched at startup with a copy built into the app as fallback. It holds family scores, lean/small flags, free limits, tool support, key prefixes, recommended onboarding provider and preset fixes.
- `coding_score`, `lean()` and the free limits read from it; today's constants become the built-in copy.
- Later, a separate decision: opt-in anonymous health sharing across installs, which needs a backend.

## Suggested order

P0 → 3b → P1 → 3a → P2 → P4 → 3c. P0 comes first because it makes every other item measurable. 3b is the smallest win with immediate effect.

## Open decisions

1. Is a one-time clipboard check after "Get key" acceptable?
2. Keyless OVHcloud: Chat only, or Code too with a clear "slow" label?
3. Registry: hosted as a file in this repo and signed with the updater key?
4. Request metrics: local only for now?
