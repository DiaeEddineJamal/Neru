//! How Neru stretches free quotas: the architect/editor split (a strong model plans each change, a
//! faster model on its own quota writes it) and chores on the small-model lane, such as commit messages.

use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::{AppHandle, Manager};

use crate::{AppState, ProviderConfig};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct EngineSettings {
    /// "auto" (on for free plans), "on" or "off".
    pub architect_editor: String,
}

impl Default for EngineSettings {
    fn default() -> Self {
        Self { architect_editor: "auto".into() }
    }
}

static SETTINGS: LazyLock<Mutex<Option<EngineSettings>>> = LazyLock::new(|| Mutex::new(None));

fn path() -> Option<std::path::PathBuf> {
    // Tests must not touch the user's data folder.
    if cfg!(test) {
        return None;
    }
    crate::workspace::data_dir().ok().map(|dir| dir.join("engine.json"))
}

pub fn get() -> EngineSettings {
    let mut guard = SETTINGS.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get_or_insert_with(|| path().and_then(|path| std::fs::read(path).ok()).and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default())
        .clone()
}

#[tauri::command]
pub fn engine_settings() -> EngineSettings {
    get()
}

#[tauri::command]
pub fn set_engine_settings(settings: EngineSettings) -> Result<EngineSettings, String> {
    if !matches!(settings.architect_editor.as_str(), "auto" | "on" | "off") {
        return Err("architectEditor must be auto, on or off".into());
    }
    if let Some(path) = path() {
        std::fs::write(path, serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    }
    *SETTINGS.lock().map_err(|e| e.to_string())? = Some(settings.clone());
    Ok(settings)
}

/// Whether file changes go through the editor model for this run.
pub fn architect_editor(config: &ProviderConfig) -> bool {
    match get().architect_editor.as_str() {
        "on" => true,
        "off" => false,
        _ => crate::fallback::is_free_plan(config),
    }
}

/// Removes the code fences and quotes models like to wrap a commit message in.
fn clean_message(text: &str) -> String {
    let text = text.trim();
    let text = text.strip_prefix("```").map(|rest| rest.split_once('\n').map_or("", |(_, body)| body)).unwrap_or(text);
    let text = text.strip_suffix("```").unwrap_or(text).trim();
    text.trim_matches(|c| c == '"' || c == '\'' || c == '`').trim().to_string()
}

/// A commit message for the staged changes (or, with nothing staged, the unstaged ones), written on
/// the small-model lane and in the style of the repository's recent subjects.
#[tauri::command]
pub async fn generate_commit_message(app: AppHandle) -> Result<String, String> {
    let state = app.state::<AppState>();
    let root = crate::workspace::project_root(&state)?;
    let staged = crate::git::git(&root, &["diff", "--cached", "--no-color", "-U2"]).unwrap_or_default();
    let diff = if staged.trim().is_empty() { crate::git::git(&root, &["diff", "--no-color", "-U2"]).unwrap_or_default() } else { staged };
    if diff.trim().is_empty() {
        return Err("There are no changes to describe".into());
    }
    let recent = crate::git::git(&root, &["log", "-8", "--pretty=%s"]).unwrap_or_default();
    let diff = crate::agent::clip_output(diff, 14_000);
    let messages = vec![
        json!({"role":"system","content":"You write git commit messages. Reply with only the message: a subject line in the imperative mood of at most 72 characters with no trailing period, then, only if the change needs explaining, a blank line and a short body wrapped at 72 characters. Follow the style of the recent subjects (prefixes, capitalization). No quotes, no code fences, no commentary."}),
        json!({"role":"user","content":format!("Recent subjects:\n{}\n\nDiff:\n{diff}", if recent.trim().is_empty() { "(none yet)" } else { recent.trim() })}),
    ];
    let config = state.provider.lock().map_err(|e| e.to_string())?.clone();
    let rule = crate::privacy::rule_for(Some(&root));
    let cancel = tokio::sync::Notify::new();
    let text = crate::agent::small_round(&state, &config, &rule, &cancel, &messages, 4_000).await?;
    let message = clean_message(&text);
    if message.is_empty() {
        return Err("The model returned an empty message".into());
    }
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_split_is_on_for_free_plans_by_default() {
        let free = ProviderConfig { provider_id: "gemini".into(), api_format: "openai-chat".into(), base_url: String::new(), api_key: String::new(), model: "gemini-3.8-flash".into() };
        let paid = ProviderConfig { provider_id: "anthropic".into(), model: "claude-sonnet-5".into(), ..free.clone() };
        assert_eq!(get().architect_editor, "auto");
        assert!(architect_editor(&free));
        assert!(!architect_editor(&paid));
        assert!(set_engine_settings(EngineSettings { architect_editor: "sometimes".into() }).is_err());
    }

    #[test]
    fn commit_messages_lose_their_wrapping() {
        assert_eq!(clean_message("```\nFix login redirect\n```"), "Fix login redirect");
        assert_eq!(clean_message("\"Add dark mode\""), "Add dark mode");
        assert_eq!(clean_message("Add dark mode\n\nUses the system setting."), "Add dark mode\n\nUses the system setting.");
    }
}
