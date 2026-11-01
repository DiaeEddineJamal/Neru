//! A second opinion on each proposed edit, from a model of another family than the one that wrote it (a
//! Gemini or DeepSeek model checking what Qwen wrote). It runs while the change waits for approval, and its
//! verdict shows on the approval card, here and on the phone. On free keys it costs nothing.

use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};

use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Manager};

use crate::{AppState, ProviderConfig, agent, workspace::EditProposal};

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Review {
    /// running, agree, concerns, skipped or failed.
    pub status: String,
    pub reviewer: String,
    pub summary: String,
    pub notes: Vec<String>,
}

/// Reviews by the diff they are about, so a snapshot of the waiting edit can carry its review.
static REVIEWS: LazyLock<Mutex<HashMap<String, Review>>> = LazyLock::new(Default::default);

fn key(diff: &str) -> String {
    let digest = <sha2::Sha256 as sha2::Digest>::digest(diff.as_bytes());
    digest.iter().take(12).map(|byte| format!("{byte:02x}")).collect()
}

pub fn lookup(diff: &str) -> Option<Review> {
    REVIEWS.lock().ok()?.get(&key(diff)).cloned()
}

fn store(diff: &str, review: Review) {
    if let Ok(mut map) = REVIEWS.lock() {
        if map.len() > 200 {
            map.clear();
        }
        map.insert(key(diff), review);
    }
}

/// The model family by name: the same family reviewing its own work would share its blind spots.
pub fn family(model: &str) -> String {
    let name = model.rsplit('/').next().unwrap_or(model).to_lowercase();
    const FAMILIES: &[(&str, &[&str])] = &[
        ("qwen", &["qwen", "qwq"]),
        ("google", &["gemini", "gemma"]),
        ("deepseek", &["deepseek"]),
        ("meta", &["llama"]),
        ("moonshot", &["kimi", "moonshot"]),
        ("zhipu", &["glm", "chatglm"]),
        ("mistral", &["mistral", "codestral", "devstral", "magistral", "ministral", "pixtral"]),
        ("minimax", &["minimax"]),
        ("anthropic", &["claude"]),
        ("openai", &["gpt", "o1", "o3", "o4", "codex"]),
        ("xai", &["grok"]),
        ("nvidia", &["nemotron"]),
        ("microsoft", &["phi-"]),
    ];
    FAMILIES
        .iter()
        .find(|(_, parts)| parts.iter().any(|part| name.starts_with(part) || name.contains(&format!("-{part}")) || name.contains(part) && part.len() > 3))
        .map(|(family, _)| family.to_string())
        .unwrap_or_else(|| name.split(['-', ':', '.']).next().unwrap_or(&name).to_string())
}

/// Builds the reviewer's request: the user's request, the change, and how to answer.
fn request(task: &str, proposal: &EditProposal) -> Vec<Value> {
    let diff: String = proposal.diff.chars().take(24_000).collect();
    let task: String = task.chars().take(4_000).collect();
    vec![
        json!({"role":"system","content":"You are a senior engineer giving a second opinion on a change another AI model proposed. Judge only what the diff shows: bugs, broken logic, security problems, missing pieces the request needs, and changes the user did not ask for. Do not nitpick style. Answer with JSON only, no fences: {\"verdict\":\"agree\" or \"concerns\",\"summary\":\"one short sentence\",\"notes\":[\"each concrete problem, with the line it is on\"]}. Use agree with an empty notes list when the change is sound."}),
        json!({"role":"user","content":format!("The user asked:\n{task}\n\nThe proposed change to {}:\n```diff\n{diff}\n```", proposal.path)}),
    ]
}

/// Reads the reviewer's JSON, tolerating code fences, thinking tags or prose around it.
pub fn parse(reply: &str, reviewer: &str) -> Review {
    let text = reply.split("</think>").last().unwrap_or(reply);
    let json = text.find('{').and_then(|start| text.rfind('}').filter(|end| *end > start).map(|end| &text[start..=end])).and_then(|body| serde_json::from_str::<Value>(body).ok());
    match json {
        Some(value) => {
            let notes: Vec<String> = value["notes"].as_array().into_iter().flatten().filter_map(Value::as_str).map(|note| note.trim().chars().take(400).collect()).filter(|note: &String| !note.is_empty()).take(6).collect();
            let agree = value["verdict"].as_str().is_some_and(|verdict| verdict.eq_ignore_ascii_case("agree")) && notes.is_empty();
            Review { status: if agree { "agree" } else { "concerns" }.into(), reviewer: reviewer.into(), summary: value["summary"].as_str().unwrap_or("").trim().chars().take(300).collect(), notes }
        }
        None => Review { status: "failed".into(), reviewer: reviewer.into(), summary: "The reviewer did not answer in a form Neru could read.".into(), notes: vec![] },
    }
}

