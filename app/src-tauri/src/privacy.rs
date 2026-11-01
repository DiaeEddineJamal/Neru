//! What leaves the machine. Providers carry a privacy level from their published terms (the same table as
//! src/providerCatalog.ts); a rule, global or per project in `.neru/privacy.json`, keeps requests to the
//! providers it allows. Secrets in what goes to a cloud model (keys, tokens, passwords, private keys) are
//! swapped for stable placeholders like `«secret:OpenAI key:1a2b3c»` and swapped back in the reply, so the
//! model never sees them while its edits and commands still use the real values. The phone uses the same
//! patterns in mobile/src/shared/redact.ts.

use std::{
    collections::HashMap,
    fs,
    path::Path,
    sync::{LazyLock, Mutex},
};

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ProviderConfig, workspace::data_dir};

/// A provider's privacy level: `local`, `no-training`, `may-train` or `unknown`.
pub fn privacy_of(config: &ProviderConfig) -> &'static str {
    if is_local(config) {
        return "local";
    }
    match config.provider_id.as_str() {
        "gemini" | "mistral" | "openrouter" | "deepseek" => "may-train",
        "groq" | "openai" | "anthropic" | "cloudflare" | "ovhcloud" => "no-training",
        _ => "unknown",
    }
}

/// Ollama, a local gateway, or any endpoint on this machine.
pub fn is_local(config: &ProviderConfig) -> bool {
    matches!(config.provider_id.as_str(), "ollama" | "local") || reqwest::Url::parse(&config.base_url).is_ok_and(|url| crate::providers::is_local(&url))
}

pub fn label(privacy: &str) -> &'static str {
    match privacy {
        "local" => "stays on your devices",
        "no-training" => "is not used for training",
        "may-train" => "may train on what you send",
        _ => "does not clearly say what it keeps",
    }
}

/// 'any' allows every provider, 'no-train' blocks `may-train`, 'local' allows only this machine.
pub fn allows(rule: &str, config: &ProviderConfig) -> bool {
    match rule {
        "local" => privacy_of(config) == "local",
        "no-train" => privacy_of(config) != "may-train",
        _ => true,
    }
}

// ---------- settings ----------

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    /// The rule for projects without their own `.neru/privacy.json`, and for chats.
    pub privacy: String,
    /// Swap secrets for placeholders before a request leaves for a cloud model.
    pub redact: bool,
    /// A model of another family reviews each proposed edit before you approve it.
    pub second_opinion: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Self { privacy: "any".into(), redact: true, second_opinion: true }
    }
}

static PREFS: LazyLock<Mutex<Option<Prefs>>> = LazyLock::new(|| Mutex::new(None));

pub fn prefs() -> Prefs {
    let mut cached = PREFS.lock().unwrap_or_else(|e| e.into_inner());
    cached
        .get_or_insert_with(|| data_dir().ok().and_then(|dir| fs::read_to_string(dir.join("privacy.json")).ok()).and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default())
        .clone()
}

#[tauri::command]
pub fn get_privacy_prefs() -> Prefs {
    prefs()
}

