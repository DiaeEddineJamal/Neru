//! Options one run of the terminal CLI sets for every request: `--allowedTools`, `--disallowedTools`,
//! `--max-turns`, `--fallback-model`, `--system-prompt` and `--add-dir`. The rules use the same
//! syntax as `permissions` in Claude Code's settings files.

use std::sync::{LazyLock, RwLock};

#[derive(Clone, Default, Debug, PartialEq)]
pub struct RunOptions {
    /// Rules that skip the approval prompt, like `Bash(npm run test:*)` or `Edit`.
    pub allow: Vec<String>,
    /// Rules that refuse a call; a bare tool name removes the tool altogether.
    pub deny: Vec<String>,
    /// Most tool rounds per request.
    pub max_rounds: Option<usize>,
    /// Models to switch to, in order, before Neru picks one itself when the current model fails.
    pub fallback_models: Vec<String>,
    /// Replaces Neru's own system prompt; instruction files, skills and memory still follow it.
    pub system_prompt: Option<String>,
    /// Folders outside the project the agent may also read and edit, canonical.
    pub add_dirs: Vec<std::path::PathBuf>,
    /// --append-system-prompt: added to the end of the system prompt.
    pub append_system_prompt: Option<String>,
    /// The CLI's --model and automatic switches last for this run only, as in Claude Code: saving
    /// settings keeps the model already on disk. /model and `neru login` turn it off.
    pub keep_saved_model: bool,
}

/// The `--add-dir` folder that holds `path`, when `path` is absolute and inside one.
pub fn added_dir_for(path: &std::path::Path) -> Option<std::path::PathBuf> {
    if !path.is_absolute() {
        return None;
    }
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    get().add_dirs.into_iter().find(|dir| path.starts_with(dir))
}

static OPTIONS: LazyLock<RwLock<RunOptions>> = LazyLock::new(Default::default);

pub fn set(options: RunOptions) {
    if let Ok(mut current) = OPTIONS.write() {
        *current = options;
    }
}

pub fn update(change: impl FnOnce(&mut RunOptions)) {
    if let Ok(mut current) = OPTIONS.write() {
        change(&mut current);
    }
}

pub fn get() -> RunOptions {
    OPTIONS.read().map(|options| options.clone()).unwrap_or_default()
}

/// The rules as a settings value, for `policy::Rules`.
pub fn as_settings(options: &RunOptions) -> serde_json::Value {
    serde_json::json!({"permissions": {"allow": options.allow, "deny": options.deny}})
}

/// Splits `--allowedTools "Bash(git log:*) Edit,Read"` into rules, keeping parentheses together.
pub fn split_rules(text: &str) -> Vec<String> {
    let mut rules = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    for c in text.chars() {
        match c {
            '(' => {
                depth += 1;
                current.push(c);
            }
            ')' => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            ',' | ' ' if depth == 0 => {
                if !current.trim().is_empty() {
                    rules.push(current.trim().to_string());
                }
                current.clear();
            }
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        rules.push(current.trim().to_string());
    }
    rules
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_split_on_commas_and_spaces_outside_parentheses() {
        assert_eq!(split_rules("Bash(git log:*) Edit,Read"), vec!["Bash(git log:*)", "Edit", "Read"]);
        assert_eq!(split_rules(" mcp__docs , Bash(npm run test:*) "), vec!["mcp__docs", "Bash(npm run test:*)"]);
        assert!(split_rules("").is_empty());
    }
}
