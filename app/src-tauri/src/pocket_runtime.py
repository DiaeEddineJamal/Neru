"""Neru's local prompt lab. Input and output are NDJSON; model data never leaves the PC."""
import json
import sys


def emit(**record):
    print(json.dumps(record, ensure_ascii=False), flush=True)


def run(request):
    config = request["config"]
    import litert_lm as lm
    lm.set_min_log_severity(lm.LogSeverity.ERROR)
    backend = lm.Backend.GPU() if config["accelerator"] == "gpu" else lm.Backend.CPU()
    messages = request["messages"]
    initial = [lm.Message.model(m["text"]) if m["role"] == "assistant" else lm.Message.user(m["text"]) for m in messages[:-1][-12:]]
    # ponytail: one process per turn isolates native memory; reuse an engine if reload time becomes a bottleneck.
    with lm.Engine(request["modelPath"], backend=backend, max_num_tokens=config["contextTokens"], cache_dir=request["cachePath"], enable_speculative_decoding=config["speculative"]) as engine:
        with engine.create_conversation(messages=initial, system_message=config["systemPrompt"], max_output_tokens=config["maxTokens"], sampler_config=lm.SamplerConfig(top_k=config["topK"], top_p=config["topP"], temperature=config["temperature"]), thinking_config=lm.ThinkingConfig(enable_thinking=config["thinking"]), automatic_tool_calling=False) as conversation:
            for chunk in conversation.send_message_async(messages[-1]["text"]):
                emit(text="".join(c.get("text", "") for c in chunk.get("content", []) if c.get("type") == "text"), reasoning=chunk.get("channels", {}).get("thought", ""))
    emit(done=True)


if __name__ == "__main__":
    try:
        run(json.loads(sys.stdin.readline()))
    except Exception as error:
        emit(error=str(error))
        sys.exit(1)