#[tauri::command]
pub fn set_privacy_prefs(prefs: Prefs) -> Result<Prefs, String> {
    if !matches!(prefs.privacy.as_str(), "any" | "no-train" | "local") {
        return Err("Unknown privacy rule".into());
    }
    fs::write(data_dir()?.join("privacy.json"), serde_json::to_string_pretty(&prefs).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    *PREFS.lock().map_err(|e| e.to_string())? = Some(prefs.clone());
    Ok(prefs)
}

/// The project's own rule from `.neru/privacy.json` ({"rule": "no-train"}), which a team can commit.
pub fn project_rule(root: &Path) -> Option<String> {
    let text = fs::read_to_string(root.join(".neru").join("privacy.json")).ok()?;
    let rule = serde_json::from_str::<Value>(&text).ok()?["rule"].as_str()?.to_string();
    matches!(rule.as_str(), "any" | "no-train" | "local").then_some(rule)
}

/// The rule in force for a project (empty root: a chat).
pub fn rule_for(root: Option<&Path>) -> String {
    root.and_then(project_rule).unwrap_or_else(|| prefs().privacy)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectPrivacy {
    /// The project's own rule, or None when it follows the global one.
    pub rule: Option<String>,
    pub global: String,
}

#[tauri::command]
pub fn get_project_privacy(state: tauri::State<'_, crate::AppState>) -> Result<ProjectPrivacy, String> {
    let root = crate::workspace::project_root(&state)?;
    Ok(ProjectPrivacy { rule: project_rule(&root), global: prefs().privacy })
}

/// Sets the open project's rule (None removes `.neru/privacy.json`, so the global rule applies).
#[tauri::command]
pub fn set_project_privacy(rule: Option<String>, state: tauri::State<'_, crate::AppState>) -> Result<ProjectPrivacy, String> {
    let root = crate::workspace::project_root(&state)?;
    let path = root.join(".neru").join("privacy.json");
    match rule.as_deref() {
        None => {
            let _ = fs::remove_file(&path);
        }
        Some(rule @ ("any" | "no-train" | "local")) => {
            fs::create_dir_all(root.join(".neru")).map_err(|e| e.to_string())?;
            fs::write(&path, format!("{{\n  \"rule\": \"{rule}\"\n}}\n")).map_err(|e| e.to_string())?;
        }
        Some(_) => return Err("Unknown privacy rule".into()),
    }
    get_project_privacy(state)
}

pub fn provider_name(provider_id: &str) -> &str {
    match provider_id {
        "nvidia" => "NVIDIA NIM",
        "modelscope" => "ModelScope",
        "gemini" => "Google Gemini",
        "cerebras" => "Cerebras",
        "mistral" => "Mistral",
        "openrouter" => "OpenRouter",
        "groq" => "Groq",
        "huggingface" => "Hugging Face",
        "cloudflare" => "Cloudflare Workers AI",
        "sambanova" => "SambaNova",
        "ovhcloud" => "OVHcloud AI Endpoints",
        "llm7" => "LLM7",
        "kilo" => "Kilo Code",
        "ollamacloud" => "Ollama Cloud",
        "zai" => "Z.AI",
        "ollama" => "Ollama",
        "local" => "Your local gateway",
        "openai" => "OpenAI",
        "anthropic" => "Anthropic",
        "deepseek" => "DeepSeek",
        "xai" => "xAI",
        other => other,
    }
}

/// Why `rule` keeps `config` out, or None when it may be used.
pub fn blocked_reason(rule: &str, config: &ProviderConfig) -> Option<String> {
    if allows(rule, config) {
        return None;
    }
    let what = if rule == "local" { "only models on this machine" } else { "only providers that do not train on your data" };
    Some(format!(
        "Privacy rule: {} {}, and this project allows {what}. Pick another model, or change the rule in Settings → Privacy.",
        provider_name(&config.provider_id),
        label(privacy_of(config))
    ))
}

// ---------- secrets ----------

static RULES: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    [
        ("private key", r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----"),
        ("Anthropic key", r"\bsk-ant-[A-Za-z0-9_-]{20,}"),
        ("OpenAI key", r"\bsk-(?:proj-|svcacct-)?[A-Za-z0-9_-]{20,}"),
        ("GitHub token", r"\b(?:gh[pousr]_[A-Za-z0-9]{30,}|github_pat_[A-Za-z0-9_]{30,})"),
        ("AWS key", r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b"),
        ("Google key", r"\bAIza[0-9A-Za-z_-]{35}"),
        ("Slack token", r"\bxox[abprs]-[A-Za-z0-9-]{10,}"),
        ("Stripe key", r"\b(?:sk|rk)_live_[0-9A-Za-z]{20,}"),
        ("NVIDIA key", r"\bnvapi-[A-Za-z0-9_-]{30,}"),
        ("Hugging Face token", r"\bhf_[A-Za-z0-9]{30,}"),
        ("Groq key", r"\bgsk_[A-Za-z0-9]{40,}"),
        ("OpenRouter key", r"\bsk-or-v1-[a-f0-9]{40,}"),
        ("JSON web token", r"\beyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}"),
    ]
    .into_iter()
    .map(|(kind, pattern)| (kind, Regex::new(pattern).expect("secret pattern")))
    .collect()
});
static ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\b([A-Za-z0-9_.-]*(?:secret|token|passw(?:or)?d|pwd|api[_-]?key|apikey|private[_-]?key|access[_-]?key|client[_-]?secret|auth)[A-Za-z0-9_.-]*)(["']?\s*[:=]\s*["']?)([^\s"'`,;#]{8,})"#).expect("assignment pattern")
});
static URL_PASSWORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b[a-z][a-z0-9+.-]*://[^:\s/@]+:([^@\s/]{3,})@").expect("url pattern"));
static PLACEHOLDER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"«secret:[^:»]{1,60}:[0-9a-f]{8}»").expect("placeholder pattern"));
static EXAMPLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:your|my|example|sample|dummy|test|xxx|changeme|placeholder|redacted|\*+|<|\$\{|\{\{|process\.env|os\.environ|env\(|true|false|null|none|undefined)").expect("example pattern")
});

/// Values that are clearly examples or code, not secrets. Real secrets have a digit or are long.
fn example(value: &str) -> bool {
    (!value.chars().any(|c| c.is_ascii_digit()) && value.len() < 16)
        || value.contains(['(', ')', '[', ']', '{', '}'])
        || EXAMPLE.is_match(value)
        || value.chars().all(|c| matches!(c, 'x' | 'X' | '*' | '.'))
        || value.contains("«secret")
}

/// Every secret in `text` as (kind, byte range), in order, without overlaps.
pub fn find_secrets(text: &str) -> Vec<(String, std::ops::Range<usize>)> {
    let mut found: Vec<(String, std::ops::Range<usize>)> = Vec::new();
    for (kind, rule) in RULES.iter() {
        for m in rule.find_iter(text) {
            found.push((kind.to_string(), m.range()));
        }
    }
    for caps in ASSIGNMENT.captures_iter(text) {
        let value = caps.get(3).expect("value group");
        if !example(value.as_str()) {
            found.push((caps[1].to_string(), value.range()));
        }
    }
    for caps in URL_PASSWORD.captures_iter(text) {
        let value = caps.get(1).expect("password group");
        if !example(value.as_str()) {
            found.push(("password".into(), value.range()));
        }
    }
    found.sort_by(|a, b| a.1.start.cmp(&b.1.start).then(b.1.end.cmp(&a.1.end)));
    let mut out: Vec<(String, std::ops::Range<usize>)> = Vec::new();
    for item in found {
        if out.last().is_none_or(|last| item.1.start >= last.1.end) {
            out.push(item);
        }
    }
    out
}

/// Placeholders and the secrets they stand for, for one request and its reply.
#[derive(Default)]
pub struct Vault {
    secrets: HashMap<String, String>,
}

impl Vault {
    pub fn hidden(&self) -> usize {
        self.secrets.len()
    }

    fn hide(&mut self, text: &str) -> Option<String> {
        let found = find_secrets(text);
        if found.is_empty() {
            return None;
        }
        let mut out = String::with_capacity(text.len());
        let mut at = 0;
        for (kind, range) in found {
            let value = &text[range.clone()];
            // Stable per value, so the model sees the same placeholder in every turn.
            let digest = <sha2::Sha256 as sha2::Digest>::digest(value.as_bytes());
            let hash: String = digest.iter().take(4).map(|byte| format!("{byte:02x}")).collect();
            let kind: String = kind.chars().filter(|c| *c != ':' && *c != '»').take(60).collect();
            let mark = format!("«secret:{kind}:{hash}»");
            self.secrets.insert(mark.clone(), value.to_string());
            out.push_str(&text[at..range.start]);
            out.push_str(&mark);
            at = range.end;
        }
        out.push_str(&text[at..]);
        Some(out)
    }

    /// Puts the secrets back in every string of `value` (reply text, tool call arguments).
    pub fn restore(&self, value: &mut Value) {
        if self.secrets.is_empty() {
            return;
        }
        match value {
            Value::String(text) => {
                if text.contains("«secret:") {
                    *text = self.restore_text(text);
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|item| self.restore(item)),
            Value::Object(map) => map.values_mut().for_each(|item| self.restore(item)),
            _ => {}
        }
    }

    pub fn restore_text(&self, text: &str) -> String {
        if self.secrets.is_empty() || !text.contains("«secret:") {
            return text.to_string();
        }
        PLACEHOLDER.replace_all(text, |caps: &regex::Captures| self.secrets.get(&caps[0]).cloned().unwrap_or_else(|| caps[0].to_string())).into_owned()
    }
}

fn hide_in(value: &mut Value, vault: &mut Vault) {
    match value {
        Value::String(text) => {
            if let Some(hidden) = vault.hide(text) {
                *text = hidden;
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| hide_in(item, vault)),
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                // Images and ids are not text a secret hides in.
                if !matches!(key.as_str(), "image_url" | "url" | "data" | "id" | "tool_call_id" | "role" | "type") {
                    hide_in(item, vault);
                }
            }
        }
        _ => {}
    }
}

static ANNOUNCED: LazyLock<Mutex<HashMap<String, usize>>> = LazyLock::new(Default::default);

/// Whether to tell `session` about `hidden` secrets: only when more are hidden than it was last told.
pub fn announce(session: &str, hidden: usize) -> bool {
    let mut told = ANNOUNCED.lock().unwrap_or_else(|e| e.into_inner());
    let last = told.entry(session.to_string()).or_insert(0);
    let fresh = hidden > *last;
    *last = (*last).max(hidden);
    fresh
}

/// The messages to send to `config`, with secrets hidden when it is a cloud model and redaction is on.
pub fn outgoing(config: &ProviderConfig, messages: &[Value]) -> (Vec<Value>, Vault) {
    let mut vault = Vault::default();
    if is_local(config) || !prefs().redact {
        return (messages.to_vec(), vault);
    }
    let mut out = messages.to_vec();
    out.iter_mut().for_each(|message| hide_in(message, &mut vault));
    (out, vault)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cloud() -> ProviderConfig {
        ProviderConfig { provider_id: "gemini".into(), api_format: "openai-chat".into(), base_url: "https://generativelanguage.googleapis.com/v1beta/openai".into(), model: "gemini-3.8-flash".into(), api_key: String::new() }
    }

    #[test]
    fn secrets_are_found_and_examples_are_not() {
        let kinds = |text: &str| find_secrets(text).into_iter().map(|(kind, _)| kind).collect::<Vec<_>>();
        assert_eq!(kinds("OPENAI_API_KEY=sk-proj-abcdefghijklmnopqrstuvwx1234"), vec!["OpenAI key"]);
        assert_eq!(kinds("DB_PASSWORD=\"hunter2hunter2\""), vec!["DB_PASSWORD"]);
        assert_eq!(kinds("postgres://neru:s3cretpass@db:5432/app"), vec!["password"]);
        assert_eq!(kinds("-----BEGIN RSA PRIVATE KEY-----\nMIIabc\n-----END RSA PRIVATE KEY-----"), vec!["private key"]);
        for fine in ["API_KEY=your-key-here", "password = process.env.DB_PASSWORD", "const token = getToken()", "SECRET=${SECRET}", "Set the TOKEN env var first."] {
            assert!(find_secrets(fine).is_empty(), "{fine}");
        }
    }

    #[test]
    fn secrets_leave_as_placeholders_and_come_back_in_the_reply() {
        let messages = vec![json!({"role":"tool","tool_call_id":"call_1","content":"GITHUB_TOKEN=ghp_abcdefghijklmnopqrstuvwxyz0123456789\nPORT=3000"})];
        let (sent, vault) = outgoing(&cloud(), &messages);
        let text = sent[0]["content"].as_str().unwrap();
        assert!(!text.contains("ghp_"), "{text}");
        assert!(text.contains("«secret:GitHub token:") && text.contains("PORT=3000"));
        assert_eq!(vault.hidden(), 1);
        let mark = PLACEHOLDER.find(text).unwrap().as_str().to_string();
        // The model writes the placeholder into a file edit; the real value goes to disk.
        let mut reply = json!({"role":"assistant","tool_calls":[{"id":"c","function":{"name":"propose_edit","arguments":format!("{{\"new\":\"token: {mark}\"}}")}}]});
        vault.restore(&mut reply);
        assert!(reply["tool_calls"][0]["function"]["arguments"].as_str().unwrap().contains("ghp_abcdefghijklmnopqrstuvwxyz0123456789"));
        // The same value gets the same placeholder next turn.
        let (again, _) = outgoing(&cloud(), &messages);
        assert_eq!(again, sent);
    }

    #[test]
    fn local_models_see_everything_and_rules_block_by_terms() {
        let local = ProviderConfig { provider_id: "ollama".into(), base_url: "http://localhost:11434/v1".into(), ..cloud() };
        let messages = vec![json!({"role":"user","content":"key sk-ant-abcdefghijklmnopqrstuvwxyz"})];
        assert_eq!(outgoing(&local, &messages).0, messages);
        assert!(allows("local", &local) && !allows("local", &cloud()));
        assert!(!allows("no-train", &cloud()));
        assert!(allows("no-train", &ProviderConfig { provider_id: "groq".into(), ..cloud() }));
    }
}
