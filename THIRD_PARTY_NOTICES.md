# Third-party notices

## llama.cpp (built-in local engine)

Neru's built-in engine downloads prebuilt `llama-server` archives from the
[ggml-org/llama.cpp](https://github.com/ggml-org/llama.cpp) releases (pinned build `b12230`, each archive
checked against its published SHA-256) the first time a local model runs. llama.cpp is MIT-licensed:
Copyright (c) 2023-2026 The ggml authors. The archives carry their own LICENSE files, which are kept where
they are unpacked (`<data>/local/engine/`). CUDA builds also download NVIDIA's CUDA runtime archive
published alongside them, under NVIDIA's redistribution terms.

## Atomic Chat and Jan (ideas, no code copied)

The built-in engine's defaults and its memory-fit approach were informed by
[Atomic Chat](https://github.com/AtomicBot-ai/Atomic-Chat) (Apache-2.0; Copyright 2026 Atomic Chat,
Copyright 2025 Menlo Research), which began as a fork of [Jan](https://github.com/janhq/jan)
(Apache-2.0, Menlo Research). Neru adopted these ideas from them: `--fit on`; `--flash-attn auto` (off on
Vulkan); two slots sharing one cache (`-kvu`); a smaller `--cache-ram` on machines with 16 GB or less;
integrated graphics stay on the processor; and growing the context on overflow (8K, 32K, then by half).
The code in `llama_server.rs`, `llama_models.rs`, `gguf.rs` and `hardware.rs` is Neru's own.

## Models

Models downloaded in Settings come from Hugging Face under their own licenses (Qwen: Apache-2.0;
Gemma: Gemma Terms of Use; Llama 3.2: Llama 3.2 Community License; gpt-oss: Apache-2.0;
Devstral: Apache-2.0). The GGUF conversions in the catalog are published by Unsloth.
