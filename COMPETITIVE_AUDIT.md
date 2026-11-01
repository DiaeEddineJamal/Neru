# Competitive audit: Neru vs. Windows coding agents

*November 1, 2026. Neru v0.11.0.*

Neru's side is checked against the code; the evidence paths are in [Neru today](#neru-today-what-is-already-strong). The competitor side comes from official docs where they could be read. Many 2026 changelogs were truncated or moved, so some competitor cells come from secondary trackers. Free-tier numbers change often. Neru should read them at runtime, not hardcode them.

**Compared against:**
- Claude Code (CLI and desktop)
- Codex, now inside the ChatGPT desktop app
- Cursor
- Devin Desktop (formerly Windsurf)
- Cline, Kilo Code, OpenCode, Aider
- Zed, GitHub Copilot (VS Code)
- Goose, Continue
- Antigravity and Gemini CLI
- Warp, Trae, Kiro, JetBrains Air

**Market changes this year:**
- Roo Code shut down in May 2026.
- Windsurf became Devin Desktop in June 2026.
- Codex merged into the ChatGPT app in July 2026, and its Free plan includes it.

---

## Neru today: what is already strong

These are rare among competitors. Keep them, and lead with them in marketing.

- **Automatic fallback across providers.**
  - When a model is rate-limited, out of credits or down, Neru switches to another model on any saved key.
  - Choices are ranked by `switch_score`, with health saved to disk (`fallback.rs`).
  - Only Claude Code (`--fallback-model`) and Goose (lead/worker) do anything close. Nobody else does it across providers.
- **Quota awareness.**
  - Reads rate-limit headers and OpenRouter `/key`, then paces requests and waits out short limits (`limits.rs`).
  - A "tonight" queue runs work after free quotas reset.
- **Privacy routing.**
  - A no-train or local-only rule applies globally or per project, fallback included.
  - Secrets are redacted before reaching cloud models and restored in replies (`privacy.rs`).
- **Second-opinion review.** A model from a different family reviews each pending edit (`review.rs`).
- **Team.**
  - Runs 15 external agent CLIs plus Neru in one shared thread.
  - Handoffs, artifacts, tickets and worktrees.
  - Closest equivalents: JetBrains Air, Devin Desktop, Zed and Warp Oz, which do this over ACP.
- **Claude Code parity.** Permission modes, plan mode, subagents, checkpoints and rewind, hooks, skills, slash commands, MCP with OAuth, phone remote, session replay sharing, and a headless `-p` CLI with Claude-compatible JSON.

---

## Part 1: Missing features

### Tier 1: high impact, and it fits Neru's free-model focus

Each item below follows the same pattern:
- **Who has it:** the competitors that ship it.
- **Neru today:** what Neru has now.
- **Why it matters:** what it would buy.
- **Build:** what it would take.

1. **Diagnostics after edits, plus an automatic fix loop.**
   - **Who has it:**
     - Cursor, Devin, Cline, Kilo, OpenCode, Zed, Copilot and Kiro feed LSP or compiler errors back to the agent.
     - Aider has `--auto-lint` and `--auto-test`.
   - **Neru today:** only `run_project_task` when the model decides to call it. No LSP and no lint gate at edit time.
   - **Why it matters:** SWE-agent's ablation found the edit-time lint gate was the single most important part of its interface. Weak free models gain the most from it.
   - **Build:**
     - Run a fast syntax or lint check on each applied edit and return errors with the tool result.
     - Then start language servers per project, as OpenCode does, and attach new diagnostics after edits.
     - Add an optional "test until green" loop with a retry cap.

2. **A better repo map.**
   - **Who has it:** Aider builds a tree-sitter map ranked PageRank-style by references. Cline and Kilo have tree-sitter definitions.
   - **Neru today:** `index.rs` extracts symbols with regex, and `project_map` is a flat outline.
   - **Why it matters:** on free tiers, *requests per day* is the binding limit. A good map replaces exploratory read and search turns.
   - **Build:**
     - Parse with tree-sitter (signatures, not only names).
     - Build a reference graph and rank it, so the map shows the files that matter to the current task within about 1–2K tokens.

3. **Semantic code search with embeddings.**
   - **Who has it:** Cursor, Devin, Kilo, Copilot, Warp and Trae.
   - **Neru today:** none. Embedding models are only filtered out of model lists.
   - **Why it matters:** it finds code by meaning, which is fewer tool rounds again.
   - **Build:**
     - Embed chunks locally: an Ollama embedding model, or a small ONNX model in-process, since the app already ships Transformers.js.
     - Or use a free cloud tier (Cloudflare Workers AI neurons, Gemini embeddings).
     - Add a `semantic_search` tool, and respect the privacy rule.

