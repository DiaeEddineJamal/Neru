//! `neru login` and `/login`: connect a model provider from the terminal, the way the app's
//! onboarding does. Pick a provider, paste its key (hidden), pick a model from the key's own list.

use crate::{
    agent,
    cli_ui::{self, BOLD, CYAN, DIM, GREEN, RESET, SAGE, Screen, YELLOW},
};

pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    pub format: &'static str,
    pub model: &'static str,
    pub key_url: &'static str,
    pub free: &'static str,
}

const fn preset(id: &'static str, name: &'static str, base_url: &'static str, format: &'static str, model: &'static str, key_url: &'static str, free: &'static str) -> Preset {
    Preset { id, name, base_url, format, model, key_url, free }
}

/// The app's provider list (app/src/providerCatalog.ts), free coding providers first.
pub const PRESETS: &[Preset] = &[
    preset("nvidia", "NVIDIA NIM", "https://integrate.api.nvidia.com/v1", "openai-chat", "", "https://build.nvidia.com/settings/api-keys", "free · ~10,000 requests/day"),
    preset("modelscope", "ModelScope", "https://api-inference.modelscope.cn/v1", "openai-chat", "", "https://modelscope.cn/my/myaccesstoken", "free · 2,000 requests/day"),
    preset("gemini", "Google Gemini", "https://generativelanguage.googleapis.com/v1beta/openai", "openai-chat", "gemini-3.8-flash", "https://aistudio.google.com/apikey", "free · daily quota per model"),
    preset("cerebras", "Cerebras", "https://api.cerebras.ai/v1", "openai-chat", "gpt-oss-120b", "https://cloud.cerebras.ai/platform/", "free · 1M tokens/day"),
    preset("mistral", "Mistral", "https://api.mistral.ai/v1", "openai-chat", "codestral-latest", "https://console.mistral.ai/api-keys", "free · monthly credit"),
    preset("openrouter", "OpenRouter", "https://openrouter.ai/api/v1", "openai-chat", "openrouter/free", "https://openrouter.ai/settings/keys", "free · 50 requests/day"),
    preset("groq", "Groq Cloud", "https://api.groq.com/openai/v1", "openai-chat", "llama-3.3-70b-versatile", "https://console.groq.com/keys", "free · small per-minute cap"),
    preset("huggingface", "Hugging Face", "https://router.huggingface.co/v1", "openai-chat", "openai/gpt-oss-120b", "https://huggingface.co/settings/tokens", "free · small monthly credit"),
    preset("ollama", "Ollama (local models)", "http://localhost:11434/v1", "openai-chat", "", "", "runs on this computer"),
    preset("local", "Local gateway", "http://localhost:3001/v1", "openai-chat", "auto", "", "any OpenAI-compatible server"),
    preset("opencode", "OpenCode Zen", "https://opencode.ai/zen/v1", "openai-chat", "", "https://opencode.ai/auth", ""),
    preset("xai", "Grok (xAI)", "https://api.x.ai/v1", "openai-chat", "", "https://console.x.ai", ""),
    preset("openai", "OpenAI", "https://api.openai.com/v1", "openai-responses", "", "https://platform.openai.com/api-keys", ""),
    preset("anthropic", "Anthropic", "https://api.anthropic.com/v1", "anthropic", "", "https://console.anthropic.com/settings/keys", ""),
    preset("deepseek", "DeepSeek", "https://api.deepseek.com", "openai-chat", "", "https://platform.deepseek.com/api_keys", ""),
    preset("qwen", "Alibaba Qwen", "https://dashscope-intl.aliyuncs.com/compatible-mode/v1", "openai-chat", "", "https://modelstudio.console.alibabacloud.com/?tab=playground#/api-key", ""),
    preset("kimi", "Moonshot Kimi", "https://api.moonshot.ai/v1", "openai-chat", "", "https://platform.moonshot.ai/console/api-keys", ""),
    preset("zai", "Z.AI GLM", "https://api.z.ai/api/paas/v4", "openai-chat", "", "https://z.ai/manage-apikey/apikey-list", ""),
    preset("minimax", "MiniMax", "https://api.minimax.io/v1", "openai-chat", "", "https://platform.minimax.io/user-center/basic-information/interface-key", ""),
    preset("custom", "Custom endpoint", "", "openai-chat", "", "", "any Chat, Responses or Messages API"),
];

