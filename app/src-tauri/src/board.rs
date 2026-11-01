//! The session board: every session of the open project as a card in Running, Needs you, Done or
//! Merged, with its branch, uncommitted files, commits ahead of its base and pull request, and
//! one-click commit, pull request and merge for the session's own checkout.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::{
    AppState, git,
    sessions::{self, SessionPr, SessionSummary},
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BoardCard {
    pub session: SessionSummary,
    pub branch: String,
    pub base: String,
    /// Uncommitted files in the session's checkout.
    pub dirty: usize,
    /// Commits on the session's branch that its base does not have (worktree sessions).
    pub ahead: usize,
}

/// The column a session belongs in.
pub(crate) fn column(running: bool, waiting: bool, pr_state: Option<&str>, landed: bool) -> &'static str {
    if waiting {
        "needs-you"
    } else if running {
        "running"
    } else if landed || pr_state.is_some_and(|state| state.eq_ignore_ascii_case("merged")) {
        "merged"
    } else {
        "done"
    }
}

/// "passing", "failing" or "pending" for a pull request's checks; empty without any.
pub(crate) fn checks_summary(states: &[&str]) -> String {
    if states.is_empty() {
        String::new()
    } else if states.contains(&"failure") {
        "failing".into()
    } else if states.contains(&"pending") {
        "pending".into()
    } else {
        "passing".into()
    }
}

fn dirty_files(root: &Path) -> usize {
    git::git(root, &["status", "--porcelain"]).map(|text| text.lines().filter(|line| !line.trim().is_empty()).count()).unwrap_or(0)
}

/// A worktree branch whose work is in its base: it moved since the worktree was made, and its
/// tip is an ancestor of the base.
fn landed(project: &Path, branch: &str, base: &str) -> bool {
    let Ok(tip) = git::git(project, &["rev-parse", "--verify", "--quiet", branch]) else { return false };
    let created = git::git(project, &["reflog", "show", "--format=%H", branch]).ok().and_then(|text| text.lines().last().map(str::to_string));
    created.is_some_and(|created| created.trim() != tip.trim()) && git::git(project, &["merge-base", "--is-ancestor", branch, base]).is_ok()
}

fn pr_for(root: &Path) -> Option<SessionPr> {
    let pr = git::pr_status_at(root).ok()??;
    let states: Vec<&str> = pr.checks.iter().map(|check| check.state.as_str()).collect();
    Some(SessionPr { number: pr.number, url: pr.url.clone(), state: pr.state.clone(), checks: checks_summary(&states) })
}

fn card(project: &Path, mut summary: SessionSummary, running: bool, waiting: bool, with_pr: bool) -> BoardCard {
    let (root, branch, base) = match &summary.worktree {
        Some(tree) if Path::new(&tree.path).is_dir() => (PathBuf::from(&tree.path), tree.branch.clone(), tree.base.clone()),
        _ => {
            let branch = git::git(project, &["symbolic-ref", "--quiet", "--short", "HEAD"]).map(|name| name.trim().to_string()).unwrap_or_default();
            (project.to_path_buf(), branch, String::new())
        }
    };
    let dirty = dirty_files(&root);
    let ahead = if base.is_empty() {
        0
    } else {
        git::git(project, &["rev-list", "--count", &format!("{base}..{branch}")]).ok().and_then(|count| count.trim().parse().ok()).unwrap_or(0)
    };
    // Only worktree sessions own their branch; a session in the main checkout shares it.
    let pr = if with_pr && !base.is_empty() && crate::git::gh_available() { pr_for(&root) } else { None };
    let merged = !base.is_empty() && ahead == 0 && dirty == 0 && landed(project, &branch, &base);
    summary.status = Some(column(running, waiting, pr.as_ref().map(|pr| pr.state.as_str()), merged).into());
    summary.running = running;
    summary.pr = pr;
    BoardCard { session: summary, branch, base, dirty, ahead }
}