/// Starts a review of `proposal` (written by `author`) unless second opinions are off. Emits a Review event
/// to the session when it lands, so the window and phones update the waiting card.
pub fn start(app: &AppHandle, session: &str, author: &ProviderConfig, task: String, proposal: &EditProposal, rule: String) {
    if !crate::privacy::prefs().second_opinion || proposal.diff.trim().is_empty() {
        return;
    }
    let (app, session, author, proposal) = (app.clone(), session.to_string(), author.clone(), proposal.clone());
    store(&proposal.diff, Review { status: "running".into(), reviewer: "A second model".into(), summary: String::new(), notes: vec![] });
    tauri::async_runtime::spawn(async move {
        let state = app.state::<AppState>();
        let review = match crate::fallback::reviewer_model(&state, &author, &family(&author.model), &rule).await {
            None => Review { status: "skipped".into(), reviewer: String::new(), summary: "Add a key from another provider (Gemini, NVIDIA, Mistral…) so a model of a different family can check changes.".into(), notes: vec![] },
            Some(reviewer) => {
                let label = reviewer.model.rsplit('/').next().unwrap_or(&reviewer.model).to_string();
                store(&proposal.diff, Review { status: "running".into(), reviewer: label.clone(), summary: String::new(), notes: vec![] });
                emit(&app, &session, &proposal.diff);
                let client = reqwest::Client::new();
                let cancel = Arc::new(tokio::sync::Notify::new());
                let mut visible = String::new();
                let round = tokio::time::timeout(Duration::from_secs(150), agent::model_round_silent(&client, &reviewer, &cancel, &request(&task, &proposal), &json!([]), &mut visible)).await;
                match round {
                    Ok(Ok(agent::Round::Message(message))) => parse(message["content"].as_str().filter(|text| !text.is_empty()).unwrap_or(&visible), &label),
                    Ok(Err(error)) => Review { status: "failed".into(), reviewer: label, summary: error.chars().take(200).collect(), notes: vec![] },
                    _ => Review { status: "failed".into(), reviewer: label, summary: "The reviewer took too long.".into(), notes: vec![] },
                }
            }
        };
        store(&proposal.diff, review);
        emit(&app, &session, &proposal.diff);
    });
}

fn emit(app: &AppHandle, session: &str, diff: &str) {
    if let Some(review) = lookup(diff) {
        agent::emit(app, session, agent::AgentEvent::Review { review });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn families_group_models_by_maker() {
        assert_eq!(family("qwen/qwen3-coder-480b-a35b-instruct"), "qwen");
        assert_eq!(family("gemini-3.8-flash"), "google");
        assert_eq!(family("google/gemma-3-27b-it"), "google");
        assert_eq!(family("deepseek-ai/deepseek-v3.1"), "deepseek");
        assert_eq!(family("meta/llama-4-maverick"), "meta");
        assert_eq!(family("codestral-latest"), "mistral");
        assert_eq!(family("openai/gpt-oss-120b"), "openai");
        assert_eq!(family("moonshotai/kimi-k2-instruct"), "moonshot");
        assert_ne!(family("qwen3-coder"), family("glm-4.6"));
    }

    #[test]
    fn verdicts_are_read_from_messy_replies() {
        let agree = parse("<think>hmm</think>```json\n{\"verdict\":\"agree\",\"summary\":\"Looks right.\",\"notes\":[]}\n```", "gemini");
        assert_eq!((agree.status.as_str(), agree.summary.as_str()), ("agree", "Looks right."));
        let concerns = parse("{\"verdict\":\"concerns\",\"summary\":\"Off by one.\",\"notes\":[\"Line 4 skips the last item\"]}", "glm");
        assert_eq!(concerns.status, "concerns");
        assert_eq!(concerns.notes, vec!["Line 4 skips the last item"]);
        // "agree" with notes is still a concern worth reading.
        assert_eq!(parse("{\"verdict\":\"agree\",\"notes\":[\"x is unused\"]}", "m").status, "concerns");
        assert_eq!(parse("I think it's fine", "m").status, "failed");
    }
}