fn leaf(model: &str) -> String {
    model.rsplit('/').next().unwrap_or(model).trim().to_lowercase()
}

fn starts(name: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| name.starts_with(prefix))
}

/// Which protocol a model speaks, as `formatForModel` decides in the app.
pub fn format_for(provider_id: &str, model: &str, base_url: &str, current: &str) -> String {
    let name = leaf(model);
    let host = reqwest::Url::parse(base_url).ok().and_then(|url| url.host_str().map(str::to_lowercase)).unwrap_or_default();
    let openai = |name: &str| if starts(name, &["gpt-3.5", "gpt-4-", "text-", "davinci", "babbage", "whisper", "tts", "dall-e"]) || name == "gpt-4" { "openai-chat" } else { "openai-responses" };
    let opencode = |name: &str| {
        if starts(name, &["gpt-", "grok-", "muse-spark"]) {
            "openai-responses"
        } else if starts(name, &["claude-", "qwen3.5-", "qwen3.6-", "qwen3.7-", "qwen3.8-flash"]) {
            "anthropic"
        } else {
            "openai-chat"
        }
    };
    match provider_id {
        "anthropic" => "anthropic",
        "openai" => openai(&name),
        "opencode" => opencode(&name),
        "custom" if host == "api.openai.com" => openai(&name),
        "custom" if host == "api.anthropic.com" || name.starts_with("claude-") => "anthropic",
        "custom" if host.ends_with("opencode.ai") => opencode(&name),
        "custom" if starts(&name, &["gpt-4o", "gpt-4.1", "gpt-5", "o1", "o3", "o4"]) => "openai-responses",
        "custom" if name.is_empty() => current,
        _ => PRESETS.iter().find(|item| item.id == provider_id).map_or("openai-chat", |item| item.format),
    }
    .to_string()
}

fn note(screen: &mut Screen, text: &str) {
    screen.print(&format!("  {DIM}⎿{RESET}  {text}"));
}

/// Asks for one line of plain text (a URL or a model id). None when cancelled.
fn ask(screen: &mut Screen, label: &str, hint: &str) -> Option<String> {
    let mut editor = cli_ui::Editor::new(Vec::new());
    let prompt = cli_ui::Prompt { hint, left: format!("{BOLD}{label}{RESET} {DIM}· esc twice to cancel{RESET}"), right: String::new() };
    match editor.read(screen, &prompt, &|_| Vec::new()) {
        cli_ui::Input::Line(line) if !line.trim().is_empty() => Some(line.trim().to_string()),
        _ => None,
    }
}

