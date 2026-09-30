//! Project trust, like Claude Code's workspace trust. A project's `.mcp.json` servers, the hooks
//! in its `.claude/settings.json`, `.claude/settings.local.json` and `.neru/settings.json`, and the
//! allow-rules in those files only take effect once the user has trusted the folder. Trusted
//! folders are kept in `<data_dir>/trusted-projects.json` as canonical path strings; trusting a
//! folder trusts everything inside it, and a Neru worktree counts as its main project.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::Serialize;
use serde_json::Value;

/// Serializes read-modify-write of the trust file.
static LOCK: Mutex<()> = Mutex::new(());

/// What an untrusted project would run once trusted.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TrustStatus {
    pub trusted: bool,
    /// Enabled servers from the project's `.mcp.json`.
    pub mcp_servers: Vec<String>,
    /// Claude-style events (`PreToolUse`, …) with hooks in the project's settings files.
    pub hook_events: Vec<String>,
}

fn store_path() -> Result<PathBuf, String> {
    #[cfg(test)]
    {
        let dir = std::env::temp_dir().join(format!("neru-trust-tests-{}", std::process::id()));
        fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        Ok(dir.join("trusted-projects.json"))
    }
    #[cfg(not(test))]
    {
        Ok(crate::workspace::data_dir()?.join("trusted-projects.json"))
    }
}

/// The canonical form of `path` as stored: resolved, without Windows' `\\?\` prefix or a
/// trailing separator.
fn canonical(path: &Path) -> String {
    let resolved = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = resolved.display().to_string();
    let text = text.strip_prefix(r"\\?\UNC\").map(|rest| format!(r"\\{rest}")).unwrap_or_else(|| text.trim_start_matches(r"\\?\").to_string());
    text.trim_end_matches(['/', '\\']).to_string()
}

fn same(a: &str, b: &str) -> bool {
    if cfg!(windows) { a.eq_ignore_ascii_case(b) } else { a == b }
}

fn read_store(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')).ok())
        .and_then(|value| value.as_array().cloned())
        .map(|items| items.iter().filter_map(|item| item.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

/// The main project of a Neru worktree (its `.git` file points into `<main>/.git/worktrees/…`).
fn main_project(root: &Path) -> Option<PathBuf> {
    let text = fs::read_to_string(root.join(".git")).ok()?;
    let gitdir = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
    let worktrees = gitdir.parent()?;
    if worktrees.file_name()? != "worktrees" {
        return None;
    }
    let dot_git = worktrees.parent()?;
    (dot_git.file_name()? == ".git").then(|| dot_git.parent().map(Path::to_path_buf)).flatten()
}

fn trusted_in(store: &[String], root: &Path) -> bool {
    let candidates = std::iter::once(root.to_path_buf()).chain(main_project(root));
    candidates.map(|path| canonical(&path)).any(|path| {
        let path = Path::new(&path);
        path.ancestors().any(|folder| {
            let folder = folder.display().to_string();
            let folder = folder.trim_end_matches(['/', '\\']);
            !folder.is_empty() && store.iter().any(|entry| same(entry.trim_end_matches(['/', '\\']), folder))
        })
    })
}

/// True when the user has trusted `root` (or a folder that contains it).
pub fn is_trusted(root: &Path) -> bool {
    store_path().is_ok_and(|path| trusted_in(&read_store(&path), root))
}

/// Remembers `root` as trusted.
pub fn trust_project(root: &Path) -> Result<(), String> {
    let _guard = LOCK.lock().map_err(|e| e.to_string())?;
    let path = store_path()?;
    let mut store = read_store(&path);
    let entry = canonical(root);
    if !store.iter().any(|item| same(item, &entry)) {
        store.push(entry);
        store.sort();
    }
    let text = serde_json::to_string_pretty(&store).map_err(|e| e.to_string())?;
    fs::write(&path, text).map_err(|e| e.to_string())
}

/// Whether `root` is trusted, and what trusting it would turn on.
pub fn project_trust(root: &Path) -> TrustStatus {
    let mcp_servers = crate::mcp::read_project_config(root).into_iter().filter(|server| server.config.enabled).map(|server| server.config.name).collect();
    TrustStatus { trusted: is_trusted(root), mcp_servers, hook_events: crate::hooks::project_hook_events(root) }
}

/// The window's commands: `project_trust_status` and `trust_project`.
pub mod commands {
    use std::path::PathBuf;

    use tauri::{AppHandle, Manager};

    use super::TrustStatus;
    use crate::AppState;

    fn open_root(app: &AppHandle) -> Result<PathBuf, String> {
        app.state::<AppState>().root.lock().map_err(|e| e.to_string())?.clone().ok_or_else(|| "Open a project first".to_string())
    }

    #[tauri::command]
    pub fn project_trust_status(app: AppHandle) -> Result<TrustStatus, String> {
        Ok(super::project_trust(&open_root(&app)?))
    }

    /// Trusts the open project, then starts its `.mcp.json` connectors.
    #[tauri::command]
    pub async fn trust_project(app: AppHandle) -> Result<TrustStatus, String> {
        let root = open_root(&app)?;
        super::trust_project(&root)?;
        app.state::<AppState>().mcp.sync_project(Some(&root)).await;
        Ok(super::project_trust(&root))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_store_round_trips() {
        let base = std::env::temp_dir().join(format!("neru-trust-{}", uuid::Uuid::new_v4()));
        let project = base.join("project");
        let inner = project.join("packages").join("web");
        let other = base.join("other");
        fs::create_dir_all(&inner).unwrap();
        fs::create_dir_all(&other).unwrap();
        assert!(!is_trusted(&project));
        trust_project(&project).unwrap();
        trust_project(&project).unwrap();
        assert!(is_trusted(&project));
        assert!(is_trusted(&inner));
        assert!(!is_trusted(&other));
        let store = read_store(&store_path().unwrap());
        assert_eq!(store.iter().filter(|entry| same(entry, &canonical(&project))).count(), 1);
        assert!(!store.iter().any(|entry| entry.starts_with(r"\\?\")));

        // A worktree of a trusted project is trusted too.
        let tree = base.join("tree");
        fs::create_dir_all(&tree).unwrap();
        fs::write(tree.join(".git"), format!("gitdir: {}\n", project.join(".git").join("worktrees").join("tree").display())).unwrap();
        assert!(is_trusted(&tree));
        let _ = fs::remove_dir_all(base);
    }
}