4. **Interactive browser verification.**
   - **Who has it:** Claude Code Desktop, Codex, Cursor, Antigravity (with recordings) and Copilot click, type and screenshot the running app.
   - **Neru today:** `check_preview` returns title, text and console errors, with no clicking or typing. The roadmap already lists this.
   - **Build:**
     - Add `preview_click`, `preview_type` and `preview_screenshot` through the existing preview bridge.
     - Send screenshots to vision models only.

5. **ACP (Agent Client Protocol), as an agent and as a host.**
   - **Who has it:**
     - Zed, JetBrains Air, Devin Desktop and Warp host agents over ACP.
     - Cline and OpenCode can run inside them.
   - **Neru today:** Team drives other CLIs through each one's own interface.
   - **Why it matters:** as an ACP *agent*, Neru with its free-model fallback becomes usable inside Zed and JetBrains, which reaches new users at low cost. As an ACP *host*, Team gets a single protocol in place of 15 adapters.

6. **A Kanban board for parallel worktree sessions.**
   - **Who has it:** Cline Kanban, the Kilo Agent Manager and the Antigravity Manager view.
   - **What it adds:** one card per worktree, with status, dependencies, one-click commit or PR, and merge.
   - **Neru today:** has the parts (worktrees, sessions, PR and CI), but no board.

### Tier 2: valuable, more work

7. **Inline edit (Ctrl+K in the editor) and tab completion.**
   - Every IDE-style competitor has these.
   - Neru has a Monaco editor but neither feature.
   - Mistral's free plan includes Codestral fill-in-the-middle, which could power free completions.
8. **LLM risk-scored auto-approval.**
   - VS Code "Assisted permissions", Claude Code Auto mode and Cursor auto-review use a model to judge whether an action is safe.
   - Neru's Auto mode allows a command only when every part is on the allowlist.
   - A cheap model could judge everything off the allowlist.
9. **Event-driven and background automations.**
   - Cursor Automations, Continue Mission Control and Warp Oz start runs from a CI failure, a PR, a webhook or cron.
   - Neru's queue runs only while the app is open.
   - A small background service, or a Windows Task Scheduler entry that calls `neru -p`, would close most of the gap.
10. **Checks as files in the repo.**
    - Continue keeps `.continue/checks/*.md`: pass or fail checks that run on each PR and locally.
    - Neru has `/security-review` and hooks; a `.neru/checks/` folder would fit naturally.
11. **Reviewable artifacts in solo chat.**
    - Antigravity has plans and walkthroughs, with screenshots, that you comment on.
    - Neru has artifacts in Team only.
12. **MCP gaps:**
    - resources and prompts
    - MCP Apps, interactive widgets in chat (Goose, Claude Code)
    - a plugin or extension marketplace (Cline, Kilo, OpenCode, Copilot, Cursor and Claude Code)
13. **A Windows write sandbox.**
    - Codex ships its own Windows sandbox.
    - Neru's job object limits resources, not folders. AppContainer or a low-integrity token is on the roadmap.
