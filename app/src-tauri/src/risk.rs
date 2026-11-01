//! Risk-scored auto-approval: in Auto mode, a shell command that the rules gate (policy.rs) neither
//! allows nor denies goes to a cheap model with the rules, the command and the folder. A `low`
//! verdict runs it; `medium` and `high` ask, with the model's reason on the approval card.
//!
//! Hard refusals and destructive-always-ask rules are decided before this and never reach the
//! model. Verdicts are cached per session and command, so a repeated command costs nothing.

use std::{
    collections::HashMap,
    path::Path,
    sync::{Mutex, OnceLock},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::ProviderConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Risk {
    Low,
    Medium,
    High,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verdict {
    pub risk: Risk,
    pub reason: String,
}

impl Verdict {
    fn unsure(reason: &str) -> Self {
        Verdict { risk: Risk::Medium, reason: reason.into() }
    }
}

fn cache() -> &'static Mutex<HashMap<(String, String), Verdict>> {
    static CACHE: OnceLock<Mutex<HashMap<(String, String), Verdict>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// The verdict already given for this command in this session.
pub fn cached(session: &str, command: &str) -> Option<Verdict> {
    cache().lock().ok()?.get(&(session.to_string(), command.trim().to_string())).cloned()
}

/// The latest verdict for a command in any session, for redrawing an approval card.
pub fn lookup(command: &str) -> Option<Verdict> {
    let command = command.trim();
    cache().lock().ok()?.iter().find(|((_, cached), _)| cached == command).map(|(_, verdict)| verdict.clone())
}

pub fn remember(session: &str, command: &str, verdict: &Verdict) {
    if let Ok(mut cache) = cache().lock() {
        cache.insert((session.to_string(), command.trim().to_string()), verdict.clone());
    }
}

const RULES: &str = "You judge whether a coding agent may run a shell command without asking the user. \
Rate the risk:\n\
- low: reads or builds inside the project, runs its tests, linters, formatters or package scripts, \
inspects files or Git history, or writes only inside the project or the temp folder. Easy to undo.\n\
- medium: installs or removes packages, changes Git history or remotes, deletes files inside the project, \
starts network requests that send project data, or changes settings outside the project in a reversible way.\n\
- high: deletes or overwrites data outside the project, reads or sends secrets or credentials, \
publishes or deploys, pushes to a remote, changes system or security settings, or runs code downloaded from the internet.\n\
When unsure, choose the higher risk. Reply with only a JSON object: {\"risk\":\"low|medium|high\",\"reason\":\"one short sentence\"}.";

/// The request sent to the scoring model.
pub fn prompt(root: &Path, command: &str) -> Vec<Value> {
    let mut rules = RULES.to_string();
    for name in ["allow.txt", "deny.txt"] {
        if let Ok(text) = std::fs::read_to_string(root.join(".neru").join(name)) {
            let lines: Vec<&str> = text.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')).take(40).collect();
            if !lines.is_empty() {
                rules.push_str(&format!("\n\nThe project's .neru/{name} lists:\n{}", lines.join("\n")));
            }
        }
    }
    vec![
        json!({"role": "system", "content": rules}),
        json!({"role": "user", "content": format!("Working directory: {}\nCommand:\n{}", root.display(), command)}),
    ]
}

/// The verdict in a model reply: a JSON object, possibly inside a code fence or prose.
pub fn parse(reply: &str) -> Option<Verdict> {
    let start = reply.find('{')?;
    let end = reply.rfind('}')?;
    let value: Value = serde_json::from_str(reply.get(start..=end)?).ok()?;
    let risk = match value["risk"].as_str()?.trim().to_lowercase().as_str() {
        "low" => Risk::Low,
        "medium" | "moderate" => Risk::Medium,
        "high" => Risk::High,
        _ => return None,
    };
    let reason: String = value["reason"].as_str().unwrap_or("").trim().chars().take(240).collect();
    Some(Verdict { risk, reason: if reason.is_empty() { "No reason given.".into() } else { reason } })
}

/// The model that scores: a fast helper on the same key when there is one, else the session's own.
async fn scorer(main: &ProviderConfig) -> ProviderConfig {
    let mut config = main.clone();
    if let Some(model) = crate::fallback::helper_model(main).await {
        config.model = model;
    }
    config
}

/// Scores a command, from the session's cache when it was scored before. Any failure is `medium`,
/// so the user is asked.
pub async fn score(main: &ProviderConfig, session: &str, root: &Path, command: &str) -> Verdict {
    if let Some(verdict) = cached(session, command) {
        return verdict;
    }
    let rule = crate::privacy::rule_for(Some(root));
    let config = scorer(main).await;
    if !crate::privacy::allows(&rule, &config) {
        return Verdict::unsure("The privacy rule keeps this project off the scoring model.");
    }
    let client = crate::providers::http();
    let cancel = tokio::sync::Notify::new();
    let mut visible = String::new();
    let round = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        crate::agent::model_round_silent(&client, &config, &cancel, &prompt(root, command), &json!([]), &mut visible),
    )
    .await;
    let verdict = match round {
        Ok(Ok(crate::agent::Round::Message(message))) => parse(message["content"].as_str().filter(|text| !text.is_empty()).unwrap_or(&visible)),
        _ => None,
    };
    let Some(verdict) = verdict else {
        return Verdict::unsure("The risk check did not answer, so Neru asks.");
    };
    remember(session, command, &verdict);
    verdict
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_verdicts_in_fences_and_prose() {
        let fenced = "```json\n{\"risk\": \"low\", \"reason\": \"Lists files.\"}\n```";
        assert_eq!(parse(fenced), Some(Verdict { risk: Risk::Low, reason: "Lists files.".into() }));
        assert_eq!(parse("Sure: {\"risk\":\"HIGH\",\"reason\":\"pushes\"}").map(|verdict| verdict.risk), Some(Risk::High));
        assert_eq!(parse("{\"risk\":\"moderate\"}").map(|verdict| verdict.risk), Some(Risk::Medium));
        assert_eq!(parse("{\"risk\":\"maybe\"}"), None);
        assert_eq!(parse("no json here"), None);
    }

    #[test]
    fn caches_per_session_and_command() {
        let verdict = Verdict { risk: Risk::Low, reason: "Reads a file.".into() };
        remember("risk-test-a", " cat README.md ", &verdict);
        assert_eq!(cached("risk-test-a", "cat README.md"), Some(verdict.clone()));
        assert_eq!(cached("risk-test-b", "cat README.md"), None);
        assert_eq!(lookup("cat README.md"), Some(verdict));
    }

    #[test]
    fn prompt_carries_rules_command_and_folder() {
        let root = std::env::temp_dir().join(format!("neru-risk-{}", std::process::id()));
        std::fs::create_dir_all(root.join(".neru")).unwrap();
        std::fs::write(root.join(".neru").join("deny.txt"), "# comment\nterraform apply\n").unwrap();
        let messages = prompt(&root, "ls -la");
        let system = messages[0]["content"].as_str().unwrap();
        assert!(system.contains("terraform apply") && !system.contains("# comment"));
        let user = messages[1]["content"].as_str().unwrap();
        assert!(user.contains("ls -la") && user.contains(&root.display().to_string()));
        let _ = std::fs::remove_dir_all(root);
    }
}