/// The whole flow. Returns the connected "model · provider", or None when cancelled.
pub fn login(app: &tauri::AppHandle, screen: &mut Screen) -> Result<Option<String>, String> {
    use tauri::Manager;
    let current = agent::provider_status(app.state()).ok();
    let items: Vec<(String, String)> = PRESETS.iter().map(|item| (item.name.to_string(), item.free.to_string())).collect();
    let selected = current.as_ref().and_then(|view| PRESETS.iter().position(|item| item.id == view.provider_id));
    screen.print(&format!("{SAGE}✻{RESET} {BOLD}Connect a model{RESET}\n  {DIM}Free providers come first. Your key is stored encrypted on this computer and sent only to that provider.{RESET}"));
    let Some(index) = cli_ui::pick(screen, "Provider", &items, selected) else { return Ok(None) };
    let preset = &PRESETS[index];
    let base_url = if preset.base_url.is_empty() {
        match ask(screen, "Base URL", "https://api.example.com/v1") {
            Some(url) => url,
            None => return Ok(None),
        }
    } else {
        preset.base_url.to_string()
    };
    let local = reqwest::Url::parse(&base_url).is_ok_and(|url| crate::providers::is_local(&url));
    let same = current.as_ref().is_some_and(|view| view.provider_id == preset.id && view.base_url == base_url && view.has_key);
    let mut key = String::new();
    if !local {
        if !preset.key_url.is_empty() {
            screen.print(&format!("  Get a key at {CYAN}\x1b]8;;{0}\x1b\\{0}\x1b]8;;\x1b\\{RESET}", preset.key_url));
            if !same {
                let _ = crate::web::open_url(preset.key_url.to_string());
            }
        }
        let label = if same { format!("{} API key (enter to keep the saved one)", preset.name) } else { format!("{} API key", preset.name) };
        match cli_ui::read_secret(screen, &label) {
            Some(value) => key = value,
            None if same => {}
            None => return Ok(None),
        }
    }
    screen.footer(&[format!("  {DIM}Loading {}'s models…{RESET}", preset.name)], None);
    let models = tauri::async_runtime::block_on(crate::models::list_models(preset.id.into(), preset.format.into(), base_url.clone(), key.clone(), app.clone()));
    screen.clear();
    let model = match models {
        Ok(list) if !list.is_empty() => {
            let usable: Vec<_> = list.into_iter().filter(|info| info.verified != "unavailable").collect();
            let rows: Vec<(String, String)> = usable
                .iter()
                .map(|info| {
                    let mut detail = Vec::new();
                    if info.free {
                        detail.push("free".to_string());
                    }
                    if info.kind == "code" {
                        detail.push("coding".into());
                    }
                    if let Some(window) = info.context_window {
                        detail.push(format!("{}K context", window / 1000));
                    }
                    (info.id.clone(), detail.join(" · "))
                })
                .collect();
            let suggested = usable.iter().position(|info| !preset.model.is_empty() && info.id == preset.model).or(Some(0));
            match cli_ui::pick(screen, &format!("Model · {} ({} available)", preset.name, rows.len()), &rows, suggested) {
                Some(index) => usable[index].id.clone(),
                None => return Ok(None),
            }
        }
        Ok(_) if !preset.model.is_empty() => preset.model.to_string(),
        result => {
            if let Err(error) = &result {
                note(screen, &crate::cli::friendly(error));
            }
            match ask(screen, "Model id", "the model's id, e.g. qwen/qwen3-coder") {
                Some(model) => model,
                None => return Ok(None),
            }
        }
    };
    let format = format_for(preset.id, &model, &base_url, preset.format);
    // Signing in picks the default model, even inside a run started with --model.
    crate::run_options::update(|run| run.keep_saved_model = false);
    let view = agent::configure_provider(preset.id.into(), format, base_url, key, model, app.state())?;
    screen.print(&format!("{GREEN}⏺{RESET} Connected to {BOLD}{}{RESET} with {BOLD}{}{RESET}", preset.name, view.model));
    // A listed model can still be refused (a free tier locked to the provider's own app, no
    // credits), so find out now rather than on the first request.
    screen.footer(&[format!("  {DIM}Checking that {} answers…{RESET}", view.model)], None);
    let config = app.state::<crate::AppState>().provider.lock().map_err(|e| e.to_string())?.clone();
    let health = tauri::async_runtime::block_on(crate::models::health(&config, false));
    screen.clear();
    match health.status.as_str() {
        "unavailable" | "badKey" => screen.print(&format!("  {YELLOW}⚠{RESET} {} does not work with this key: {}.\n    {DIM}Run{RESET} /login {DIM}again to pick another model.{RESET}", view.model, health.reason)),
        "unknown" => note(screen, &format!("Could not confirm {} right now ({}). It may still work.", view.model, health.reason)),
        _ => {}
    }
    Ok(Some(format!("{} · {}", view.model, view.provider_id)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_follow_the_app() {
        assert_eq!(format_for("openai", "gpt-5.1", "", "openai-chat"), "openai-responses");
        assert_eq!(format_for("openai", "gpt-3.5-turbo", "", "openai-chat"), "openai-chat");
        assert_eq!(format_for("opencode", "claude-sonnet-5", "", "openai-chat"), "anthropic");
        assert_eq!(format_for("nvidia", "qwen/qwen3-coder", "", "openai-chat"), "openai-chat");
        assert_eq!(format_for("custom", "claude-x", "https://gw.example.com/v1", "openai-chat"), "anthropic");
    }
}
