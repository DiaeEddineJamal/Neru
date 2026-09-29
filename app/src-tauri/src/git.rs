use std::{
    path::{Path, PathBuf},
    process::Command,
};

use serde::Serialize;
use tauri::State;

use crate::{
    AppState,
    workspace::{ProjectInfo, open_project, project_root, read_limited, resolve_new},
};

#[tauri::command]
pub fn clone_project(
    url: String,
    destination: String,
    state: State<'_, AppState>,
) -> Result<ProjectInfo, String> {
    let parsed = reqwest::Url::parse(url.trim()).map_err(|_| "Enter an HTTPS or SSH Git URL")?;
    if !matches!(parsed.scheme(), "https" | "ssh")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty() && parsed.scheme() == "https"
        || parsed.password().is_some()
    {
        return Err("Use an HTTPS or SSH Git URL without embedded credentials".into());
    }
    let target = PathBuf::from(destination.trim());
    if !target.is_absolute() || target.exists() {
        return Err("Choose a new absolute destination folder".into());
    }
    let parent = target
        .parent()
        .ok_or("Invalid destination")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    #[cfg(windows)]
    if !parent
        .to_string_lossy()
        .to_ascii_lowercase()
        .starts_with(r"\\?\d:\")
        && !parent
            .to_string_lossy()
            .to_ascii_lowercase()
            .starts_with(r"d:\")
    {
        return Err("Neru projects must be cloned to D: on this computer".into());
    }
    let name = target.file_name().ok_or("Invalid destination")?;
    let target = parent.join(name);
    let output = Command::new("git")
        .args(["clone", "--", url.trim(), target.to_string_lossy().as_ref()])
        .output()
        .map_err(|e| format!("Git unavailable: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    open_project(target.to_string_lossy().to_string(), state)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    pub branch: String,
    pub files: Vec<GitFile>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitFile {
    pub status: String,
    pub path: String,
    pub staged: bool,
}

pub fn git(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| format!("Git unavailable: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub fn status(root: &Path) -> Result<GitStatus, String> {
    let branch = git(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .unwrap_or_else(|_| "detached HEAD".into())
        .trim()
        .to_string();
    let output = git(root, &["-c", "core.quotepath=false", "status", "--short"])?;
    let files = output
        .lines()
        .filter(|line| line.len() >= 3)
        .map(|line| {
            let code = &line[..2];
            GitFile {
                status: code.to_string(),
                path: line[3..].to_string(),
                staged: code.chars().next().is_some_and(|c| c != ' ' && c != '?'),
            }
        })
        .collect();
    Ok(GitStatus { branch, files })
}

#[tauri::command]
pub fn git_status(state: State<'_, AppState>) -> Result<GitStatus, String> {
    status(&project_root(&state)?)
}

#[tauri::command]
pub fn git_diff(
    path: Option<String>,
    staged: bool,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let root = project_root(&state)?;
    let mut args = vec!["diff"];
    if staged {
        args.push("--cached");
    }
    args.push("--");
    if let Some(ref p) = path {
        let _ = resolve_new(&root, p)?;
        args.push(p);
    }
    let diff = git(&root, &args)?;
    if diff.is_empty() && !staged {
        if let Some(p) = path {
            if git(&root, &["ls-files", "--error-unmatch", "--", &p]).is_err() {
                let target = resolve_new(&root, &p)?;
                if target.is_file() {
                    let content = read_limited(&target)?;
                    return Ok(similar::TextDiff::from_lines("", &content)
                        .unified_diff()
                        .header("/dev/null", &format!("b/{p}"))
                        .to_string());
                }
            }
        }
    }
    Ok(diff)
}

#[tauri::command]
pub fn git_stage(path: String, state: State<'_, AppState>) -> Result<(), String> {
    let root = project_root(&state)?;
    let _ = resolve_new(&root, &path)?;
    git(&root, &["add", "--", &path]).map(|_| ())
}

#[tauri::command]
pub fn git_unstage(path: String, state: State<'_, AppState>) -> Result<(), String> {
    let root = project_root(&state)?;
    let _ = resolve_new(&root, &path)?;
    git(&root, &["restore", "--staged", "--", &path]).map(|_| ())
}

#[tauri::command]
pub fn git_commit(message: String, state: State<'_, AppState>) -> Result<String, String> {
    let root = project_root(&state)?;
    if message.trim().is_empty() || message.len() > 200 {
        return Err("Commit message must be 1–200 characters".into());
    }
    git(&root, &["commit", "-m", &message])
}

#[tauri::command]
pub fn git_branches(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let root = project_root(&state)?;
    Ok(git(&root, &["branch", "--format=%(refname:short)"])?
        .lines()
        .map(str::to_string)
        .collect())
}

#[tauri::command]
pub fn git_create_branch(name: String, state: State<'_, AppState>) -> Result<String, String> {
    let root = project_root(&state)?;
    if name.len() > 100 || name.trim().is_empty() {
        return Err("Invalid branch name".into());
    }
    git(&root, &["check-ref-format", "--branch", &name])?;
    git(&root, &["switch", "-c", &name])
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteInfo {
    pub branch: String,
    pub base: String,
    pub remote: Option<String>,
    /// Commits on this branch that the remote does not have yet (None when it was never pushed).
    pub ahead: Option<u32>,
    /// Commits on the upstream that this branch does not have yet.
    pub behind: Option<u32>,
    pub github: Option<String>,
    /// Web URL for GitHub, GitLab, or Bitbucket.
    pub web: Option<String>,
    pub gh_cli: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub url: String,
    /// True when the PR was created with the GitHub CLI; false when the compare page was opened.
    pub created: bool,
}

/// `https://github.com/owner/repo` for GitHub remotes in HTTPS or SSH form.
pub fn github_web_url(remote: &str) -> Option<String> {
    forge_web_url(remote).filter(|url| url.contains("://github.com/"))
}

/// Web root for a GitHub, GitLab, or Bitbucket remote.
pub fn forge_web_url(remote: &str) -> Option<String> {
    let remote = remote.trim().trim_end_matches('/').trim_end_matches(".git");
    let (host, path) = if let Some(path) = remote.strip_prefix("https://") {
        let (host, path) = path.split_once('/')?;
        (host, path)
    } else if let Some(rest) = remote.strip_prefix("git@") {
        let (host, path) = rest.split_once(':')?;
        (host, path)
    } else if let Some(rest) = remote.strip_prefix("ssh://git@") {
        let (host, path) = rest.split_once('/')?;
        (host, path)
    } else {
        return None;
    };
    if !matches!(host, "github.com" | "gitlab.com" | "bitbucket.org") {
        return None;
    }
    let mut parts = path.split('/');
    let (owner, repo) = (parts.next()?, parts.next()?);
    (!owner.is_empty() && !repo.is_empty() && parts.next().is_none())
        .then(|| format!("https://{host}/{owner}/{repo}"))
}

fn current_branch(root: &Path) -> Result<String, String> {
    let branch = git(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .map_err(|_| "Check out a branch before pushing".to_string())?;
    Ok(branch.trim().to_string())
}

fn base_branch(state: &AppState, root: &Path) -> String {
    if let Ok(shared) = crate::sessions::active(state) {
        if let Some(tree) = shared
            .lock()
            .ok()
            .and_then(|runtime| runtime.summary.worktree.clone())
        {
            return tree.base;
        }
    }
    git(
        root,
        &["symbolic-ref", "--quiet", "--short", "refs/remotes/origin/HEAD"],
    )
    .ok()
    .and_then(|name| name.trim().strip_prefix("origin/").map(str::to_string))
    .unwrap_or_else(|| "main".into())
}

fn glab_available() -> bool {
    Command::new("glab").arg("version").output().is_ok_and(|output| output.status.success())
}

fn compare_url(web: &str, base: &str, branch: &str, title: Option<&str>, body: Option<&str>) -> String {
    let encode = |value: &str| {
        value.bytes().flat_map(|byte| {
            let c = byte as char;
            if c.is_ascii_alphanumeric() || b"-_.~".contains(&byte) {
                vec![c]
            } else {
                format!("%{byte:02X}").chars().collect()
            }
        }).collect::<String>()
    };
    if web.contains("://gitlab.com/") {
        format!("{web}/-/merge_requests/new?merge_request[source_branch]={}&merge_request[target_branch]={}&merge_request[title]={}&merge_request[description]={}", encode(branch), encode(base), encode(title.unwrap_or("")), encode(body.unwrap_or("")))
    } else if web.contains("://bitbucket.org/") {
        format!("{web}/pull-requests/new?source={}&dest={}", encode(branch), encode(base))
    } else {
        format!("{web}/compare/{base}...{branch}?expand=1&title={}&body={}", encode(title.unwrap_or("")), encode(body.unwrap_or("")))
    }
}

fn gh_available() -> bool {
    Command::new("gh")
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

#[tauri::command]
pub async fn git_remote_info(app: tauri::AppHandle) -> Result<RemoteInfo, String> {
    use tauri::Manager;
    let state = app.state::<AppState>();
    let root = project_root(&state)?;
    let branch = current_branch(&root).unwrap_or_else(|_| "detached HEAD".into());
    let base = base_branch(&state, &root);
    tauri::async_runtime::spawn_blocking(move || {
        let remote = git(&root, &["remote", "get-url", "origin"])
            .ok()
            .map(|url| url.trim().to_string());
        let ahead = git(&root, &["rev-list", "--count", "@{upstream}..HEAD"])
            .ok()
            .and_then(|count| count.trim().parse().ok());
        let behind = git(&root, &["rev-list", "--count", "HEAD..@{upstream}"])
            .ok()
            .and_then(|count| count.trim().parse().ok());
        let web = remote.as_deref().and_then(forge_web_url);
        Ok(RemoteInfo {
            github: web.clone().filter(|url| url.contains("://github.com/")),
            web,
            remote,
            ahead,
            behind,
            branch,
            base,
            gh_cli: gh_available(),
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Pushes the current branch to origin, setting it as upstream.
#[tauri::command]
pub async fn git_push(app: tauri::AppHandle) -> Result<String, String> {
    use tauri::Manager;
    let root = project_root(&app.state::<AppState>())?;
    tauri::async_runtime::spawn_blocking(move || {
        let branch = current_branch(&root)?;
        git(&root, &["remote", "get-url", "origin"])
            .map_err(|_| "This repository has no origin remote".to_string())?;
        let output = Command::new("git")
            .args(["push", "--set-upstream", "origin", &branch])
            .env("GIT_TERMINAL_PROMPT", "0")
            .current_dir(&root)
            .output()
            .map_err(|e| format!("Git unavailable: {e}"))?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if !output.status.success() {
            return Err(text.trim().to_string());
        }
        Ok(text.trim().to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Opens a pull request for the current branch: with the GitHub CLI when it is installed and
/// signed in, otherwise by opening GitHub's compare page in the browser.
#[tauri::command]
pub async fn create_pull_request(
    title: Option<String>,
    body: Option<String>,
    app: tauri::AppHandle,
) -> Result<PullRequest, String> {
    use tauri::Manager;
    let state = app.state::<AppState>();
    let root = project_root(&state)?;
    let base = base_branch(&state, &root);
    tauri::async_runtime::spawn_blocking(move || {
        let branch = current_branch(&root)?;
        if branch == base {
            return Err(format!(
                "You are on {base}. Create or switch to a feature branch first"
            ));
        }
        let title = title.as_deref().map(str::trim).filter(|title| !title.is_empty()).map(str::to_string);
        let body = body.as_deref().map(str::trim).filter(|body| !body.is_empty()).map(str::to_string);
        if gh_available() {
            let mut command = Command::new("gh");
            command.args(["pr", "create", "--head", &branch, "--base", &base]);
            match &title {
                Some(title) => {
                    command.args(["--title", title, "--body", body.as_deref().unwrap_or("Opened from Neru.")]);
                }
                None => {
                    command.arg("--fill");
                }
            };
            let output = command
                .env("GH_PROMPT_DISABLED", "1")
                .current_dir(&root)
                .output()
                .map_err(|e| e.to_string())?;
            if output.status.success() {
                let url = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .rev()
                    .find(|line| line.starts_with("https://"))
                    .unwrap_or_default()
                    .to_string();
                return Ok(PullRequest { url, created: true });
            }
            let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if !error.contains("auth") && !error.contains("login") {
                return Err(error);
            }
        }
        let remote = git(&root, &["remote", "get-url", "origin"])
            .map_err(|_| "This repository has no origin remote".to_string())?;
        let web = forge_web_url(&remote).ok_or("Pull requests from Neru support GitHub, GitLab, and Bitbucket remotes")?;
        if web.contains("://gitlab.com/") && glab_available() {
            let mut command = Command::new("glab");
            command.args(["mr", "create", "--source-branch", &branch, "--target-branch", &base, "--yes"]);
            if let Some(title) = &title {
                command.args(["--title", title]);
            }
            if let Some(body) = &body {
                command.args(["--description", body]);
            }
            let output = command.current_dir(&root).output().map_err(|e| e.to_string())?;
            if output.status.success() {
                let url = String::from_utf8_lossy(&output.stdout).lines().rev().find(|line| line.starts_with("https://")).unwrap_or_default().to_string();
                return Ok(PullRequest { url, created: true });
            }
        }
        let url = compare_url(&web, &base, &branch, title.as_deref(), body.as_deref());
        crate::web::open_url(url.clone())?;
        Ok(PullRequest { url, created: false })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod remote_tests {
    use super::github_web_url;

    #[test]
    fn recognises_github_remotes() {
        assert_eq!(
            github_web_url("https://github.com/o/r.git").as_deref(),
            Some("https://github.com/o/r")
        );
        assert_eq!(
            github_web_url("git@github.com:o/r.git").as_deref(),
            Some("https://github.com/o/r")
        );
        assert_eq!(github_web_url("https://gitlab.com/o/r.git"), None);
        assert_eq!(super::forge_web_url("git@gitlab.com:o/r.git").as_deref(), Some("https://gitlab.com/o/r"));
        assert_eq!(super::forge_web_url("https://bitbucket.org/o/r").as_deref(), Some("https://bitbucket.org/o/r"));
    }
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub name: String,
    /// "pending", "success", "failure" or "skipped".
    pub state: String,
    pub url: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrStatus {
    pub number: u64,
    pub url: String,
    pub title: String,
    pub state: String,
    pub checks: Vec<Check>,
}

fn check_state(item: &serde_json::Value) -> String {
    let status = item["status"].as_str().unwrap_or("").to_uppercase();
    let conclusion = item["conclusion"].as_str().unwrap_or("").to_uppercase();
    let context_state = item["state"].as_str().unwrap_or("").to_uppercase();
    match (status.as_str(), conclusion.as_str(), context_state.as_str()) {
        (_, "SUCCESS", _) | (_, _, "SUCCESS") => "success",
        (_, "FAILURE" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED" | "STARTUP_FAILURE", _) | (_, _, "FAILURE" | "ERROR") => "failure",
        (_, "SKIPPED" | "NEUTRAL" | "STALE", _) => "skipped",
        _ => "pending",
    }
    .into()
}

pub fn parse_pr(body: &serde_json::Value) -> PrStatus {
    let checks = body["statusCheckRollup"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|item| Check {
            name: item["name"].as_str().or(item["context"].as_str()).unwrap_or("check").to_string(),
            state: check_state(item),
            url: item["detailsUrl"].as_str().or(item["targetUrl"].as_str()).map(String::from),
        })
        .collect();
    PrStatus {
        number: body["number"].as_u64().unwrap_or(0),
        url: body["url"].as_str().unwrap_or("").into(),
        title: body["title"].as_str().unwrap_or("").into(),
        state: body["state"].as_str().unwrap_or("").into(),
        checks,
    }
}

fn gh(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("gh")
        .args(args)
        .env("GH_PROMPT_DISABLED", "1")
        .current_dir(root)
        .output()
        .map_err(|_| "Install the GitHub CLI (gh) and run `gh auth login` to follow CI checks".to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// The pull request for the current branch and its CI checks, or None when there is none.
#[tauri::command]
pub async fn pull_request_status(app: tauri::AppHandle) -> Result<Option<PrStatus>, String> {
    use tauri::Manager;
    let root = project_root(&app.state::<AppState>())?;
    tauri::async_runtime::spawn_blocking(move || {
        match gh(&root, &["pr", "view", "--json", "number,url,title,state,statusCheckRollup"]) {
            Ok(text) => {
                let body: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                Ok(Some(parse_pr(&body)))
            }
            Err(error) if error.contains("no pull requests found") => Ok(None),
            Err(error) => Err(error),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The failing log lines of the latest failed workflow run on this branch, for Neru to fix.
#[tauri::command]
pub async fn failed_check_log(app: tauri::AppHandle) -> Result<String, String> {
    use tauri::Manager;
    let root = project_root(&app.state::<AppState>())?;
    tauri::async_runtime::spawn_blocking(move || {
        let branch = current_branch(&root)?;
        let runs: serde_json::Value = serde_json::from_str(&gh(
            &root,
            &["run", "list", "--branch", &branch, "--limit", "10", "--json", "databaseId,conclusion,name"],
        )?)
        .map_err(|e| e.to_string())?;
        let run = runs
            .as_array()
            .and_then(|runs| runs.iter().find(|run| run["conclusion"] == "failure"))
            .ok_or("No failed workflow run on this branch")?;
        let id = run["databaseId"].as_u64().ok_or("Run without an id")?.to_string();
        let log = gh(&root, &["run", "view", &id, "--log-failed"])?;
        let tail: String = log.chars().rev().take(20_000).collect::<Vec<_>>().into_iter().rev().collect();
        Ok(format!("Workflow \"{}\" (run {id}) failed. Failing steps:\n{tail}", run["name"].as_str().unwrap_or("CI")))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod ci_tests {
    use super::parse_pr;

    #[test]
    fn maps_check_runs_and_status_contexts() {
        let body = serde_json::json!({"number": 7, "url": "https://github.com/o/r/pull/7", "title": "Fix", "state": "OPEN", "statusCheckRollup": [
            {"name": "test", "status": "COMPLETED", "conclusion": "FAILURE", "detailsUrl": "https://x"},
            {"name": "lint", "status": "IN_PROGRESS", "conclusion": ""},
            {"context": "ci/netlify", "state": "SUCCESS", "targetUrl": "https://y"}
        ]});
        let pr = parse_pr(&body);
        assert_eq!(pr.number, 7);
        let states: Vec<_> = pr.checks.iter().map(|check| check.state.as_str()).collect();
        assert_eq!(states, ["failure", "pending", "success"]);
        assert_eq!(pr.checks[2].name, "ci/netlify");
    }
}

fn branch_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() || name.len() > 200 || name.contains("..") || name.contains(' ') {
        return Err("Invalid branch name".into());
    }
    Ok(name.to_string())
}

async fn git_on_project(app: tauri::AppHandle, args: Vec<String>) -> Result<String, String> {
    use tauri::Manager;
    let root = project_root(&app.state::<AppState>())?;
    tauri::async_runtime::spawn_blocking(move || {
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        git(&root, &refs)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn git_fetch(app: tauri::AppHandle) -> Result<String, String> {
    git_on_project(app, vec!["fetch".into(), "--prune".into()]).await
}

#[tauri::command]
pub async fn git_pull(app: tauri::AppHandle) -> Result<String, String> {
    git_on_project(app, vec!["pull".into(), "--no-rebase".into()]).await
}

#[tauri::command]
pub async fn git_checkout(name: String, app: tauri::AppHandle) -> Result<String, String> {
    let name = branch_name(&name)?;
    git_on_project(app, vec!["switch".into(), name]).await
}

#[tauri::command]
pub async fn git_merge(name: String, app: tauri::AppHandle) -> Result<String, String> {
    let name = branch_name(&name)?;
    git_on_project(app, vec!["merge".into(), "--no-edit".into(), "--".into(), name]).await
}

#[tauri::command]
pub async fn git_rebase(name: String, app: tauri::AppHandle) -> Result<String, String> {
    let name = branch_name(&name)?;
    git_on_project(app, vec!["rebase".into(), name]).await
}

/// `push` stashes the working tree. `pop` restores the latest stash.
#[tauri::command]
pub async fn git_stash(action: String, app: tauri::AppHandle) -> Result<String, String> {
    let args = match action.as_str() {
        "push" => vec!["stash".into(), "push".into(), "-u".into(), "-m".into(), "Neru".into()],
        "pop" => vec!["stash".into(), "pop".into()],
        _ => return Err("Stash action must be push or pop".into()),
    };
    git_on_project(app, args).await
}

/// Squash-merges the current branch's pull request once checks have passed.
#[tauri::command]
pub async fn merge_pull_request(app: tauri::AppHandle) -> Result<String, String> {
    use tauri::Manager;
    let root = project_root(&app.state::<AppState>())?;
    tauri::async_runtime::spawn_blocking(move || {
        if !gh_available() {
            return Err("Install the GitHub CLI to merge pull requests from Neru".into());
        }
        gh(&root, &["pr", "merge", "--squash"])
    })
    .await
    .map_err(|e| e.to_string())?
}
