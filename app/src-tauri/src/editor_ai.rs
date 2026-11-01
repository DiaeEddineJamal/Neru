//! AI in the code editor: Ctrl+K rewrites a selection from an instruction, and Tab completion
//! fills in code at the cursor. Completion uses Mistral's Codestral fill-in-the-middle endpoint
//! when a Mistral key is saved, since it is fast and free, and the selected chat model otherwise.
//! Both follow the project's privacy rule and hide secrets like a normal request.

use std::time::Duration;

use serde_json::{Value, json};
use tauri::{AppHandle, Manager};

use crate::{
    AppState, ProviderConfig,
    agent::{Round, model_round_silent},
    providers, settings,
    workspace::project_root,
};

const MISTRAL_BASE: &str = "https://api.mistral.ai/v1";
const FIM_MODEL: &str = "codestral-latest";
/// How much code around the cursor or selection goes with a request.
const CONTEXT_CHARS: usize = 6_000;
const COMPLETION_CHARS: usize = 3_000;

fn tail(text: &str, max: usize) -> &str {
    let start = text.len().saturating_sub(max);
    let start = (start..=text.len()).find(|index| text.is_char_boundary(*index)).unwrap_or(text.len());
    &text[start..]
}

fn head(text: &str, max: usize) -> &str {
    let end = (0..=max.min(text.len())).rev().find(|index| text.is_char_boundary(*index)).unwrap_or(0);
    &text[..end]
}

/// The model's reply as plain code: one surrounding Markdown fence is removed, and so is a
/// leading "Here is…" line some models add despite being told not to.
pub(crate) fn clean_replacement(reply: &str) -> String {
    let trimmed = reply.trim_matches(|c| c == '\n' || c == '\r');
    let mut lines: Vec<&str> = trimmed.lines().collect();
    if lines.first().is_some_and(|line| {
        let lower = line.trim().to_lowercase();
        (lower.starts_with("here is") || lower.starts_with("here's")) && lower.ends_with(':')
    }) {
        lines.remove(0);
    }
    let fenced = lines.first().is_some_and(|line| line.trim_start().starts_with("```"));
    if fenced {
        lines.remove(0);
        if let Some(end) = lines.iter().rposition(|line| line.trim() == "```") {
            lines.truncate(end);
        }
    }
    lines.join("\n")
}

/// Keeps a completion from repeating what is already on either side of the cursor and from
/// running on past a few lines.
pub(crate) fn trim_completion(text: &str, prefix: &str, suffix: &str) -> String {
    let mut text = clean_replacement(text);
    // Chat models often echo the current line before continuing it.
    let line_start = prefix.rsplit('\n').next().unwrap_or("").trim_start();
    if !line_start.trim().is_empty() && text.starts_with(line_start) {
        text = text[line_start.len()..].to_string();
    }
    // Stop where the text runs into what follows the cursor.
    let next = suffix.lines().map(str::trim).find(|line| !line.is_empty()).unwrap_or("");
    if next.len() >= 3 {
        if let Some(at) = text.find(next) {
            text.truncate(at);
        }
    }
    let lines: Vec<&str> = text.lines().take(12).collect();
    let mut out = lines.join("\n");
    if text.ends_with('\n') && lines.len() < 12 {
        out.push('\n');
    }
    out.trim_end_matches([' ', '\t']).to_string()
}

/// The completion in a Codestral FIM reply (`choices[0].message.content`, or `text` on older
/// versions of the API).
pub(crate) fn parse_fim(body: &Value) -> Option<String> {
    let choice = &body["choices"][0];
    choice["message"]["content"].as_str().or_else(|| choice["text"].as_str()).map(str::to_string)
}

fn language(path: &str) -> &str {
    path.rsplit('.').next().filter(|ext| !ext.contains(['/', '\\'])).unwrap_or("")
}

fn current_config(app: &AppHandle) -> Result<(ProviderConfig, String), String> {
    let state = app.state::<AppState>();
    let config = state.provider.lock().map_err(|e| e.to_string())?.clone();
    let rule = crate::privacy::rule_for(project_root(&state).ok().as_deref());
    Ok((config, rule))
}

/// One reply from the selected model, with nothing streamed to the window.
async fn ask(config: &ProviderConfig, rule: &str, system: &str, user: String, limit: Duration) -> Result<String, String> {
    if !crate::agent::provider_view(config).configured || config.model.is_empty() {
        return Err("Configure a provider in Settings".into());
    }
    if let Some(reason) = crate::privacy::blocked_reason(rule, config) {
        return Err(reason);
    }
    let messages = vec![json!({"role":"system","content":system}), json!({"role":"user","content":user})];
    let client = providers::http();
    let cancel = tokio::sync::Notify::new();
    let mut visible = String::new();
    let round = tokio::time::timeout(limit, model_round_silent(&client, config, &cancel, &messages, &json!([]), &mut visible))
        .await
        .map_err(|_| "The model took too long to answer".to_string())??;
    match round {
        Round::Message(message) => Ok(message["content"].as_str().map(str::to_string).unwrap_or(visible)),
        Round::Cancelled => Err("Cancelled".into()),
    }
}