14. **Smaller items:**
    - side chat (`/btw`)
    - cross-session messaging
    - multi-model side-by-side compare (Kilo)
    - spec-driven mode, with requirements, design and tasks (Kiro; Neru's Team has specs, solo chat does not)
    - an output-style picker
    - customizable keyboard shortcuts
    - notebook editing

### Low priority

- Desktop computer use on Windows (Codex, plus Claude Code in preview): high effort and high risk.
- Cloud agents: they conflict with being local-first.

---

## Part 2: Missing free-model optimizations

Ranked by impact for an agent that relies on many free providers. Each item lists what Neru has now and what is missing.

### 1. Keep provider prompt caches warm

**Neru has:**
- `cache_control` marks for Anthropic and OpenRouter `anthropic/*` models.
- Automatic prefix caching on other providers, which depends on an append-only history.

**Missing:**
- **History edits that may break the cache.**
  - `fit_context` runs before every round, and both `age_tool_outputs` and `drop_stale_reads` change older messages.
  - Any change in the middle of the history invalidates the provider's prefix cache from that point on.
  - *Needs measuring.* If confirmed, batch trimming so it happens only at compaction, not every round.
  - This matters most on Cerebras: cached tokens don't count against its 30K uncached-tokens/minute limit, so a warm cache roughly triples the budget.
- **No sticky routing.**
  - OpenRouter keeps a request on the same upstream provider for 10 minutes when it carries a `session_id`, the `x-session-id` header or a `prompt_cache_key`. Neru sends none of these.
- **No reading of cached-token counts** (`cached_tokens` in usage).
  - Neru can't see cache hits, so `switch_score` can't value staying on a warm model. It gives a bonus only for staying on the same key.
- **Model order in the prompt.**
  - Keep the system prompt, tools and project map first and byte-stable.
  - Gemini caches implicitly only above 2,048 tokens (2.5) or 4,096 tokens (3.x/4).

### 2. A small-model lane for chores

**Neru has:** `helper_model` for read-only sub-agents and `reviewer_model` for second opinions.

**Missing:**
- A small or fast model slot (OpenCode's `small_model`) for:
  - session titles
  - commit and PR messages
  - compaction summaries
  - intent and relevance checks
  - tool-call repair
- Good candidates: Groq, Cerebras, Gemini Flash, or a local model.
- Why: every chore that runs on the strong model spends its daily request quota.

### 3. Strong model plans, cheap model applies (architect/editor)

- **How others do it:**
  - Aider: a strong model writes the change in prose, and a fast model turns it into edits. Its benchmarks show gains of 5–10 points even when weak models are paired.
  - Goose: the strong model takes the first turns, then a worker model takes over, escalating back after 2 consecutive failures.
- **Neru:** none.
- **What it would take:** Neru already has plan mode and `helper_model`. Wiring "plan with A, execute with B, escalate to A on failure" is a contained change.

### 4. Reliability shims for weaker and odd models

**Neru has:**
- `recover_text_tool_calls`, which handles `<tool_call>` blocks written as text.
- Tolerant JSON argument parsing.
- Models without tool support are filtered out.

**Missing:**
- **A prompt-based tool protocol** (XML or JSON in plain text) for models without native tools.
  - This would open many free models that are currently excluded.
  - Roo Code dropped its XML fallback and broke vLLM and SGLang endpoints. Keep a switch per model.
- **Tool-call repair.**
  - Fuzzy-match tool names.
  - Re-ask a cheap model to fix arguments that fail the schema (OpenCode does this through the AI SDK).
  - Return an "invalid tool call" error to the model instead of failing the turn.
- **Per-model settings learned from health data.** For example, after N parse failures:
  - switch the model to the text protocol, or
  - switch from search/replace edits to whole-file writes for small files, or
  - send it a compact prompt.
  - Aider ships `diff-fenced` for Gemini; Cline ships a compact prompt for local models.

### 5. Plan ahead with a full quota ledger

**Neru has:**
- Header parsing (`x-ratelimit-*`, `anthropic-ratelimit-*`) and OpenRouter `/key`.
- Short-limit waits and a TPM shrink for Groq.

**Missing:**
- **Separate windows per (provider, model, key):**
  - requests per minute
  - tokens per minute
  - requests per day
  - tokens per day
  - Cerebras's hourly token cap
- **The right reset clock for each:**
  - Gemini resets at midnight Pacific and sends no headers, so Neru must count locally.
  - OpenRouter's free cap resets at UTC midnight.
- **Switch before the 429.** If the estimated prompt tokens exceed the remaining TPM (Groq 8K, Cerebras 30K uncached), choose another model up front instead of failing first.
- **Gemini limits are per Google Cloud project, not per key.** Extra keys in one project add nothing; Settings should say so. Note that multi-account farming breaks some providers' terms, so leave it to the user.
- **Full-jitter backoff that always honors `Retry-After`.** Neru has jitter; check that it uses the header as a minimum.

### 6. Local models (Ollama)

**Neru has:** an Ollama preset on the OpenAI-compatible `/v1` endpoint.

**Missing** (confirmed by code search: none of these options appear anywhere):
- **Context size.**
  - Ollama defaults to **4,096 tokens**, so Neru's system prompt, tools and history get silently truncated.
  - The OpenAI-compatible endpoint can't set it per request. Either:
    - call the native `/api/chat` with `options.num_ctx`, or
    - create a model variant with a larger `num_ctx`, or
    - tell the user to set `OLLAMA_CONTEXT_LENGTH`.
- **`keep_alive`.** The default unloads the model after 5 minutes idle; reloading adds seconds to each first turn.
- **Setup advice in Neru's doctor or onboarding:**
  - `OLLAMA_FLASH_ATTENTION=1` plus `OLLAMA_KV_CACHE_TYPE=q8_0` roughly doubles usable context on the same VRAM.
  - For llama.cpp users, n-gram speculative decoding speeds up edit-heavy output.

### 7. A leaner prompt for weak models

**Neru has:**
- On-demand MCP tool loading (`find_connector_tools`).
- Read-only tools in plan mode.
- A 64k "economy" budget.

**Missing:**
- A compact system prompt and minimal tool set for local and small models. Cline's compact prompt is about 90% smaller.
- Tools scoped per step, so weak models choose the right tool more often.

### 8. Structured compaction

**Neru has:** summarizes older turns and keeps the last two requests.

**Missing:**
- **A fixed template:** goal, decisions, files touched, open TODOs, failing tests.
- **A reserved token buffer**, so compaction itself never overflows (OpenCode `reserved`).
- **Running it on the small-model lane** (see section 2).

### 9. Real token counting

- **Neru now:** estimates about 4 characters per token when the provider doesn't report usage.
- **Fix:** a real tokenizer for the main model families (`tiktoken`-style for OpenAI, Qwen, Llama and DeepSeek) makes the context and TPM checks above accurate, especially for code, where the 4-character estimate is often off.

### 10. Update the provider catalog

| Change | Why |
| --- | --- |
| Mark the **Hugging Face router** as paid, not free | Free accounts now get $0 in monthly credits (PRO gets $2) |
| Add **Cloudflare Workers AI** | 10,000 neurons/day free (about 100–150K tokens with qwen2.5-coder-32b), plus free embeddings |
| Add **SambaNova** as fallback only | Free, but 20 requests/day and 200K tokens/day |
| Add **Z.ai GLM-4.7-Flash** (free) | Free on the direct API and as `z-ai/glm-4.7-flash:free` on OpenRouter, with 200K context |
| Use **Mistral Codestral** for completions | Its free plan includes fill-in-the-middle (FIM); see Part 1, item 7 |
| Note **Groq's 8K tokens/min** | Too small for full agent turns; good for the small-model lane |
| Note **Cerebras: 5 requests/min, 1M tokens/day** | The cached vs uncached split makes caching worth roughly 3× here |
| Check **NVIDIA NIM** credits | About 40 requests/min and 1,000 starting credits (forum source); limits vary by model and load |
| Do **not** add Gemini CLI or Qwen Code sign-in quotas | Gemini CLI's free Google-account tier moved to Antigravity CLI on June 18, 2026; Qwen's free OAuth tier was cut and may close |
| GitHub Models is retired (July 30, 2026) | Neru doesn't list it; don't add it |

---

## Suggested order

Ranked by impact for the effort, given Neru's free-model focus:

1. Edit-time lint gate, then LSP diagnostics. *(Part 1, item 1)*
2. Cache-friendly history plus OpenRouter session stickiness. Measure first. *(Part 2, section 1)*
3. Small-model lane. *(Part 2, section 2)*
4. Ollama `num_ctx` and `keep_alive`. *(Part 2, section 6; small change, big local win)*
5. Tree-sitter ranked repo map. *(Part 1, item 2)*
6. Text tool protocol, tool-call repair, and per-model auto-demotion. *(Part 2, section 4)*
7. Architect/editor split. *(Part 2, section 3)*
8. Full quota ledger with up-front TPM checks. *(Part 2, section 5)*
9. Catalog updates. *(Part 2, section 10; quick)*
10. Interactive preview verification, then semantic search, then ACP.

---

## Sources

**Rate limits and pricing:**
- OpenRouter: https://openrouter.ai/docs/api-reference/limits
- OpenRouter prompt caching: https://openrouter.ai/docs/features/prompt-caching
- Groq: https://console.groq.com/docs/rate-limits
- Cerebras: https://inference-docs.cerebras.ai/support/rate-limits
- SambaNova: https://docs.sambanova.ai/docs/en/models/rate-limits
- Cloudflare Workers AI: https://developers.cloudflare.com/workers-ai/platform/pricing/
- Hugging Face Inference Providers: https://huggingface.co/docs/inference-providers/pricing
- Gemini API: https://ai.google.dev/gemini-api/docs/rate-limits
- Gemini CLI quota: https://geminicli.com/docs/resources/quota-and-pricing/
- Ollama FAQ: https://docs.ollama.com/faq

**Techniques:**
- Aider repo map: https://aider.chat/docs/repomap.html
- Aider edit formats: https://aider.chat/docs/more/edit-formats.html
- Aider architect/editor: https://aider.chat/2024/09/26/architect.html
- Goose lead/worker: https://block.github.io/goose/docs/tutorials/lead-worker
- OpenCode config: https://opencode.ai/docs/config/
- LiteLLM routing: https://docs.litellm.ai/docs/routing
- SWE-agent: arXiv 2405.15793
- RouteLLM: https://lmsys.org/blog/2024-07-01-routellm/

**Competitors:**
- Claude Code Desktop: https://code.claude.com/docs/en/desktop
- Cursor changelog: https://cursor.com/changelog
- VS Code updates: https://code.visualstudio.com/updates
- Zed AI: https://zed.dev/docs/ai/overview
- Continue checks: https://docs.continue.dev/mission-control/beyond-checks
- Cline Kanban: https://cline.bot/blog/announcing-kanban
- Devin Desktop: https://devin.ai/blog/windsurf-is-now-devin-desktop
- Codex on Windows: https://www.neowin.net/news/openai-rolls-out-major-codex-for-windows-update-with-computer-use-and-mobile-access/

**Least certain:** Kiro, Trae and Antigravity internals, Cursor memories, and exact pricing for Antigravity, Kiro, Trae and Warp.