/// Every session of the open project with its column and Git state. `with_pr` also asks the
/// GitHub CLI for each worktree branch's pull request, which takes a moment per session.
#[tauri::command]
pub async fn session_board(with_pr: Option<bool>, app: AppHandle) -> Result<Vec<BoardCard>, String> {
    let state = app.state::<AppState>();
    let project = sessions::main_root(&state)?;
    let listed = sessions::all_for(&project)?;
    let ids: std::collections::HashSet<String> = listed.iter().map(|summary| summary.id.clone()).collect();
    // Loaded sessions are read from memory, since they can be newer than their files.
    let entries: Vec<(SessionSummary, bool)> = sessions::all_with_status(&state)?.into_iter().filter(|(summary, _)| ids.contains(&summary.id)).collect();
    let with_pr = with_pr.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || {
        entries
            .into_iter()
            .map(|(summary, waiting)| {
                let running = summary.running;
                card(&project, summary, running, waiting, with_pr)
            })
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}

/// `commit` (everything in the session's checkout), `pr` (push and open a pull request) or
/// `merge` (its pull request, or a worktree branch into its base in the main checkout).
#[tauri::command]
pub async fn board_action(session_id: String, action: String, message: Option<String>, app: AppHandle) -> Result<String, String> {
    let state = app.state::<AppState>();
    let project = sessions::main_root(&state)?;
    let shared = sessions::runtime(&state, &session_id)?;
    let (summary, root) = {
        let runtime = sessions::lock(&shared)?;
        if runtime.running {
            return Err("Wait for this session to finish first".into());
        }
        (runtime.summary.clone(), runtime.work_root())
    };
    tauri::async_runtime::spawn_blocking(move || run_action(&project, &root, &summary, &action, message.as_deref())).await.map_err(|e| e.to_string())?
}

fn run_action(project: &Path, root: &Path, summary: &SessionSummary, action: &str, message: Option<&str>) -> Result<String, String> {
    match action {
        "commit" => {
            if dirty_files(root) == 0 {
                return Err("Nothing to commit".into());
            }
            let message = message.map(str::trim).filter(|message| !message.is_empty()).unwrap_or(&summary.title);
            git::git(root, &["add", "-A"])?;
            git::git(root, &["commit", "-m", message]).map(|out| out.trim().to_string())
        }
        "pr" => {
            let tree = summary.worktree.as_ref().ok_or("Pull requests from the board are for worktree sessions; use Git for the main checkout")?;
            if !git::gh_available() {
                return Err("Install the GitHub CLI (gh) and run `gh auth login` to open pull requests from the board".into());
            }
            git::git(root, &["push", "--set-upstream", "origin", &tree.branch])?;
            let title = message.map(str::trim).filter(|message| !message.is_empty()).unwrap_or(&summary.title);
            let out = git::gh(root, &["pr", "create", "--head", &tree.branch, "--base", &tree.base, "--title", title, "--body", "Opened from Neru's session board."])?;
            Ok(out.lines().rev().find(|line| line.starts_with("https://")).unwrap_or(out.trim()).to_string())
        }
        "merge" => {
            let tree = summary.worktree.as_ref().ok_or("Merge from the board is for worktree sessions")?;
            if git::gh_available() {
                if let Some(pr) = pr_for(root).filter(|pr| pr.state.eq_ignore_ascii_case("open")) {
                    return git::gh(root, &["pr", "merge", &pr.number.to_string(), "--squash"]).map(|_| format!("Merged pull request #{}", pr.number));
                }
            }
            if dirty_files(root) > 0 {
                return Err("Commit the session's changes before merging".into());
            }
            let current = git::git(project, &["symbolic-ref", "--quiet", "--short", "HEAD"]).unwrap_or_default();
            if current.trim() != tree.base {
                return Err(format!("Switch the main checkout to {} to merge this branch into it", tree.base));
            }
            git::git(project, &["merge", "--no-edit", "--", &tree.branch]).map(|out| out.trim().to_string())
        }
        other => Err(format!("Unknown board action {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_land_in_the_right_column() {
        assert_eq!(column(true, true, None, false), "needs-you");
        assert_eq!(column(true, false, Some("OPEN"), false), "running");
        assert_eq!(column(false, false, Some("MERGED"), false), "merged");
        assert_eq!(column(false, false, None, true), "merged");
        assert_eq!(column(false, false, Some("OPEN"), false), "done");
    }

    #[test]
    fn checks_sum_up() {
        assert_eq!(checks_summary(&[]), "");
        assert_eq!(checks_summary(&["success", "pending"]), "pending");
        assert_eq!(checks_summary(&["success", "failure", "pending"]), "failing");
        assert_eq!(checks_summary(&["success", "skipped"]), "passing");
    }

    #[test]
    fn unknown_actions_are_refused() {
        let summary: SessionSummary = serde_json::from_value(serde_json::json!({"id":"a","title":"T","projectPath":"","updatedAt":0})).unwrap();
        let here = Path::new(".");
        assert!(run_action(here, here, &summary, "push", None).unwrap_err().contains("Unknown"));
        assert!(run_action(here, here, &summary, "pr", None).unwrap_err().contains("worktree"));
        assert!(run_action(here, here, &summary, "merge", None).unwrap_err().contains("worktree"));
    }

    #[test]
    fn summaries_keep_status_off_disk() {
        let summary: SessionSummary = serde_json::from_value(serde_json::json!({"id":"a","title":"T","projectPath":"","updatedAt":0})).unwrap();
        let text = serde_json::to_string(&summary).unwrap();
        assert!(!text.contains("status") && !text.contains("\"pr\""));
    }
}