/// Rewrites `selection` (from the file at `path`) as `instruction` asks and returns the new text.
#[tauri::command]
pub async fn inline_edit(path: String, selection: String, instruction: String, before: Option<String>, after: Option<String>, app: AppHandle) -> Result<String, String> {
    let instruction = instruction.trim();
    if instruction.is_empty() {
        return Err("Say what to change".into());
    }
    let (config, rule) = current_config(&app)?;
    let before = before.unwrap_or_default();
    let after = after.unwrap_or_default();
    let system = "You edit code inside an editor. You get a file path, the code before and after a selection, the selected code and an instruction. Reply with only the code that replaces the selection: no explanation, no Markdown fences, the same indentation as the original. When the selection is empty, reply with the code to insert at the cursor.";
    let user = format!(
        "File: {path}\n\nCode before the selection:\n{}\n\n<selection>\n{selection}\n</selection>\n\nCode after the selection:\n{}\n\nInstruction: {instruction}",
        tail(&before, CONTEXT_CHARS),
        head(&after, CONTEXT_CHARS / 2),
    );
    let reply = ask(&config, &rule, system, user, Duration::from_secs(90)).await?;
    let replacement = clean_replacement(&reply);
    if replacement.trim().is_empty() && !selection.trim().is_empty() && !instruction.to_lowercase().contains("delete") && !instruction.to_lowercase().contains("remove") {
        return Err("The model returned no code".into());
    }
    Ok(replacement)
}

/// Code to insert at the cursor, or an empty string when there is nothing worth suggesting.
#[tauri::command]
pub async fn inline_complete(prefix: String, suffix: String, path: String, app: AppHandle) -> Result<String, String> {
    if prefix.trim().is_empty() {
        return Ok(String::new());
    }
    let (config, rule) = current_config(&app)?;
    let prefix = tail(&prefix, COMPLETION_CHARS).to_string();
    let suffix = head(&suffix, COMPLETION_CHARS / 2).to_string();
    let mistral = {
        let state = app.state::<AppState>();
        let keys = state.provider_keys.lock().map_err(|e| e.to_string())?;
        settings::key_for(&keys, "mistral", MISTRAL_BASE)
    };
    if let Some(key) = mistral {
        let fim = ProviderConfig { provider_id: "mistral".into(), api_format: "openai-chat".into(), base_url: MISTRAL_BASE.into(), model: FIM_MODEL.into(), api_key: key };
        if crate::privacy::allows(&rule, &fim) {
            if let Ok(text) = fim_complete(&fim, &prefix, &suffix).await {
                return Ok(trim_completion(&text, &prefix, &suffix));
            }
        }
    }
    let system = "You are a code completion engine. Continue the code at <cursor>. Reply with only the text to insert there: no explanation, no Markdown fences, and do not repeat code that is already before or after the cursor. Keep it short (at most a few lines). Reply with nothing if no completion makes sense.";
    let user = format!("File: {path} ({})\n\n{prefix}<cursor>{suffix}", language(&path));
    let reply = ask(&config, &rule, system, user, Duration::from_secs(15)).await?;
    Ok(trim_completion(&reply, &prefix, &suffix))
}

async fn fim_complete(config: &ProviderConfig, prefix: &str, suffix: &str) -> Result<String, String> {
    // The same secret handling as chat requests: placeholders go out, real values come back.
    let (outgoing, vault) = crate::privacy::outgoing(config, &[json!({"role":"user","content":prefix}), json!({"role":"user","content":suffix})]);
    let body = json!({
        "model": config.model,
        "prompt": outgoing[0]["content"],
        "suffix": outgoing[1]["content"],
        "max_tokens": 96,
        "temperature": 0.1,
        "stop": ["\n\n\n"],
    });
    let request = providers::authorize(providers::http().post(format!("{MISTRAL_BASE}/fim/completions")), config).json(&body);
    let response = tokio::time::timeout(Duration::from_secs(10), request.send())
        .await
        .map_err(|_| "Codestral timed out".to_string())?
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("Codestral answered {}", response.status()));
    }
    let value: Value = response.json().await.map_err(|e| e.to_string())?;
    parse_fim(&value).map(|text| vault.restore_text(&text)).ok_or_else(|| "Codestral sent no completion".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_lose_fences_and_preambles() {
        assert_eq!(clean_replacement("```ts\nconst a = 1\n```\n"), "const a = 1");
        assert_eq!(clean_replacement("Here is the updated code:\n```\nfoo()\n```"), "foo()");
        assert_eq!(clean_replacement("  indented()\n"), "  indented()");
    }

    #[test]
    fn completions_do_not_repeat_the_surroundings() {
        // The model echoed the current line.
        assert_eq!(trim_completion("const total = items.length", "  const total = ", "\n}"), "items.length");
        // It ran into the code after the cursor.
        assert_eq!(trim_completion("items.length\nconsole.log(total)", "const total = ", "\nconsole.log(total)"), "items.length\n");
        assert_eq!(trim_completion("```\nx\n```", "let y = ", ""), "x");
    }

    #[test]
    fn reads_fim_replies() {
        assert_eq!(parse_fim(&json!({"choices":[{"message":{"content":"x + 1"}}]})).as_deref(), Some("x + 1"));
        assert_eq!(parse_fim(&json!({"choices":[{"text":"y"}]})).as_deref(), Some("y"));
        assert_eq!(parse_fim(&json!({"error":"bad"})), None);
    }

    #[test]
    fn clips_on_char_boundaries() {
        assert_eq!(tail("héllo", 4), "llo");
        assert_eq!(head("héllo", 2), "h");
    }
}
