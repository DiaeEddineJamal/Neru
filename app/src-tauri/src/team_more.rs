//! Team's task-wide tools: the combined diff, the task's pull request, member terminals inside
//! Neru, each agent's models and own slash commands, CLI paths and updates, smart execution of
//! tickets, and the usage export.
use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::Serialize;
use serde_json::json;
use tauri::AppHandle;

use crate::{
    team::{self, Post},
    team_agents::{self, Kind},
    team_tools,
};

/// The folder a member works in (its worktree, else the project), or the project for no member.
fn place(id: &str, handle: Option<&str>) -> Result<PathBuf, String> {
    let shared = team::load(id)?;
    let task = team::lock(&shared)?;
    let member = handle.and_then(|handle| task.members.iter().find(|m| m.handle == handle));
    Ok(match (member.and_then(|m| m.worktree.as_ref()), task.project_path.is_empty()) {
        (Some(tree), _) => PathBuf::from(&tree.path),
        (None, true) => team::folder(id)?,
        (None, false) => PathBuf::from(&task.project_path),
    })
}

/// A program for one of Neru's terminal tabs (see `terminal::Launch`).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchView {
    pub cwd: String,
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub title: String,
}

/// Same-key, normalized paths: `\\?\D:\x` and `D:/x/` are one checkout.
fn same_path(a: &str, b: &str) -> bool {
    let clean = |path: &str| path.trim_start_matches(r"\\?\").replace('\\', "/").trim_end_matches('/').to_lowercase();
    clean(a) == clean(b)
}

pub mod commands {
    use super::*;

    /// Every file the task changed, per checkout (the project and each member worktree), from the
    /// tree before the task's first change there to now, with unified diffs.
    #[tauri::command]
    pub async fn team_task_changes(id: String) -> Result<Vec<team_tools::TaskFile>, String> {
        let shared = team::load(&id)?;
        let places: Vec<(PathBuf, String, String)> = {
            let task = team::lock(&shared)?;
            let mut roots: Vec<(String, String)> = Vec::new();
            if !task.project_path.is_empty() {
                roots.push((task.project_path.clone(), "project".into()));
            }
            for member in &task.members {
                if let Some(tree) = &member.worktree {
                    roots.push((tree.path.clone(), member.handle.clone()));
                }
            }
            roots
                .into_iter()
                .filter_map(|(root, name)| {
                    let base = task.posts.iter().filter_map(|post| post.changes.as_ref()).find(|changes| same_path(&changes.root, &root))?.before.clone();
                    Some((PathBuf::from(root), name, base))
                })
                .collect()
        };
        tauri::async_runtime::spawn_blocking(move || places.iter().flat_map(|(root, name, base)| team_tools::changes_since(root, base, name)).collect())
            .await
            .map_err(|e| e.to_string())
    }

    /// The pull request for a member's branch (or the project's) and its checks.
    #[tauri::command]
    pub async fn team_pr_status(id: String, handle: Option<String>) -> Result<Option<crate::git::PrStatus>, String> {
        let root = place(&id, handle.as_deref())?;
        tauri::async_runtime::spawn_blocking(move || crate::git::pr_status_at(&root)).await.map_err(|e| e.to_string())?
    }

    /// The failing CI log for a member's branch (or the project's), to hand to a member to fix.
    #[tauri::command]
    pub async fn team_failed_log(id: String, handle: Option<String>) -> Result<String, String> {
        let root = place(&id, handle.as_deref())?;
        tauri::async_runtime::spawn_blocking(move || crate::git::failed_log_at(&root)).await.map_err(|e| e.to_string())?
    }

    /// The folder a task terminal opens in.
    #[tauri::command]
    pub fn team_task_cwd(id: String, handle: Option<String>) -> Result<String, String> {
        Ok(place(&id, handle.as_deref())?.to_string_lossy().trim_start_matches(r"\\?\").to_string())
    }

    /// The member's own CLI in a Neru terminal tab, continuing its session.
    #[tauri::command]
    pub fn team_terminal_launch(app: AppHandle, id: String, handle: String) -> Result<LaunchView, String> {
        let cwd = place(&id, Some(&handle))?;
        let shared = team::load(&id)?;
        let task = team::lock(&shared)?;
        let member = task.members.iter().find(|m| m.handle == handle).ok_or("No such member")?;
        let kind = Kind::parse(&member.kind).ok_or("Only coding-agent CLIs open in a terminal")?;
        let program = team_agents::program(kind).ok_or_else(|| format!("{} is not installed", kind.name()))?;
        let args: Vec<String> = match (kind, member.upstream.as_deref()) {
            (Kind::Claude | Kind::Cursor | Kind::Gemini | Kind::Qwen, Some(session)) => vec!["--resume".into(), session.into()],
            (Kind::Codex, Some(session)) => vec!["resume".into(), session.into()],
            (Kind::OpenCode, Some(session)) => vec!["--session".into(), session.into()],
            _ => Vec::new(),
        };
        let env = if kind == Kind::Gemini { team::gemini_env(&app, None) } else { Vec::new() };
        Ok(LaunchView { cwd: cwd.to_string_lossy().trim_start_matches(r"\\?\").to_string(), program: program.to_string_lossy().into_owned(), args, env, title: format!("@{handle}") })
    }

    /// Models a member can pick: the CLI's own list where it keeps one, else its documented aliases.
    #[tauri::command]
    pub async fn list_agent_models(kind: String) -> Result<Vec<String>, String> {
        let home = dirs::home_dir().unwrap_or_default();
        Ok(match kind.as_str() {
            "claude" => ["sonnet", "opus", "haiku", "opusplan", "sonnet[1m]"].map(String::from).to_vec(),
            "gemini" => ["auto", "pro", "flash", "flash-lite"].map(String::from).to_vec(),
            "codex" => {
                let codex = std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".codex"));
                let cache: serde_json::Value = fs::read(codex.join("models_cache.json")).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
                cache["models"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|model| model["visibility"] != "hide")
                    .filter_map(|model| model["slug"].as_str().map(String::from))
                    .collect()
            }
            "opencode" | "cursor" => {
                let kind = Kind::parse(&kind).ok_or("Unknown agent")?;
                let Some(program) = team_agents::program(kind) else { return Ok(Vec::new()) };
                tauri::async_runtime::spawn_blocking(move || {
                    let mut command = crate::cli_setup::command(&program);
                    command.arg("models");
                    let text = crate::cli_setup::output_within(command, std::time::Duration::from_secs(20)).unwrap_or_default();
                    // One model per line ("provider/model" or "model - Name"); keep the id.
                    text.lines()
                        .filter_map(|line| line.split_whitespace().next())
                        .filter(|id| id.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:[]".contains(c)) && id.chars().any(|c| c.is_ascii_alphabetic()) && id.len() > 2)
                        .map(String::from)
                        .collect()
                })
                .await
                .map_err(|e| e.to_string())?
            }
            _ => Vec::new(),
        })
    }

    /// A slash command one agent keeps in its own folders, expanded by Neru before sending.
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    pub struct AgentCommand {
        pub agent: String,
        pub name: String,
        pub description: String,
        /// The prompt, with `$ARGUMENTS` / `{{args}}` where the arguments go.
        pub body: String,
    }

    /// Each agent's own slash commands, from its user folder and the open project's.
    #[tauri::command]
    pub fn list_agent_commands(app: AppHandle) -> Vec<AgentCommand> {
        use tauri::Manager;
        let project = app.state::<crate::AppState>().root.lock().ok().and_then(|root| root.clone());
        let home = dirs::home_dir().unwrap_or_default();
        let config = dirs::config_dir().unwrap_or_else(|| home.join(".config"));
        let mut dirs: Vec<(&str, PathBuf, &str)> = vec![
            ("claude", home.join(".claude").join("commands"), ""),
            ("codex", home.join(".codex").join("prompts"), "prompts:"),
            ("gemini", home.join(".gemini").join("commands"), ""),
            ("qwen", home.join(".qwen").join("commands"), ""),
            ("opencode", home.join(".config").join("opencode").join("command"), ""),
            ("opencode", config.join("opencode").join("command"), ""),
            ("cursor", home.join(".cursor").join("commands"), ""),
        ];
        if let Some(project) = &project {
            dirs.push(("claude", project.join(".claude").join("commands"), ""));
            dirs.push(("gemini", project.join(".gemini").join("commands"), ""));
            dirs.push(("qwen", project.join(".qwen").join("commands"), ""));
            dirs.push(("opencode", project.join(".opencode").join("command"), ""));
            dirs.push(("cursor", project.join(".cursor").join("commands"), ""));
        }
        let mut found: Vec<AgentCommand> = Vec::new();
        for (agent, dir, prefix) in dirs {
            collect(agent, &dir, &dir, prefix, &mut found);
        }
        found.sort_by(|a, b| (a.agent.as_str(), a.name.as_str()).cmp(&(b.agent.as_str(), b.name.as_str())));
        found.dedup_by(|a, b| a.agent == b.agent && a.name == b.name);
        found
    }

    fn collect(agent: &str, base: &Path, dir: &Path, prefix: &str, found: &mut Vec<AgentCommand>) {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect(agent, base, &path, prefix, found);
                continue;
            }
            let ext = path.extension().and_then(|ext| ext.to_str()).unwrap_or_default();
            if !matches!(ext, "md" | "toml") || found.len() > 400 {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else { continue };
            // Subfolders become namespaces: commands/git/commit.md is /git:commit.
            let relative = path.strip_prefix(base).unwrap_or(&path).with_extension("");
            let name = format!("{prefix}{}", relative.to_string_lossy().replace(['\\', '/'], ":"));
            let (description, body) = if ext == "toml" { toml_command(&text) } else { md_command(&text) };
            if !body.trim().is_empty() {
                found.push(AgentCommand { agent: agent.into(), name, description, body });
            }
        }
    }

    /// A Markdown command: optional front matter with `description:`, then the prompt.
    fn md_command(text: &str) -> (String, String) {
        let (description, _, _) = team::front_fields(text);
        let description = description.or_else(|| {
            text.strip_prefix("---")?.split("\n---").next()?.lines().find_map(|line| line.trim().strip_prefix("description:").map(|value| value.trim().trim_matches('"').to_string()))
        });
        let body = match text.strip_prefix("---") {
            Some(rest) => rest.split_once("\n---").map(|(_, body)| body.trim_start_matches(['-', '\r', '\n']).to_string()).unwrap_or_default(),
            None => text.to_string(),
        };
        (description.unwrap_or_else(|| body.lines().find(|line| !line.trim().is_empty()).unwrap_or_default().chars().take(80).collect()), body)
    }

    /// A Gemini/Qwen TOML command: `description = "…"` and `prompt = """…"""`.
    fn toml_command(text: &str) -> (String, String) {
        let value = |key: &str| -> Option<String> {
            let start = text.find(&format!("{key} ="))? + key.len() + 2;
            let rest = text[start..].trim_start();
            if let Some(rest) = rest.strip_prefix("\"\"\"") {
                return rest.split_once("\"\"\"").map(|(value, _)| value.trim_start_matches(['\r', '\n']).to_string());
            }
            let rest = rest.strip_prefix('"')?;
            rest.split_once('"').map(|(value, _)| value.replace("\\n", "\n"))
        };
        (value("description").unwrap_or_default(), value("prompt").unwrap_or_default())
    }

    /// Uses a CLI at `path` for `kind` instead of the one on PATH; None goes back to PATH.
    #[tauri::command]
    pub fn set_agent_path(kind: String, path: Option<String>) -> Result<(), String> {
        team_agents::set_path(Kind::parse(&kind).ok_or("Unknown agent")?, path)
    }

    /// The newest version of an npm-installed CLI.
    #[tauri::command]
    pub async fn agent_latest_version(kind: String) -> Result<Option<String>, String> {
        let kind = Kind::parse(&kind).ok_or("Unknown agent")?;
        tauri::async_runtime::spawn_blocking(move || team_agents::latest_version(kind)).await.map_err(|e| e.to_string())
    }

    /// Installs or updates a CLI. npm packages install in the background at `version` (default
    /// latest) and the output comes back; other installers open in a terminal window.
    #[tauri::command]
    pub async fn install_agent(kind: String, version: Option<String>) -> Result<String, String> {
        let kind = Kind::parse(&kind).ok_or("Unknown agent")?;
        let version = version.filter(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_alphanumeric() || ".-".contains(c))).unwrap_or_else(|| "latest".into());
        let Some(package) = kind.npm_package() else {
            let home = dirs::home_dir().unwrap_or_default();
            let (program, args) = if cfg!(windows) {
                (PathBuf::from("powershell.exe"), vec!["-NoProfile".to_string(), "-NoExit".into(), "-Command".into(), kind.install().into()])
            } else {
                (PathBuf::from("sh"), vec!["-c".to_string(), format!("{}; exec ${{SHELL:-sh}}", kind.install())])
            };
            team_tools::open_terminal(&program, &args, &home, &[])?;
            return Ok(format!("The {} installer opened in a terminal window.", kind.name()));
        };
        let npm = crate::cli_setup::which("npm", &[]).ok_or("Install Node.js (npm) first")?;
        tauri::async_runtime::spawn_blocking(move || {
            let mut command = crate::cli_setup::command(&npm);
            command.args(["install", "-g", &format!("{package}@{version}")]);
            crate::cli_setup::output_within(command, std::time::Duration::from_secs(600)).ok_or_else(|| format!("npm could not install {package}@{version}"))
        })
        .await
        .map_err(|e| e.to_string())?
    }

    /// Who executes the tickets, who reviews them, and how often a ticket may go back.
    #[tauri::command]
    pub fn set_team_execution(id: String, executor: String, reviewer: String, max_rounds: u32) -> Result<team::TaskView, String> {
        let shared = team::load(&id)?;
        let mut task = team::lock(&shared)?;
        if !task.members.iter().any(|m| m.handle == executor) {
            return Err("Pick an executor from the task's members".into());
        }
        if !reviewer.is_empty() && !task.members.iter().any(|m| m.handle == reviewer) {
            return Err("Pick a reviewer from the task's members".into());
        }
        task.execution = team::Execution { executor, reviewer, max_rounds: max_rounds.clamp(0, 5), running: task.execution.running };
        team::save(&task)?;
        team::view(&task)
    }

    /// Smart execution: the executor implements each open ticket in order and the reviewer checks
    /// it, sending it back until it passes or runs out of rounds. Stop ends it after the current turn.
    #[tauri::command]
    pub fn run_team_execution(id: String, app: AppHandle) -> Result<(), String> {
        let shared = team::load(&id)?;
        {
            let mut task = team::lock(&shared)?;
            if task.execution.executor.is_empty() {
                return Err("Choose who executes the tickets first".into());
            }
            if task.execution.running {
                return Err("Smart execution is already running".into());
            }
            task.execution.running = true;
            team::save(&task)?;
        }
        tauri::async_runtime::spawn(async move {
            execute(&app, &id).await;
            if let Ok(shared) = team::load(&id) {
                if let Ok(mut task) = team::lock(&shared) {
                    task.execution.running = false;
                    let _ = team::save(&task);
                    team::emit(&app, &id, "execution", json!({ "running": false }));
                }
            }
        });
        Ok(())
    }

    /// Ends smart execution once the current turn finishes.
    #[tauri::command]
    pub fn stop_team_execution(id: String) -> Result<(), String> {
        let shared = team::load(&id)?;
        let mut task = team::lock(&shared)?;
        task.execution.running = false;
        team::save(&task)
    }

    /// Writes every member's per-turn usage as CSV.
    #[tauri::command]
    pub fn export_team_usage(id: String, destination: String) -> Result<usize, String> {
        let shared = team::load(&id)?;
        let task = team::lock(&shared)?;
        let mut csv = String::from("member,agent,time,input_tokens,output_tokens,cost_usd\n");
        let mut rows = 0;
        for member in &task.members {
            for spend in &member.usage.history {
                let time = utc(spend.at);
                csv.push_str(&format!("@{},{},{time},{},{},{:.4}\n", member.handle, team::kind_name(&member.kind), spend.input, spend.output, spend.cost));
                rows += 1;
            }
        }
        fs::write(&destination, csv).map_err(|e| e.to_string())?;
        Ok(rows)
    }
}

/// "2026-10-03T14:05:09Z" for a time in milliseconds since 1970.
fn utc(ms: u64) -> String {
    let secs = ms / 1000;
    let (days, rest) = ((secs / 86_400) as i64, secs % 86_400);
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60)
}

fn post_notice(app: &AppHandle, id: &str, text: String) {
    let post = Post::notice(text);
    if let Ok(shared) = team::load(id) {
        if let Ok(mut task) = team::lock(&shared) {
            task.posts.push(post.clone());
            let _ = team::save(&task);
        }
    }
    team::emit(app, id, "post", json!({ "post": post }));
}

/// Posts a message from the user to `handle` and runs the turns it starts, to the end.
async fn ask(app: &AppHandle, id: &str, handle: &str, text: String) {
    let post = Post { to: vec![handle.to_string()], kind: team::message_kind(), ..Post::notice(text) };
    if let Ok(shared) = team::load(id) {
        if let Ok(mut task) = team::lock(&shared) {
            task.posts.push(post.clone());
            task.updated_at = team::now();
            let _ = team::save(&task);
        }
    }
    team::emit(app, id, "post", json!({ "post": post }));
    team::route(app.clone(), id.to_string(), vec![handle.to_string()]).await;
}

fn running(id: &str) -> bool {
    team::load(id).ok().and_then(|shared| team::lock(&shared).ok().map(|task| task.execution.running)).unwrap_or(false)
}

async fn execute(app: &AppHandle, id: &str) {
    let Ok(shared) = team::load(id) else { return };
    let Ok(plan) = team::lock(&shared).map(|task| task.execution.clone()) else { return };
    let mut rounds: std::collections::HashMap<String, u32> = Default::default();
    post_notice(app, id, format!("Smart execution started: @{} implements the tickets{}.", plan.executor, if plan.reviewer.is_empty() { String::new() } else { format!(" and @{} reviews each one", plan.reviewer) }));
    loop {
        if !running(id) {
            post_notice(app, id, "Smart execution stopped.".into());
            return;
        }
        let Ok(items) = team::artifacts(id) else { return };
        let mut open: Vec<_> = items.into_iter().filter(|item| item.kind.as_deref() == Some("ticket") && item.status.as_deref() != Some("done")).collect();
        open.sort_by(|a, b| a.path.cmp(&b.path));
        // Tickets still open after their last round.
        let left = open.len();
        let Some(ticket) = open.into_iter().find(|item| rounds.get(&item.path).copied().unwrap_or(0) <= plan.max_rounds) else {
            let rounds = plan.max_rounds + 1;
            post_notice(app, id, if left == 0 { "Smart execution finished: every ticket is done.".into() } else { format!("Smart execution stopped: {left} {} still open after {rounds} {}.", if left == 1 { "ticket is" } else { "tickets are" }, if rounds == 1 { "round" } else { "rounds" }) });
            return;
        };
        let title = ticket.title.clone().unwrap_or_else(|| ticket.path.clone());
        let round = rounds.entry(ticket.path.clone()).or_insert(0);
        *round += 1;
        ask(app, id, &plan.executor, format!(
            "Smart execution, ticket \"{title}\" (artifacts/{}){}.\nImplement it now. Set the ticket's front-matter status to in_progress when you start, and to done only when its checks pass{}. Do not hand off; reply with what you changed.",
            ticket.path,
            if *round > 1 { format!(", round {round}: fix what the review found") } else { String::new() },
            if plan.reviewer.is_empty() { "" } else { " (the reviewer may set it back)" },
        )).await;
        if !running(id) {
            continue;
        }
        if !plan.reviewer.is_empty() {
            ask(app, id, &plan.reviewer, format!(
                "Smart execution review of ticket \"{title}\" (artifacts/{}). Check @{}'s change against the ticket and its checks (git diff, run the tests). If it passes, set the ticket's front-matter status to done. If not, set it to todo and list the fixes in the ticket under a \"Review\" heading. Do not change code yourself and do not hand off.",
                ticket.path, plan.executor
            )).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_times_are_utc_dates() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(1_790_975_010_000), "2026-10-02T21:03:30Z");
        assert_eq!(utc(951_782_400_000), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn checkouts_match_across_path_spellings() {
        assert!(same_path(r"\\?\D:\Neru\demo\", "d:/neru/demo"));
        assert!(!same_path("D:/a", "D:/ab"));
    }
}
