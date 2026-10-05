//! Team: one task, one shared thread, several coding agents. Each member is a CLI the user already
//! pays for (see `team_agents`) or Neru's own agent. A member's turn resumes its own CLI session
//! and starts with everything teammates posted since its last turn, so every agent works from the
//! same context. A reply whose line starts with `@handle` hands the next turn to that teammate.
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::Notify;
use uuid::Uuid;

use crate::{
    AppState, fallback, sessions, team_tools,
    team_agents::{self, Access, Event, Kind},
    workspace::data_dir,
};

/// Hand-offs one message may cause before the team stops and waits for the user.
const MAX_HOPS: usize = 6;
/// How much catch-up a member gets in its prompt; older posts are only in thread.md.
const DIGEST_CHARS: usize = 24_000;
/// How long the routing card counts down before a limited member's request moves on.
const ROUTE_SECONDS: u64 = 10;
/// The Google Gemini model provider's base URL; its saved key also signs Gemini CLI in.
pub const GEMINI_URL: &str = "https://generativelanguage.googleapis.com/v1beta/openai";

/// The Gemini API key Neru may hand to Gemini CLI: the environment's, else the one saved for the
/// Google Gemini provider (Settings → Agents or Model).
pub fn gemini_key(app: &AppHandle) -> Option<String> {
    std::env::var("GEMINI_API_KEY").ok().filter(|key| !key.is_empty()).or_else(|| {
        let state = app.state::<AppState>();
        let keys = state.provider_keys.lock().ok()?;
        crate::settings::key_for(&keys, "gemini", GEMINI_URL)
    })
}

/// Gemini CLI on an API key. Google retired its free personal sign-in, and the user's own
/// settings pin that sign-in, so the CLI runs from a home of its own that selects the key.
/// The user's GEMINI.md memory is copied across.
pub fn gemini_env(app: &AppHandle, mcp: Option<&(String, String)>) -> Vec<(String, String)> {
    let Some(key) = gemini_key(app) else { return Vec::new() };
    let Ok(home) = crate::workspace::data_dir().map(|dir| dir.join("agent-homes").join("gemini")) else { return Vec::new() };
    let config = home.join(".gemini");
    if std::fs::create_dir_all(&config).is_err() {
        return Vec::new();
    }
    let mut settings = json!({ "security": { "auth": { "selectedType": "gemini-api-key" } } });
    if let Some((url, token)) = mcp {
        settings["mcpServers"] = json!({ "neru": { "httpUrl": url, "headers": { "Authorization": format!("Bearer {token}") }, "trust": true } });
    }
    let _ = std::fs::write(config.join("settings.json"), settings.to_string());
    if let Some(memory) = dirs::home_dir().map(|dir| dir.join(".gemini").join("GEMINI.md")).filter(|path| path.is_file()) {
        let _ = std::fs::copy(memory, config.join("GEMINI.md"));
    }
    vec![("GEMINI_API_KEY".into(), key), ("GEMINI_CLI_HOME".into(), home.to_string_lossy().into_owned())]
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Limit {
    pub label: String,
    pub used: f64,
    pub resets_at: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    pub cost: f64,
    pub turns: u32,
    #[serde(default)]
    pub limits: Vec<Limit>,
    /// What each turn used, newest last, for the Usage charts and CSV export.
    #[serde(default)]
    pub history: Vec<Spend>,
}

/// One turn's usage.
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Spend {
    pub at: u64,
    pub input: u64,
    pub output: u64,
    pub cost: f64,
}

/// Turns kept per member in `Usage::history`.
const KEPT_TURNS: usize = 400;

/// A message waiting for busy members; it is posted when they are free, in order.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Queued {
    pub id: String,
    pub text: String,
    pub to: Vec<String>,
    pub at: u64,
}

/// Smart execution: an executor works through the tickets and a reviewer checks each one.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Execution {
    #[serde(default)]
    pub executor: String,
    #[serde(default)]
    pub reviewer: String,
    /// How many times a ticket may go back to the executor after review.
    #[serde(default)]
    pub max_rounds: u32,
    #[serde(default)]
    pub running: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    /// Unique in the task, without the @: "claude", "codex-2".
    pub handle: String,
    /// A `team_agents::Kind` id, or "neru".
    pub kind: String,
    #[serde(default)]
    pub model: String,
    /// Reasoning effort the CLI takes ("low" … "max"); empty for the agent's default.
    #[serde(default)]
    pub effort: String,
    /// Neru's permission mode: "plan" (read-only), "accept_edits" or "bypass".
    #[serde(default)]
    pub mode: String,
    /// The CLI's own session (a Neru session for "neru"), resumed every turn.
    #[serde(default)]
    pub upstream: Option<String>,
    /// Posts this member has seen; the next turn catches up from here.
    #[serde(default)]
    pub seen: usize,
    /// "idle", "working", "failed" or "stopped".
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub usage: Usage,
    /// The member's own Git worktree, so parallel edits do not collide.
    #[serde(default)]
    pub worktree: Option<sessions::Worktree>,
    /// A forked member's next turn continues a copy of `upstream` rather than the session itself.
    #[serde(default)]
    pub fork_next: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Post {
    pub id: String,
    /// "you" or a member handle.
    pub author: String,
    /// Who it was for: handles, empty for the whole team.
    #[serde(default)]
    pub to: Vec<String>,
    pub text: String,
    #[serde(default)]
    pub steps: Vec<String>,
    pub at: u64,
    /// "message", "notice", "error", or "side" for a side chat that does not move the thread on.
    #[serde(default = "message_kind")]
    pub kind: String,
    /// Files the turn changed in a Git project, so it can be undone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changes: Option<team_tools::Changes>,
    /// A "setup" card's state: "running", "ok" or "failed".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Images the turn generated, as file names in the task's images/ folder.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
}

pub(crate) fn message_kind() -> String {
    "message".into()
}

impl Post {
    pub fn notice(text: String) -> Post {
        Post { changes: None, status: None, images: vec![], id: Uuid::new_v4().to_string(), author: "you".into(), to: vec![], text, steps: vec![], at: now(), kind: "notice".into() }
    }

    /// A message brought in from another tool's history.
    pub fn imported(author: &str, text: String) -> Post {
        Post { changes: None, status: None, images: vec![], id: Uuid::new_v4().to_string(), author: author.into(), to: vec![], text, steps: vec![], at: now(), kind: message_kind() }
    }
}

fn yes() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub title: String,
    /// The project the members work in; empty for a task with no folder.
    pub project_path: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub members: Vec<Member>,
    pub posts: Vec<Post>,
    /// Hand a request to another member when one hits its subscription limit.
    #[serde(default = "yes")]
    pub route_on_limit: bool,
    /// Kept at the top of the task list.
    #[serde(default)]
    pub pinned: bool,
    /// Shared tags; "Imported" marks tasks made from other tools' history.
    #[serde(default)]
    pub labels: Vec<String>,
    /// The task list's group heading; empty for none.
    #[serde(default)]
    pub group: String,
    /// A lucide icon name and an accent colour token for the task list.
    #[serde(default)]
    pub icon: String,
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub queue: Vec<Queued>,
    #[serde(default)]
    pub queue_paused: bool,
    #[serde(default)]
    pub execution: Execution,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TaskSummary {
    pub id: String,
    pub title: String,
    pub project_path: String,
    pub updated_at: u64,
    pub members: Vec<MemberBadge>,
    pub running: bool,
    pub pinned: bool,
    pub labels: Vec<String>,
    pub created_at: u64,
    pub group: String,
    pub icon: String,
    pub color: String,
    /// A member's last turn failed.
    pub failed: bool,
    pub posts: usize,
    pub queued: usize,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct MemberBadge {
    pub handle: String,
    pub kind: String,
    pub status: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TaskView {
    #[serde(flatten)]
    pub task: Task,
    pub folder: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewMember {
    pub kind: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub mode: String,
    /// Reasoning effort, empty for the model's default.
    #[serde(default)]
    pub effort: String,
}

type Shared = Arc<Mutex<Task>>;

static TASKS: LazyLock<Mutex<HashMap<String, Shared>>> = LazyLock::new(Default::default);
/// Members mid-turn, keyed "task\nhandle", with what stops them.
static RUNNING: LazyLock<Mutex<HashMap<String, Arc<Notify>>>> = LazyLock::new(Default::default);
/// A routing countdown per task; firing it cancels the hand-over.
static ROUTING: LazyLock<Mutex<HashMap<String, Arc<Notify>>>> = LazyLock::new(Default::default);
static AGENTS: LazyLock<Mutex<Option<Vec<team_agents::Agent>>>> = LazyLock::new(Default::default);
/// When the user last pressed Stop on each task; hand-offs queued before then are dropped.
static STOPPED: LazyLock<Mutex<HashMap<String, u64>>> = LazyLock::new(Default::default);

fn stopped_since(task: &str, since: u64) -> bool {
    STOPPED.lock().is_ok_and(|stops| stops.get(task).is_some_and(|at| *at >= since))
}

/// A generated image of a task's post, by its file name in the task's images/ folder: (MIME type, base64).
pub(crate) fn image_data(id: &str, name: &str) -> Result<(&'static str, String), String> {
    use base64::Engine;
    if name.is_empty() || name.contains(['/', '\\']) || name.contains("..") {
        return Err("Invalid image name".into());
    }
    let bytes = fs::read(folder(id)?.join("images").join(name)).map_err(|_| "That image is gone")?;
    let mime = match name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        _ => "image/png",
    };
    Ok((mime, base64::engine::general_purpose::STANDARD.encode(bytes)))
}

pub(crate) fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

pub(crate) fn folder(id: &str) -> Result<PathBuf, String> {
    Uuid::parse_str(id).map_err(|_| "Invalid task id")?;
    Ok(data_dir()?.join("teams").join(id))
}

pub(crate) fn kind_name(kind: &str) -> String {
    match kind.strip_prefix("custom:") {
        Some(name) => name.to_string(),
        None => Kind::parse(kind).map(Kind::name).unwrap_or("Neru").to_string(),
    }
}

pub(crate) fn save(task: &Task) -> Result<(), String> {
    let dir = folder(&task.id)?;
    fs::create_dir_all(dir.join("artifacts")).map_err(|e| e.to_string())?;
    let target = dir.join("task.json");
    let temp = dir.join("task.json.tmp");
    fs::write(&temp, serde_json::to_vec_pretty(task).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    fs::rename(&temp, &target).map_err(|e| e.to_string())?;
    // Members read the whole thread here with their own file tools, and each member's own
    // transcript (its posts and tool steps) in transcripts/<handle>.md.
    fs::write(dir.join("thread.md"), thread_markdown(task)).map_err(|e| e.to_string())?;
    let transcripts = dir.join("transcripts");
    fs::create_dir_all(&transcripts).map_err(|e| e.to_string())?;
    for member in &task.members {
        let mut text = format!("# @{} ({}) in {}\n\n", member.handle, kind_name(&member.kind), task.title);
        for post in task.posts.iter().filter(|post| post.author == member.handle) {
            text.push_str(&post_markdown(post));
            text.push('\n');
        }
        fs::write(transcripts.join(format!("{}.md", member.handle)), text).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(crate) fn load(id: &str) -> Result<Shared, String> {
    let mut tasks = TASKS.lock().map_err(|e| e.to_string())?;
    if let Some(shared) = tasks.get(id) {
        return Ok(shared.clone());
    }
    let mut task: Task = serde_json::from_slice(&fs::read(folder(id)?.join("task.json")).map_err(|_| "Task not found")?)
        .map_err(|e| e.to_string())?;
    // A turn that was running when Neru closed did not finish.
    for member in &mut task.members {
        if member.status == "working" {
            member.status = "stopped".into();
        }
    }
    task.execution.running = false;
    let shared = Arc::new(Mutex::new(task));
    tasks.insert(id.into(), shared.clone());
    Ok(shared)
}

pub(crate) fn lock(shared: &Shared) -> Result<std::sync::MutexGuard<'_, Task>, String> {
    shared.lock().map_err(|e| e.to_string())
}

fn running_key(task: &str, handle: &str) -> String {
    format!("{task}\n{handle}")
}

pub(crate) fn is_running(task: &str, handle: &str) -> bool {
    RUNNING.lock().is_ok_and(|running| running.contains_key(&running_key(task, handle)))
}

pub(crate) fn view(task: &Task) -> Result<TaskView, String> {
    Ok(TaskView { task: task.clone(), folder: folder(&task.id)?.to_string_lossy().into_owned() })
}

fn summary(task: &Task) -> TaskSummary {
    TaskSummary {
        id: task.id.clone(),
        title: task.title.clone(),
        project_path: task.project_path.clone(),
        updated_at: task.updated_at,
        running: task.members.iter().any(|member| is_running(&task.id, &member.handle)),
        pinned: task.pinned,
        labels: task.labels.clone(),
        created_at: task.created_at,
        group: task.group.clone(),
        icon: task.icon.clone(),
        color: task.color.clone(),
        failed: task.members.iter().any(|member| member.status == "failed"),
        posts: task.posts.iter().filter(|post| post.kind != "notice").count(),
        queued: task.queue.len(),
        members: task
            .members
            .iter()
            .map(|member| MemberBadge { handle: member.handle.clone(), kind: member.kind.clone(), status: member.status.clone() })
            .collect(),
    }
}

pub(crate) fn emit(app: &AppHandle, task: &str, kind: &str, mut payload: Value) {
    payload["taskId"] = json!(task);
    payload["type"] = json!(kind);
    let _ = app.emit("team://event", payload);
}

fn emit_member(app: &AppHandle, task: &Task, handle: &str) {
    if let Some(member) = task.members.iter().find(|member| member.handle == handle) {
        emit(app, &task.id, "member", json!({ "member": member }));
    }
}

/// A handle not yet used in the task: "claude", then "claude-2".
fn new_handle(members: &[Member], kind: &str) -> String {
    let taken = |handle: &str| members.iter().any(|member| member.handle == handle);
    if !taken(kind) {
        return kind.to_string();
    }
    (2..).map(|n| format!("{kind}-{n}")).find(|handle| !taken(handle)).unwrap_or_default()
}

fn new_member(members: &[Member], spec: NewMember) -> Result<Member, String> {
    if spec.kind != "neru" && Kind::parse(&spec.kind).is_none() && !spec.kind.starts_with("custom:") {
        return Err(format!("Unknown agent {}", spec.kind));
    }
    Ok(Member {
        handle: new_handle(members, spec.kind.trim_start_matches("custom:")),
        kind: spec.kind,
        model: spec.model,
        effort: spec.effort,
        mode: if spec.mode.is_empty() { "accept_edits".into() } else { spec.mode },
        upstream: None,
        seen: 0,
        status: "idle".into(),
        error: None,
        usage: Usage::default(),
        worktree: None,
        fork_next: false,
    })
}

fn handle_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// The handles a message addresses. People write "@codex what do you think?" anywhere in a
/// message; agents hand off only on a line that starts with the handle, so naming a teammate in
/// passing does not start a turn. Lines in code blocks never count. Returns (needs a reply, fyi).
pub fn mentions(text: &str, handles: &[String], author: &str, from_user: bool) -> (Vec<String>, Vec<String>) {
    // "@claude @codex compare these" addresses those two; "@claude ask @codex" only Claude.
    if from_user {
        let mut leading = Vec::new();
        let mut rest = text.trim_start();
        while let Some(after) = rest.strip_prefix('@') {
            let handle: String = after.chars().take_while(|c| handle_char(*c)).collect();
            rest = after[handle.len()..].trim_start_matches([' ', ',', '\t']);
            let handle = handle.to_lowercase();
            if handle == "all" || handle == "team" {
                leading.extend(handles.iter().cloned());
            } else if handles.contains(&handle) {
                leading.push(handle);
            }
        }
        if !leading.is_empty() {
            leading.dedup();
            return (leading, Vec::new());
        }
    }
    let (mut reply, mut fyi) = (Vec::new(), Vec::new());
    let mut fenced = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let line_start = trimmed.trim_start_matches(['-', '*', '>', ' ']);
        let mut found = Vec::new();
        if from_user {
            for (at, _) in line.match_indices('@') {
                let before = line[..at].chars().last();
                if before.is_some_and(|c| handle_char(c) || c == '.') {
                    continue; // an email address
                }
                found.push(line[at + 1..].chars().take_while(|c| handle_char(*c)).collect::<String>());
            }
        } else if let Some(rest) = line_start.strip_prefix('@') {
            found.push(rest.chars().take_while(|c| handle_char(*c)).collect::<String>());
        }
        let quiet = line.to_lowercase().contains("(fyi)");
        for handle in found {
            let handle = handle.to_lowercase();
            let targets: Vec<String> = if handle == "all" || handle == "team" {
                handles.iter().filter(|h| *h != author).cloned().collect()
            } else if handles.contains(&handle) && handle != author {
                vec![handle]
            } else {
                continue;
            };
            let list = if quiet { &mut fyi } else { &mut reply };
            for target in targets {
                if !list.contains(&target) {
                    list.push(target);
                }
            }
        }
    }
    fyi.retain(|handle| !reply.contains(handle));
    (reply, fyi)
}

fn who(author: &str) -> String {
    if author == "you" { "The user".into() } else { format!("@{author}") }
}

fn post_markdown(post: &Post) -> String {
    let to = if post.to.is_empty() { String::new() } else { format!(" → {}", post.to.iter().map(|h| format!("@{h}")).collect::<Vec<_>>().join(", ")) };
    let mut text = format!("### {}{to}\n\n{}\n", who(&post.author), post.text.trim());
    if !post.steps.is_empty() {
        text.push_str(&format!("\n_Tools: {}_\n", post.steps.join("; ")));
    }
    text
}

fn thread_markdown(task: &Task) -> String {
    let mut text = format!("# {}\n\n", task.title);
    for post in &task.posts {
        text.push_str(&post_markdown(post));
        text.push('\n');
    }
    text
}

/// What a member missed: posts since its last turn, newest kept when it is too long. Its own posts
/// are left out unless the CLI cannot resume its session and so has no memory of them.
pub fn digest(posts: &[Post], from: usize, handle: &str, include_own: bool) -> String {
    let missed: Vec<&Post> = posts.iter().skip(from).filter(|post| (include_own || post.author != handle) && post.kind != "notice").collect();
    let mut kept: Vec<String> = Vec::new();
    let mut size = 0;
    for post in missed.iter().rev() {
        let text = post_markdown(post);
        if size + text.len() > DIGEST_CHARS && !kept.is_empty() {
            break;
        }
        size += text.len();
        kept.push(text);
    }
    let dropped = missed.len() - kept.len();
    kept.reverse();
    let mut text = String::new();
    if dropped > 0 {
        text.push_str(&format!("({dropped} earlier posts are only in thread.md.)\n\n"));
    }
    text.push_str(&kept.join("\n"));
    text
}

fn preamble(task: &Task, member: &Member, dir: &Path) -> String {
    let team: Vec<String> = task
        .members
        .iter()
        .map(|other| format!("@{} ({}{})", other.handle, kind_name(&other.kind), if other.handle == member.handle { ", you" } else { "" }))
        .collect();
    let project = if task.project_path.is_empty() { "this task".to_string() } else { task.project_path.clone() };
    let dir = dir.to_string_lossy();
    let place = match &member.worktree {
        Some(tree) => format!(" You work in your own Git worktree at {} on branch {}; teammates' edits are in the project folder or their own worktrees.", tree.path, tree.branch),
        None => String::new(),
    };
    format!(
        "You are @{handle} on a Neru team working on {project}. The team is the user and {team}, and everyone shares one thread.{place}\n\
         - What teammates said since your last turn is below. The whole thread is in {dir}/thread.md, and each member's own transcript is in {dir}/transcripts/<handle>.md.\n\
         - Put specs, plans, tickets and reviews the team should share in {dir}/artifacts/ as Markdown.\n\
         - To hand work to a teammate, end your reply with a line that starts with their handle, for example \"@{example} please review the change in src/main.rs\". Add \"(fyi)\" to that line when you need no answer. Otherwise your reply goes back to the user.\n\
         - Build on what teammates already did instead of redoing it.\n\
         - If you have the neru tools (preview_start, browser_open, browser_read), use them to run the app, show pages to the user in Neru's browser, and check a page's text and errors.",
        handle = member.handle,
        team = team.join(", "),
        example = task.members.iter().map(|m| m.handle.as_str()).find(|h| *h != member.handle).unwrap_or("teammate"),
    )
}

/// The prompt for one turn: who the team is, what the member missed, and whose turn it is.
fn prompt_for(task: &Task, member: &Member, dir: &Path) -> String {
    let resumes = Kind::parse(&member.kind).is_none_or(Kind::resumes) && member.upstream.is_some();
    let from = if resumes { member.seen } else { 0 };
    format!(
        "{}\n\n## Thread since your last turn\n\n{}\n\n## Your turn, @{}\nReply to the team.",
        preamble(task, member, dir),
        digest(&task.posts, from, &member.handle, !resumes),
        member.handle
    )
}

/// A failure that another member's subscription may not share.
fn is_limit(error: &str) -> bool {
    let lower = error.to_lowercase();
    fallback::is_rate_limited(error) || ["usage limit", "limit reached", "hit your limit", "out of extra usage", "credit"].iter().any(|m| lower.contains(m))
}

fn friendly(kind: &str, error: &str) -> String {
    let name = kind_name(kind);
    let lower = error.to_lowercase();
    if lower.contains("not logged in") || lower.contains("login required") || lower.contains("please run /login") || lower.contains("auth") && lower.contains("required") {
        return format!("{name} is not signed in. Sign in from Settings → Agents, then try again.");
    }
    if is_limit(error) {
        return format!("{name} hit its subscription limit: {}", error.trim());
    }
    format!("{name}: {}", error.trim())
}

struct Reply {
    text: String,
    steps: Vec<String>,
    images: Vec<String>,
}

/// Runs one member's turn. Returns the teammates it handed off to.
async fn turn(app: &AppHandle, task_id: &str, handle: &str) -> Result<Vec<String>, String> {
    let shared = load(task_id)?;
    let dir = folder(task_id)?;
    let key = running_key(task_id, handle);
    let cancel = Arc::new(Notify::new());
    // A hand-off to a member that is mid-turn waits for that turn, then runs with what it missed.
    let asked = now();
    let mut told = false;
    loop {
        if stopped_since(task_id, asked) {
            emit(app, task_id, "queued", json!({ "handle": handle, "queued": false }));
            return Ok(Vec::new());
        }
        {
            let mut running = RUNNING.lock().map_err(|e| e.to_string())?;
            if !running.contains_key(&key) {
                running.insert(key.clone(), cancel.clone());
                break;
            }
        }
        if !told {
            emit(app, task_id, "queued", json!({ "handle": handle, "queued": true }));
            told = true;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    if told {
        emit(app, task_id, "queued", json!({ "handle": handle, "queued": false }));
    }
    struct Clear(String);
    impl Drop for Clear {
        fn drop(&mut self) {
            if let Ok(mut running) = RUNNING.lock() {
                running.remove(&self.0);
            }
        }
    }
    let _clear = Clear(key);
    let (member, prompt, root) = {
        let mut task = lock(&shared)?;
        let seen = task.posts.len();
        let member = task.members.iter().find(|m| m.handle == handle).cloned().ok_or("That member left the task")?;
        let prompt = prompt_for(&task, &member, &dir);
        if let Some(entry) = task.members.iter_mut().find(|m| m.handle == handle) {
            entry.status = "working".into();
            entry.error = None;
            entry.seen = seen;
        }
        save(&task)?;
        // The thread shows the member at work (its orb, steps and streamed reply) from now on.
        emit_member(app, &task, handle);
        let root = match (&member.worktree, task.project_path.is_empty()) {
            (Some(tree), _) => PathBuf::from(&tree.path),
            (None, true) => dir.clone(),
            (None, false) => PathBuf::from(&task.project_path),
        };
        (member, prompt, root)
    };
    let before = artifact_contents(task_id);
    let snapshot_root = root.clone();
    let tree_before = tokio::task::spawn_blocking(move || team_tools::snapshot(&snapshot_root)).await.ok().flatten();
    let result = if member.kind == "neru" {
        run_neru(app, task_id, &member, prompt, &root, cancel).await
    } else if member.kind.starts_with("custom:") {
        run_custom(app, task_id, &member, prompt, &root, &dir, cancel).await
    } else {
        run_cli(app, task_id, &shared, &member, prompt, &root, &dir, cancel, false).await
    };
    keep_versions(task_id, before);
    let changes = match tree_before {
        Some(before) => {
            let snapshot_root = root.clone();
            tokio::task::spawn_blocking(move || {
                let after = team_tools::snapshot(&snapshot_root)?;
                let files = team_tools::diff(&snapshot_root, &before, &after);
                (!files.is_empty()).then(|| team_tools::Changes { root: snapshot_root.to_string_lossy().into_owned(), before, after, files, undone: false })
            })
            .await
            .ok()
            .flatten()
        }
        None => None,
    };
    match finish(app, task_id, &shared, handle, &member.kind, result, changes)? {
        Finished::Next(next) => Ok(next),
        Finished::Route(other) => Ok(route_to(app, task_id, handle, &other).await),
    }
}

enum Finished {
    /// Teammates the reply handed off to.
    Next(Vec<String>),
    /// The member hit its limit and this teammate can take the request.
    Route(String),
}

/// Posts the turn's reply or failure and settles the member.
fn finish(app: &AppHandle, task_id: &str, shared: &Shared, handle: &str, kind: &str, result: Result<Reply, String>, changes: Option<team_tools::Changes>) -> Result<Finished, String> {
    let mut task = lock(shared)?;
    let handles: Vec<String> = task.members.iter().map(|m| m.handle.clone()).collect();
    let (next, post, status) = match result {
        Ok(reply) => {
            let (next, fyi) = mentions(&reply.text, &handles, handle, false);
            let text = if reply.text.trim().is_empty() { "(No reply text.)".to_string() } else { reply.text };
            let to = next.iter().chain(fyi.iter()).cloned().collect();
            let post = Post { changes: None, status: None, images: reply.images, id: Uuid::new_v4().to_string(), author: handle.into(), to, text, steps: reply.steps, at: now(), kind: message_kind() };
            (Finished::Next(next), post, "idle")
        }
        Err(error) => {
            let stopped = error == "Stopped";
            let text = if stopped { format!("@{handle} was stopped.") } else { friendly(kind, &error) };
            let post = Post { changes: None, status: None, images: vec![], id: Uuid::new_v4().to_string(), author: handle.into(), to: vec![], text, steps: vec![], at: now(), kind: if stopped { "notice".into() } else { "error".into() } };
            let other = task
                .members
                .iter()
                .find(|m| m.handle != handle && m.status != "failed" && !is_running(task_id, &m.handle))
                .map(|m| m.handle.clone())
                .filter(|_| !stopped && is_limit(&error) && task.route_on_limit);
            if let Some(entry) = task.members.iter_mut().find(|m| m.handle == handle) {
                entry.error = (!stopped).then_some(error);
            }
            let next = other.map(Finished::Route).unwrap_or(Finished::Next(Vec::new()));
            (next, post, if stopped { "stopped" } else { "failed" })
        }
    };
    if let Some(entry) = task.members.iter_mut().find(|m| m.handle == handle) {
        entry.status = status.into();
        if status == "idle" {
            entry.usage.turns += 1;
        }
    }
    let post = Post { changes, ..post };
    task.posts.push(post.clone());
    task.updated_at = now();
    save(&task)?;
    emit(app, task_id, "post", json!({ "post": post }));
    emit_member(app, &task, handle);
    Ok(next)
}

/// The routing card: counts down, then gives the request to `other` unless the user cancels.
async fn route_to(app: &AppHandle, task_id: &str, from: &str, other: &str) -> Vec<String> {
    let cancel = Arc::new(Notify::new());
    if let Ok(mut routing) = ROUTING.lock() {
        routing.insert(task_id.into(), cancel.clone());
    }
    emit(app, task_id, "routing", json!({ "from": from, "to": other, "seconds": ROUTE_SECONDS }));
    let moved = tokio::select! {
        _ = tokio::time::sleep(Duration::from_secs(ROUTE_SECONDS)) => true,
        _ = cancel.notified() => false,
    };
    if let Ok(mut routing) = ROUTING.lock() {
        routing.remove(task_id);
    }
    emit(app, task_id, "routing", json!({ "from": from, "to": other, "seconds": 0, "moved": moved }));
    if moved { vec![other.to_string()] } else { Vec::new() }
}

#[allow(clippy::too_many_arguments)]
async fn run_cli(
    app: &AppHandle,
    task_id: &str,
    shared: &Shared,
    member: &Member,
    prompt: String,
    root: &Path,
    dir: &Path,
    cancel: Arc<Notify>,
    side: bool,
) -> Result<Reply, String> {
    let kind = Kind::parse(&member.kind).ok_or("Unknown agent")?;
    // Neru's browser tools, for the CLIs that take an MCP server per run.
    let mcp = if matches!(kind, Kind::Claude | Kind::Codex | Kind::OpenCode | Kind::Gemini) {
        crate::team_mcp::ensure(app).await.ok().map(|(base, token)| (format!("{base}/{task_id}"), token))
    } else {
        None
    };
    // "Agent default" on Codex: a model this CLI and sign-in support, not whatever its config.toml says.
    let model = match (kind, member.model.trim().is_empty()) {
        (Kind::Codex, true) => team_agents::codex_default_model().unwrap_or_default(),
        _ => member.model.clone(),
    };
    let turn = team_agents::Turn {
        kind,
        model: &model,
        effort: &member.effort,
        access: if side { Access::ReadOnly } else { Access::from_mode(&member.mode) },
        resume: if kind.resumes() { member.upstream.as_deref() } else { None },
        // A side chat never moves the member's own session on.
        fork: side || member.fork_next,
        root,
        task_dir: dir,
        prompt,
        env: if kind == Kind::Gemini { gemini_env(app, mcp.as_ref()) } else { Vec::new() },
        mcp: mcp.clone(),
    };
    let mut reply = Reply { text: String::new(), steps: Vec::new(), images: Vec::new() };
    let started = now();
    let mut drawing = false;
    let mut usage = Usage::default();
    let mut session = None;
    let mut limits: Vec<Limit> = Vec::new();
    let result = team_agents::run(turn, cancel, |event| {
        match event {
            Event::Session(id) => session = Some(id),
            Event::Text(text) => {
                if !reply.text.is_empty() {
                    reply.text.push_str("\n\n");
                }
                reply.text.push_str(&text);
            }
            Event::Delta(text) => reply.text.push_str(&text),
            Event::Step(step) => reply.steps.push(step),
            Event::Usage { input, output, cost } => {
                usage.input += input;
                usage.output += output;
                usage.cost += cost;
            }
            Event::Limit { label, used, resets_at } => {
                limits.retain(|limit| limit.label != label);
                limits.push(Limit { label, used, resets_at });
            }
            Event::ImageStart => drawing = true,
            Event::Error(_) | Event::Done => return,
        }
        if !side {
            emit(app, task_id, "live", json!({ "handle": member.handle, "text": reply.text, "steps": reply.steps, "drawing": drawing }));
        }
    })
    .await;
    // Codex saves generated images in its own folder; copy them into the task so the thread (and the
    // phone) can show them.
    if kind == Kind::Codex && !side {
        if let Some(id) = session.as_deref().or(member.upstream.as_deref()) {
            let images = dir.join("images");
            for source in team_agents::codex_images(id, started) {
                let name = format!("{}.{}", Uuid::new_v4(), source.extension().and_then(|e| e.to_str()).unwrap_or("png"));
                if std::fs::create_dir_all(&images).is_ok() && std::fs::copy(&source, images.join(&name)).is_ok() {
                    reply.images.push(name);
                }
            }
        }
    }
    // Codex reports its subscription windows only in its own session log.
    if kind == Kind::Codex {
        if let Some(id) = session.as_deref().or(member.upstream.as_deref()) {
            for (label, used, resets_at) in team_agents::codex_limits(id) {
                limits.retain(|limit| limit.label != label);
                limits.push(Limit { label, used, resets_at });
            }
        }
    }
    // Keep what the turn learned even when it failed: the session to resume and what it cost.
    if let Ok(mut task) = lock(shared) {
        if let Some(entry) = task.members.iter_mut().find(|m| m.handle == member.handle) {
            if session.is_some() && !side {
                entry.upstream = session;
                entry.fork_next = false;
            }
            entry.usage.input += usage.input;
            entry.usage.output += usage.output;
            entry.usage.cost += usage.cost;
            if usage.input + usage.output > 0 || usage.cost > 0.0 {
                entry.usage.history.push(Spend { at: now(), input: usage.input, output: usage.output, cost: usage.cost });
                let extra = entry.usage.history.len().saturating_sub(KEPT_TURNS);
                entry.usage.history.drain(..extra);
            }
            for limit in limits {
                entry.usage.limits.retain(|known| known.label != limit.label);
                entry.usage.limits.push(limit);
            }
        }
    }
    result.map(|_| reply)
}

/// Neru's own agent as a member: a background Neru session runs the turn with the model set up
/// in Neru, so free and API models can work alongside the subscriptions.
async fn run_neru(app: &AppHandle, task_id: &str, member: &Member, prompt: String, root: &Path, cancel: Arc<Notify>) -> Result<Reply, String> {
    let state = app.state::<AppState>();
    let session = match &member.upstream {
        Some(id) if sessions::runtime(&state, id).is_ok() => id.clone(),
        _ => {
            let id = sessions::create_detached(&state, root, &format!("Team · @{}", member.handle))?;
            if let Ok(task) = load(task_id) {
                if let Ok(mut task) = lock(&task) {
                    if let Some(entry) = task.members.iter_mut().find(|m| m.handle == member.handle) {
                        entry.upstream = Some(id.clone());
                    }
                }
            }
            id
        }
    };
    let mode = match Access::from_mode(&member.mode) {
        Access::ReadOnly => "plan",
        Access::Edits => "accept_edits",
        Access::Auto => "auto",
        Access::Full => "bypass",
    };
    let chat = crate::agent::ai_chat(prompt, vec![], mode.into(), None, None, None, None, Some(session.clone()), None, None, app.clone());
    tokio::select! {
        response = chat => {
            let response = response?;
            let mut text = response.content;
            if response.pending.is_some() {
                text.push_str("\n\n(Waiting for your approval in this member's Neru session.)");
            }
            Ok(Reply { text, steps: response.steps, images: Vec::new() })
        }
        _ = cancel.notified() => {
            let _ = crate::agent::stop_chat(Some(session), app.state::<AppState>());
            Err("Stopped".into())
        }
    }
}

/// A custom agent script as a member: it gets the turn's prompt and prints its reply.
async fn run_custom(app: &AppHandle, task_id: &str, member: &Member, prompt: String, root: &Path, dir: &Path, cancel: Arc<Notify>) -> Result<Reply, String> {
    // Scripts live in the main project: a worktree has no copy of an untracked .neru/cli-agents.
    let project = load(task_id).ok().and_then(|shared| lock(&shared).ok().map(|task| task.project_path.clone())).filter(|path| !path.is_empty()).map(PathBuf::from);
    let name = member.kind.trim_start_matches("custom:");
    let agent = team_tools::custom_agents(project.as_deref()).into_iter().find(|agent| agent.name == name).ok_or_else(|| format!("The custom agent {name} is gone from cli-agents"))?;
    let env = [("NERU_TASK_ID", task_id.to_string()), ("NERU_TASK_DIR", dir.to_string_lossy().into_owned()), ("NERU_AGENT_HANDLE", member.handle.clone()), ("NERU_PERMISSION_MODE", member.mode.clone())];
    let text = team_tools::run_custom(Path::new(&agent.path), root, &env, &prompt, cancel, |text| {
        emit(app, task_id, "live", json!({ "handle": member.handle, "text": text, "steps": Vec::<String>::new() }));
    })
    .await?;
    Ok(Reply { text, steps: Vec::new(), images: Vec::new() })
}

/// Runs turns until nobody is handed the conversation, or the hop limit is reached.
pub(crate) async fn route(app: AppHandle, task_id: String, mut targets: Vec<String>) {
    let mut hops = 0;
    let started = now();
    while !targets.is_empty() && !stopped_since(&task_id, started) {
        if hops == MAX_HOPS {
            let post = Post {
                changes: None,
                status: None,
                images: vec![],
                id: Uuid::new_v4().to_string(),
                author: "you".into(),
                to: vec![],
                text: format!("Paused after {MAX_HOPS} hand-offs. Send a message to keep the team going."),
                steps: vec![],
                at: now(),
                kind: "notice".into(),
            };
            if let Ok(shared) = load(&task_id) {
                if let Ok(mut task) = lock(&shared) {
                    task.posts.push(post.clone());
                    let _ = save(&task);
                }
            }
            emit(&app, &task_id, "post", json!({ "post": post }));
            break;
        }
        hops += 1;
        let mut set = tokio::task::JoinSet::new();
        for handle in std::mem::take(&mut targets) {
            let (app, task_id) = (app.clone(), task_id.clone());
            set.spawn(async move { turn(&app, &task_id, &handle).await });
        }
        while let Some(done) = set.join_next().await {
            match done {
                Ok(Ok(next)) => {
                    for handle in next {
                        if !targets.contains(&handle) {
                            targets.push(handle);
                        }
                    }
                }
                Ok(Err(error)) => emit(&app, &task_id, "error", json!({ "error": error })),
                Err(error) => log::warn!("team turn panicked: {error}"),
            }
        }
    }
    if let Ok(shared) = load(&task_id) {
        if let Ok(task) = lock(&shared) {
            emit(&app, &task_id, "idle", json!({ "summary": summary(&task) }));
        }
    }
    drain(&app, &task_id);
}

/// Posts the first queued message whose members are all free, unless the queue is paused.
pub(crate) fn drain(app: &AppHandle, task_id: &str) {
    let Ok(shared) = load(task_id) else { return };
    let (post, targets) = {
        let Ok(mut task) = lock(&shared) else { return };
        if task.queue_paused {
            return;
        }
        let Some(index) = task.queue.iter().position(|item| item.to.iter().all(|handle| !is_running(task_id, handle))) else { return };
        let item = task.queue.remove(index);
        let post = Post { to: item.to.clone(), at: now(), kind: message_kind(), ..Post::notice(item.text) };
        task.posts.push(post.clone());
        task.updated_at = now();
        let _ = save(&task);
        emit(app, task_id, "queue", json!({ "queue": task.queue, "paused": task.queue_paused }));
        (post, item.to)
    };
    emit(app, task_id, "post", json!({ "post": post }));
    tauri::async_runtime::spawn(route(app.clone(), task_id.to_string(), targets));
}

/// The setup card for a new worktree: runs the setup script and reports how it went.
async fn run_setup(app: &AppHandle, id: &str, handle: &str, root: &Path, tree: &sessions::Worktree) {
    let Some(setup) = team_tools::script(root, "setup") else { return };
    let card = Post { kind: "setup".into(), status: Some("running".into()), author: handle.into(), ..Post::notice(format!("Setting up @{handle}'s worktree on {}…", tree.branch)) };
    let card_id = card.id.clone();
    if let Ok(shared) = load(id) {
        if let Ok(mut task) = lock(&shared) {
            task.posts.push(card.clone());
            let _ = save(&task);
        }
    }
    emit(app, id, "post", json!({ "post": card }));
    let (ok, output) = team_tools::run_script(&setup, Path::new(&tree.path), Duration::from_secs(600)).await;
    let text = if output.trim().is_empty() { format!("`{setup}` on {}", tree.branch) } else { format!("`{setup}` on {}\n\n```\n{}\n```", tree.branch, output.trim()) };
    if let Ok(shared) = load(id) {
        if let Ok(mut task) = lock(&shared) {
            if let Some(post) = task.posts.iter_mut().find(|post| post.id == card_id) {
                post.status = Some(if ok { "ok" } else { "failed" }.into());
                post.text = text;
                let post = post.clone();
                let _ = save(&task);
                emit(app, id, "setup", json!({ "post": post }));
            }
        }
    }
}

fn all_tasks() -> Result<Vec<TaskSummary>, String> {
    let dir = data_dir()?.join("teams");
    let mut tasks: Vec<TaskSummary> = fs::read_dir(&dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| load(&entry.file_name().to_string_lossy()).ok())
                .filter_map(|shared| lock(&shared).ok().map(|task| summary(&task)))
                .collect()
        })
        .unwrap_or_default();
    tasks.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(tasks)
}

/// Creates a task from parts, for the UI and for history imports.
/// A new project folder for a task: created (an empty existing folder is fine) and made a Git
/// repository, so members can use worktrees and Undo.
fn start_project(root: &Path) -> Result<(), String> {
    use crate::Hidden;
    if root.is_dir() && std::fs::read_dir(root).map_err(|e| e.to_string())?.next().is_some() {
        return Err(format!("{} already exists and is not empty. Pick it as an existing project instead.", root.display()));
    }
    std::fs::create_dir_all(root).map_err(|e| format!("Could not create {}: {e}", root.display()))?;
    let _ = std::process::Command::new("git").hidden().arg("init").arg("-q").current_dir(root).status();
    Ok(())
}

pub fn create(title: &str, project_path: &str, specs: Vec<NewMember>, posts: Vec<Post>) -> Result<Task, String> {
    let mut members: Vec<Member> = Vec::new();
    for spec in specs {
        let member = new_member(&members, spec)?;
        members.push(member);
    }
    let title = if title.trim().is_empty() { "New task".to_string() } else { title.trim().to_string() };
    let task = Task {
        id: Uuid::new_v4().to_string(),
        title,
        // Agent CLIs choke on Windows' \\?\ verbatim prefix.
        project_path: project_path.trim_start_matches(r"\\?\").into(),
        created_at: now(),
        updated_at: now(),
        members,
        posts,
        route_on_limit: true,
        pinned: false,
        labels: Vec::new(),
        group: String::new(),
        icon: String::new(),
        color: String::new(),
        queue: Vec::new(),
        queue_paused: false,
        execution: Execution::default(),
    };
    save(&task)?;
    TASKS.lock().map_err(|e| e.to_string())?.insert(task.id.clone(), Arc::new(Mutex::new(task.clone())));
    Ok(task)
}

/// Adds an existing CLI session (an imported conversation) as a member that will resume it.
pub fn adopt(task: &mut Task, kind: &str, upstream: Option<String>) -> String {
    let handle = new_handle(&task.members, kind);
    task.members.push(Member {
        handle: handle.clone(),
        kind: kind.into(),
        model: String::new(),
        effort: String::new(),
        mode: "accept_edits".into(),
        upstream,
        seen: task.posts.len(),
        status: "idle".into(),
        error: None,
        usage: Usage::default(),
        worktree: None,
        fork_next: false,
    });
    handle
}

pub fn store(task: Task) -> Result<(), String> {
    save(&task)?;
    TASKS.lock().map_err(|e| e.to_string())?.insert(task.id.clone(), Arc::new(Mutex::new(task)));
    Ok(())
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    /// Relative to the task's artifacts folder, with forward slashes.
    pub path: String,
    pub size: u64,
    pub modified: u64,
    /// Earlier copies, newest first, as paths `read_team_artifact` accepts.
    pub versions: Vec<String>,
    /// From front matter: spec, ticket, story or review.
    pub kind: Option<String>,
    /// Tickets and stories: todo, in_progress or done.
    pub status: Option<String>,
    pub title: Option<String>,
}

const HISTORY: &str = ".history";

/// A file inside the task's artifacts folder; `path` may not leave it.
pub(crate) fn artifact_path(id: &str, path: &str) -> Result<PathBuf, String> {
    let relative = Path::new(path);
    if path.trim().is_empty() || relative.is_absolute() || relative.components().any(|part| !matches!(part, std::path::Component::Normal(_))) {
        return Err("Artifacts live inside the task's artifacts folder".into());
    }
    Ok(folder(id)?.join("artifacts").join(relative))
}

/// Where an artifact's previous copy goes: `.history/specs__plan.md.<ms>`.
fn history_prefix(path: &str) -> String {
    format!("{}.", path.replace(['/', '\\'], "__"))
}

pub(crate) fn artifacts(id: &str) -> Result<Vec<Artifact>, String> {
    let root = folder(id)?.join("artifacts");
    let history: Vec<String> = fs::read_dir(root.join(HISTORY))
        .map(|entries| entries.flatten().map(|entry| entry.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    let mut found = Vec::new();
    let mut pending = vec![root.clone()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                if entry.file_name() != HISTORY {
                    pending.push(path);
                }
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let relative = path.strip_prefix(&root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
            let prefix = history_prefix(&relative);
            let mut versions: Vec<String> = history.iter().filter(|name| name.starts_with(&prefix)).map(|name| format!("{HISTORY}/{name}")).collect();
            versions.sort_by(|a, b| b.cmp(a));
            let modified = meta.modified().ok().and_then(|time| time.duration_since(UNIX_EPOCH).ok()).map_or(0, |time| time.as_millis() as u64);
            let head = if meta.len() < 2_000_000 { fs::read_to_string(&path).unwrap_or_default() } else { String::new() };
            let (kind, status, title) = front_fields(&head);
            found.push(Artifact { path: relative, size: meta.len(), modified, versions, kind, status, title });
        }
    }
    found.sort_by(|a, b| b.modified.cmp(&a.modified));
    Ok(found)
}

pub(crate) fn save_version(id: &str, path: &str, previous: &[u8]) -> Result<(), String> {
    let history = folder(id)?.join("artifacts").join(HISTORY);
    fs::create_dir_all(&history).map_err(|e| e.to_string())?;
    fs::write(history.join(format!("{}{}", history_prefix(&path.replace('\\', "/")), now())), previous).map_err(|e| e.to_string())
}

/// Every artifact's bytes before a turn, so whatever an agent rewrites keeps its earlier version.
fn artifact_contents(id: &str) -> HashMap<String, Vec<u8>> {
    artifacts(id)
        .unwrap_or_default()
        .into_iter()
        .filter(|artifact| artifact.size < 2_000_000)
        .filter_map(|artifact| Some((artifact.path.clone(), fs::read(artifact_path(id, &artifact.path).ok()?).ok()?)))
        .collect()
}

fn keep_versions(id: &str, before: HashMap<String, Vec<u8>>) {
    for (path, previous) in before {
        let changed = artifact_path(id, &path).ok().and_then(|file| fs::read(file).ok()).is_none_or(|now| now != previous);
        if changed {
            let _ = save_version(id, &path, &previous);
        }
    }
}

/// A member worktree a task deletion would leave behind.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeInfo {
    pub handle: String,
    pub path: String,
    pub branch: String,
    /// Uncommitted files.
    pub dirty: usize,
    /// Commits on the branch that its base does not have.
    pub ahead: usize,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Sweep {
    pub removed: Vec<String>,
    pub kept: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub task_id: String,
    pub task_title: String,
    pub post_id: String,
    pub author: String,
    pub snippet: String,
    pub at: u64,
}

/// About 140 characters of `text` around the first match of `needle` (already lowercase).
fn snippet(text: &str, needle: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let at = lower.find(needle)?;
    // Lowercasing can change byte lengths; count characters to cut the original text safely.
    let before = lower[..at].chars().count();
    let start = before.saturating_sub(40);
    let piece: String = text.chars().skip(start).take(140).collect();
    Some(format!("{}{}", if start > 0 { "…" } else { "" }, piece.replace('\n', " ")))
}

/// Sets one key of a Markdown file's front matter, adding the front matter when there is none.
pub(crate) fn with_front(text: &str, key: &str, value: &str) -> String {
    let line = format!("{key}: {value}");
    if let Some(rest) = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n")) {
        if let Some(end) = rest.find("\n---") {
            let front: Vec<String> = rest[..end].lines().filter(|l| !l.starts_with(&format!("{key}:"))).map(String::from).chain([line]).collect();
            return format!("---\n{}{}", front.join("\n"), &rest[end..]);
        }
    }
    format!("---\n{line}\n---\n\n{text}")
}

/// `kind`, `status` and `title` from an artifact's front matter.
pub(crate) fn front_fields(text: &str) -> (Option<String>, Option<String>, Option<String>) {
    let Some(rest) = text.strip_prefix("---\n").or_else(|| text.strip_prefix("---\r\n")) else { return (None, None, None) };
    let Some(end) = rest.find("\n---") else { return (None, None, None) };
    let front = &rest[..end];
    let get = |key: &str| crate::skills::front_value(front, key).map(|value| value.to_lowercase().replace([' ', '-'], "_"));
    let title = crate::skills::front_value(front, "title");
    (get("kind").or_else(|| get("type")), get("status"), title)
}

/// One side-chat turn: the member's session is copied, it may only read, and nothing it learns
/// is kept on the member.
async fn side_turn(app: &AppHandle, task_id: &str, handle: &str, question: &str) -> Result<Reply, String> {
    let shared = load(task_id)?;
    let dir = folder(task_id)?;
    let (member, prompt, root) = {
        let task = lock(&shared)?;
        let member = task.members.iter().find(|m| m.handle == handle).cloned().ok_or("That member left the task")?;
        let prompt = format!(
            "{}\n\n## Side question from the user\n\nAnswer briefly. This is a side chat: change no files and do not hand off; the main thread does not see it.\n\n{question}",
            preamble(&task, &member, &dir)
        );
        let root = match (&member.worktree, task.project_path.is_empty()) {
            (Some(tree), _) => PathBuf::from(&tree.path),
            (None, true) => dir.clone(),
            (None, false) => PathBuf::from(&task.project_path),
        };
        (member, prompt, root)
    };
    if Kind::parse(&member.kind).is_none() {
        return Err("Side chats work with coding-agent CLIs".into());
    }
    let cancel = Arc::new(Notify::new());
    run_cli(app, task_id, &shared, &member, prompt, &root, &dir, cancel, true).await
}

pub mod commands {
    use super::*;

    /// The task's shared documents: specs, plans, tickets, reviews.
    #[tauri::command]
    pub fn list_team_artifacts(id: String) -> Result<Vec<Artifact>, String> {
        artifacts(&id)
    }

    #[tauri::command]
    pub fn read_team_artifact(id: String, path: String) -> Result<String, String> {
        let file = artifact_path(&id, &path)?;
        if fs::metadata(&file).map_err(|_| "That artifact is gone")?.len() > 2_000_000 {
            return Err("That artifact is too large to open here".into());
        }
        fs::read_to_string(file).map_err(|e| e.to_string())
    }

    /// Saves an artifact, keeping the previous copy in `.history` so it can be restored.
    #[tauri::command]
    pub fn write_team_artifact(id: String, path: String, content: String) -> Result<Vec<Artifact>, String> {
        if path.starts_with(HISTORY) {
            return Err("Earlier copies cannot be edited; restore one instead".into());
        }
        let file = artifact_path(&id, &path)?;
        if let Ok(previous) = fs::read(&file) {
            save_version(&id, &path, &previous)?;
        }
        fs::create_dir_all(file.parent().ok_or("Invalid artifact path")?).map_err(|e| e.to_string())?;
        fs::write(&file, content).map_err(|e| e.to_string())?;
        artifacts(&id)
    }

    /// The agent CLIs on this machine; cached because each `--version` takes a moment.
    #[tauri::command]
    pub async fn list_team_agents(app: AppHandle, refresh: Option<bool>) -> Result<Vec<team_agents::Agent>, String> {
        if refresh != Some(true) {
            if let Some(agents) = AGENTS.lock().map_err(|e| e.to_string())?.clone() {
                return Ok(agents);
            }
        }
        let mut agents = tokio::task::spawn_blocking(team_agents::detect).await.map_err(|e| e.to_string())?;
        if gemini_key(&app).is_some() {
            for agent in agents.iter_mut().filter(|agent| agent.kind == "gemini") {
                agent.signed_in = true;
            }
        }
        *AGENTS.lock().map_err(|e| e.to_string())? = Some(agents.clone());
        Ok(agents)
    }

    #[tauri::command]
    pub fn list_team_tasks() -> Result<Vec<TaskSummary>, String> {
        all_tasks()
    }

    #[tauri::command]
    pub fn create_team_task(title: String, project_path: String, members: Vec<NewMember>, new_project: Option<bool>) -> Result<TaskView, String> {
        let path = project_path.trim();
        if !path.is_empty() {
            let root = PathBuf::from(path);
            if new_project == Some(true) {
                start_project(&root)?;
            } else if !root.is_dir() {
                return Err(format!("{path} is not a folder on this computer"));
            }
            let _ = crate::workspace::remember_project(&root);
        }
        view(&create(&title, path, members, Vec::new())?)
    }

    /// Copies files into the task folder, which every member can read, and returns their paths
    /// there for the message. `data` holds pasted images as (name, base64).
    #[tauri::command]
    pub fn team_attach(id: String, paths: Vec<String>, data: Option<Vec<(String, String)>>) -> Result<Vec<String>, String> {
        let dir = folder(&id)?.join("attachments");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mut saved = Vec::new();
        let place = |name: &str| {
            let name: String = name.chars().map(|c| if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '-' }).collect();
            let name = if name.trim_matches(['-', '.']).is_empty() { "file".to_string() } else { name };
            // A file of the same name from an earlier message stays as it was.
            let (stem, ext) = name.rsplit_once('.').map_or((name.clone(), String::new()), |(stem, ext)| (stem.to_string(), format!(".{ext}")));
            (0..).map(|n| dir.join(if n == 0 { name.clone() } else { format!("{stem}-{n}{ext}") })).find(|path| !path.exists()).unwrap_or_else(|| dir.join(&name))
        };
        for path in &paths {
            let source = PathBuf::from(path);
            if !source.is_file() {
                return Err(format!("{path} is not a file"));
            }
            let target = place(&source.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default());
            std::fs::copy(&source, &target).map_err(|e| format!("Could not attach {path}: {e}"))?;
            saved.push(target.to_string_lossy().trim_start_matches(r"\\?\").to_string());
        }
        for (name, encoded) in data.unwrap_or_default() {
            let bytes = crate::documents::unbase64(encoded.split_once(',').map_or(encoded.as_str(), |(_, body)| body)).map_err(|e| format!("Could not read {name}: {e}"))?;
            let target = place(&name);
            std::fs::write(&target, bytes).map_err(|e| e.to_string())?;
            saved.push(target.to_string_lossy().trim_start_matches(r"\\?\").to_string());
        }
        Ok(saved)
    }

    #[tauri::command]
    pub fn team_snapshot(id: String) -> Result<TaskView, String> {
        let shared = load(&id)?;
        let task = lock(&shared)?;
        view(&task)
    }

    #[tauri::command]
    pub fn add_team_member(id: String, member: NewMember, app: AppHandle) -> Result<TaskView, String> {
        let shared = load(&id)?;
        let mut task = lock(&shared)?;
        let mut member = new_member(&task.members, member)?;
        // A newcomer starts from the whole thread.
        member.seen = 0;
        let handle = member.handle.clone();
        task.members.push(member);
        task.updated_at = now();
        save(&task)?;
        emit_member(&app, &task, &handle);
        view(&task)
    }

    /// A generated image of a team post, as a data URI for the thread.
    #[tauri::command]
    pub fn team_image(id: String, name: String) -> Result<String, String> {
        let (mime, data) = image_data(&id, &name)?;
        Ok(format!("data:{mime};base64,{data}"))
    }

    #[tauri::command]
    pub fn update_team_member(id: String, handle: String, model: Option<String>, mode: Option<String>, effort: Option<String>, app: AppHandle) -> Result<TaskView, String> {
        let shared = load(&id)?;
        let mut task = lock(&shared)?;
        let member = task.members.iter_mut().find(|m| m.handle == handle).ok_or("No such member")?;
        if let Some(model) = model {
            member.model = model;
        }
        if let Some(effort) = effort {
            member.effort = effort;
        }
        if let Some(mode) = mode {
            member.mode = mode;
        }
        save(&task)?;
        // Every open view (the desktop's other windows, a paired phone) shows the change right away.
        emit_member(&app, &task, &handle);
        view(&task)
    }

    /// Gives a member its own Git worktree (branch neru/<id>) or takes it back. The repository's
    /// setup script (`.neru/environment.json` or `.traycer/environment.json`) runs in a new
    /// worktree and its teardown script before one is removed. Removing keeps a worktree that has
    /// uncommitted work, as Git does.
    #[tauri::command]
    pub async fn set_team_worktree(id: String, handle: String, enabled: bool, app: AppHandle) -> Result<TaskView, String> {
        if is_running(&id, &handle) {
            return Err(format!("@{handle} is working; stop it first"));
        }
        let shared = load(&id)?;
        let (root, current) = {
            let task = lock(&shared)?;
            let member = task.members.iter().find(|m| m.handle == handle).ok_or("No such member")?;
            (PathBuf::from(&task.project_path), member.worktree.clone())
        };
        let note: Option<String> = None;
        let tree = match (enabled, current) {
            (true, None) => {
                if root.as_os_str().is_empty() {
                    return Err("Worktrees need the task to have a project folder".into());
                }
                let tree = sessions::create_worktree(&root, &Uuid::new_v4().to_string())?;
                run_setup(&app, &id, &handle, &root, &tree).await;
                Some(tree)
            }
            (true, Some(tree)) => Some(tree),
            (false, Some(tree)) => {
                if let Some(teardown) = team_tools::script(&root, "teardown") {
                    let _ = team_tools::run_script(&teardown, Path::new(&tree.path), Duration::from_secs(60)).await;
                }
                if let Err(error) = crate::git::git(&root, &["worktree", "remove", &tree.path]) {
                    return Err(format!("Kept the worktree: {error}"));
                }
                None
            }
            (false, None) => None,
        };
        let mut task = lock(&shared)?;
        if let Some(member) = task.members.iter_mut().find(|m| m.handle == handle) {
            member.worktree = tree;
        }
        if let Some(text) = note {
            let post = Post::notice(text);
            task.posts.push(post.clone());
            emit(&app, &id, "post", json!({ "post": post }));
        }
        save(&task)?;
        view(&task)
    }

    /// Removes the members' worktrees whose work has landed in the base branch (or never left it)
    /// and that have nothing uncommitted. Branches are kept.
    #[tauri::command]
    pub async fn sweep_team_worktrees(id: String) -> Result<Sweep, String> {
        let shared = load(&id)?;
        let (root, trees) = {
            let task = lock(&shared)?;
            let trees: Vec<(String, sessions::Worktree)> = task.members.iter().filter_map(|m| Some((m.handle.clone(), m.worktree.clone()?))).collect();
            (PathBuf::from(&task.project_path), trees)
        };
        let mut sweep = Sweep::default();
        for (handle, tree) in trees {
            if is_running(&id, &handle) {
                sweep.kept.push(format!("@{handle}: working"));
                continue;
            }
            match team_tools::sweep_state(&root, &tree) {
                Ok(_) => {
                    if let Some(teardown) = team_tools::script(&root, "teardown") {
                        let _ = team_tools::run_script(&teardown, Path::new(&tree.path), Duration::from_secs(60)).await;
                    }
                    let _ = crate::git::git(&root, &["worktree", "remove", &tree.path]);
                    if let Ok(mut task) = lock(&shared) {
                        if let Some(member) = task.members.iter_mut().find(|m| m.handle == handle) {
                            member.worktree = None;
                        }
                    }
                    sweep.removed.push(format!("@{handle} ({})", tree.branch));
                }
                Err(reason) => sweep.kept.push(format!("@{handle}: {reason}")),
            }
        }
        {
            let task = lock(&shared)?;
            save(&task)?;
        }
        Ok(sweep)
    }

    /// Undoes the files one turn changed: they go back to how they were before it.
    #[tauri::command]
    pub fn undo_team_turn(id: String, post_id: String) -> Result<TaskView, String> {
        let shared = load(&id)?;
        let mut task = lock(&shared)?;
        if task.members.iter().any(|m| is_running(&id, &m.handle)) {
            return Err("Wait for the team to finish, then undo".into());
        }
        let post = task.posts.iter_mut().find(|post| post.id == post_id).ok_or("That message is gone")?;
        let changes = post.changes.as_mut().ok_or("That turn changed no files")?;
        if changes.undone {
            return Err("That turn is already undone".into());
        }
        let count = team_tools::undo(changes)?;
        changes.undone = true;
        let author = post.author.clone();
        let files = if count == 1 { "file" } else { "files" };
        task.posts.push(Post::notice(format!("Undid the turn by @{author}: {count} {files} restored.")));
        save(&task)?;
        view(&task)
    }

    /// Adds a copy of a member: same agent, model and access, continuing a fork of its session.
    #[tauri::command]
    pub fn fork_team_member(id: String, handle: String) -> Result<TaskView, String> {
        let shared = load(&id)?;
        let mut task = lock(&shared)?;
        let original = task.members.iter().find(|m| m.handle == handle).cloned().ok_or("No such member")?;
        let fork = Member {
            handle: new_handle(&task.members, original.kind.trim_start_matches("custom:")),
            fork_next: original.upstream.is_some(),
            status: "idle".into(),
            error: None,
            usage: Usage::default(),
            worktree: None,
            ..original
        };
        task.posts.push(Post::notice(format!("@{} is a fork of @{handle}.", fork.handle)));
        task.members.push(fork);
        save(&task)?;
        view(&task)
    }

    /// A side question for one member: it answers in a copy of its session, read-only, and the
    /// main thread does not move on.
    #[tauri::command]
    pub fn ask_team_side(id: String, handle: String, text: String, app: AppHandle) -> Result<Post, String> {
        let shared = load(&id)?;
        let question = {
            let mut task = lock(&shared)?;
            if !task.members.iter().any(|m| m.handle == handle) {
                return Err(format!("No member @{handle}"));
            }
            let post = Post { kind: "side".into(), to: vec![handle.clone()], ..Post::notice(text.trim().to_string()) };
            task.posts.push(post.clone());
            save(&task)?;
            post
        };
        emit(&app, &id, "post", json!({ "post": question }));
        let question_text = question.text.clone();
        tauri::async_runtime::spawn(async move {
            let post = match side_turn(&app, &id, &handle, &question_text).await {
                Ok(reply) => Post { kind: "side".into(), steps: reply.steps, ..Post::imported(&handle, reply.text) },
                Err(error) => Post { kind: "error".into(), ..Post::imported(&handle, error) },
            };
            if let Ok(shared) = load(&id) {
                if let Ok(mut task) = lock(&shared) {
                    task.posts.push(post.clone());
                    let _ = save(&task);
                }
            }
            emit(&app, &id, "post", json!({ "post": post }));
        });
        Ok(question)
    }

    /// Opens the member's own CLI in a terminal window, continuing the same session.
    #[tauri::command]
    pub fn team_open_terminal(app: AppHandle, id: String, handle: String) -> Result<(), String> {
        let shared = load(&id)?;
        let task = lock(&shared)?;
        let member = task.members.iter().find(|m| m.handle == handle).ok_or("No such member")?;
        let kind = Kind::parse(&member.kind).ok_or("Only coding-agent CLIs open in a terminal")?;
        let program = team_agents::program(kind).ok_or_else(|| format!("{} is not installed", kind.name()))?;
        let root = match (&member.worktree, task.project_path.is_empty()) {
            (Some(tree), _) => PathBuf::from(&tree.path),
            (None, true) => folder(&id)?,
            (None, false) => PathBuf::from(&task.project_path),
        };
        let args: Vec<String> = match (kind, member.upstream.as_deref()) {
            (Kind::Claude | Kind::Cursor, Some(session)) => vec!["--resume".into(), session.into()],
            (Kind::Codex, Some(session)) => vec!["resume".into(), session.into()],
            (Kind::OpenCode, Some(session)) => vec!["--session".into(), session.into()],
            (Kind::Gemini, Some(session)) => vec!["--resume".into(), session.into()],
            _ => Vec::new(),
        };
        let env = if kind == Kind::Gemini { gemini_env(&app, None) } else { Vec::new() };
        team_tools::open_terminal(&program, &args, &root, &env)
    }

    #[tauri::command]
    pub fn set_team_pinned(id: String, pinned: bool) -> Result<TaskView, String> {
        let shared = load(&id)?;
        let mut task = lock(&shared)?;
        task.pinned = pinned;
        save(&task)?;
        view(&task)
    }

    #[tauri::command]
    pub fn set_team_labels(id: String, labels: Vec<String>) -> Result<TaskView, String> {
        let shared = load(&id)?;
        let mut task = lock(&shared)?;
        let mut clean: Vec<String> = labels.into_iter().map(|label| label.trim().chars().take(24).collect::<String>()).filter(|label| !label.is_empty()).collect();
        clean.dedup();
        task.labels = clean;
        save(&task)?;
        view(&task)
    }

    /// Messages in every task that contain `query` (case-insensitive), newest first.
    #[tauri::command]
    pub fn search_team(query: String) -> Result<Vec<SearchHit>, String> {
        let needle = query.trim().to_lowercase();
        if needle.chars().count() < 2 {
            return Ok(Vec::new());
        }
        let mut hits = Vec::new();
        for summary in all_tasks()? {
            let shared = load(&summary.id)?;
            let task = lock(&shared)?;
            for post in task.posts.iter().filter(|post| post.kind != "notice") {
                if let Some(snippet) = snippet(&post.text, &needle) {
                    hits.push(SearchHit { task_id: task.id.clone(), task_title: task.title.clone(), post_id: post.id.clone(), author: post.author.clone(), snippet, at: post.at });
                }
            }
        }
        hits.sort_by(|a, b| b.at.cmp(&a.at));
        hits.truncate(60);
        Ok(hits)
    }

    /// Saves (or with an empty key, forgets) the Gemini API key Gemini CLI members use. It is the
    /// Google Gemini provider's key, so the model picker can use it too.
    #[tauri::command]
    pub fn set_gemini_key(app: AppHandle, key: String) -> Result<(), String> {
        let state = app.state::<AppState>();
        {
            let mut keys = state.provider_keys.lock().map_err(|e| e.to_string())?;
            keys.retain(|id, _| !id.starts_with("gemini\n"));
            if !key.trim().is_empty() {
                keys.insert(format!("gemini\n{GEMINI_URL}"), key.trim().to_string());
            }
        }
        crate::settings::save(&state)?;
        *AGENTS.lock().map_err(|e| e.to_string())? = None;
        Ok(())
    }

    /// Your own CLI agents from `cli-agents` folders.
    #[tauri::command]
    pub fn list_custom_agents(app: AppHandle) -> Vec<team_tools::CustomAgent> {
        let root = app.state::<AppState>().root.lock().ok().and_then(|root| root.clone());
        team_tools::custom_agents(root.as_deref())
    }

    /// Sets a ticket or story's status in its front matter: todo, in_progress or done.
    #[tauri::command]
    pub fn set_team_artifact_status(id: String, path: String, status: String) -> Result<Vec<Artifact>, String> {
        if !matches!(status.as_str(), "todo" | "in_progress" | "done") {
            return Err("Status is todo, in_progress or done".into());
        }
        let file = artifact_path(&id, &path)?;
        let text = fs::read_to_string(&file).map_err(|e| e.to_string())?;
        save_version(&id, &path, text.as_bytes())?;
        fs::write(&file, with_front(&text, "status", &status)).map_err(|e| e.to_string())?;
        artifacts(&id)
    }

    /// Copies artifacts as Markdown files into a folder you pick.
    #[tauri::command]
    pub fn export_team_artifacts(id: String, paths: Vec<String>, destination: String) -> Result<usize, String> {
        let destination = PathBuf::from(destination);
        fs::create_dir_all(&destination).map_err(|e| e.to_string())?;
        for path in &paths {
            let source = artifact_path(&id, path)?;
            let target = destination.join(path.replace(['/', '\\'], "__"));
            fs::copy(&source, &target).map_err(|e| format!("{path}: {e}"))?;
        }
        Ok(paths.len())
    }

    #[tauri::command]
    pub fn remove_team_member(id: String, handle: String) -> Result<TaskView, String> {
        if is_running(&id, &handle) {
            return Err(format!("@{handle} is working; stop it first"));
        }
        let shared = load(&id)?;
        let mut task = lock(&shared)?;
        task.members.retain(|m| m.handle != handle);
        save(&task)?;
        view(&task)
    }

    #[tauri::command]
    pub fn rename_team_task(id: String, title: String) -> Result<TaskView, String> {
        let shared = load(&id)?;
        let mut task = lock(&shared)?;
        if !title.trim().is_empty() {
            task.title = title.trim().into();
        }
        save(&task)?;
        view(&task)
    }

    #[tauri::command]
    pub fn set_team_routing(id: String, enabled: bool) -> Result<TaskView, String> {
        let shared = load(&id)?;
        let mut task = lock(&shared)?;
        task.route_on_limit = enabled;
        save(&task)?;
        view(&task)
    }

    /// What deleting a task would leave behind: each member worktree, its uncommitted files and
    /// the commits its branch has that the base branch does not.
    #[tauri::command]
    pub fn team_cleanup_info(id: String) -> Result<Vec<TreeInfo>, String> {
        let shared = load(&id)?;
        let task = lock(&shared)?;
        let root = PathBuf::from(&task.project_path);
        Ok(task
            .members
            .iter()
            .filter_map(|member| {
                let tree = member.worktree.as_ref()?;
                let (dirty, ahead) = team_tools::worktree_info(&root, tree);
                Some(TreeInfo { handle: member.handle.clone(), path: tree.path.clone(), branch: tree.branch.clone(), dirty, ahead })
            })
            .collect())
    }

    /// Deletes the task. Its members' worktrees are removed when `remove_worktrees` (with their
    /// uncommitted files only when `force`), and their branches too when `delete_branches`.
    #[tauri::command]
    pub async fn delete_team_task(id: String, remove_worktrees: Option<bool>, delete_branches: Option<bool>, force: Option<bool>) -> Result<(), String> {
        let shared = load(&id)?;
        let (root, trees) = {
            let task = lock(&shared)?;
            if task.members.iter().any(|m| is_running(&id, &m.handle)) {
                return Err("Stop the task's members first".into());
            }
            (PathBuf::from(&task.project_path), task.members.iter().filter_map(|m| m.worktree.clone()).collect::<Vec<_>>())
        };
        if remove_worktrees == Some(true) {
            for tree in &trees {
                if let Some(teardown) = team_tools::script(&root, "teardown") {
                    let _ = team_tools::run_script(&teardown, Path::new(&tree.path), Duration::from_secs(60)).await;
                }
                let mut args = vec!["worktree", "remove", tree.path.as_str()];
                if force == Some(true) {
                    args.push("--force");
                }
                crate::git::git(&root, &args).map_err(|error| format!("Kept {}: {error}", tree.branch))?;
                if delete_branches == Some(true) {
                    crate::git::git(&root, &["branch", if force == Some(true) { "-D" } else { "-d" }, &tree.branch]).map_err(|error| format!("Kept branch {}: {error}", tree.branch))?;
                }
            }
        }
        TASKS.lock().map_err(|e| e.to_string())?.remove(&id);
        fs::remove_dir_all(folder(&id)?).map_err(|e| e.to_string())
    }

    /// The task list's group, icon and colour for a task.
    #[tauri::command]
    pub fn set_team_appearance(id: String, group: Option<String>, icon: Option<String>, color: Option<String>) -> Result<TaskView, String> {
        let shared = load(&id)?;
        let mut task = lock(&shared)?;
        if let Some(group) = group {
            task.group = group.trim().chars().take(40).collect();
        }
        if let Some(icon) = icon {
            task.icon = icon.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(40).collect();
        }
        if let Some(color) = color {
            task.color = color.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(24).collect();
        }
        save(&task)?;
        view(&task)
    }

    /// Edits, removes or moves a queued message, pauses or resumes the queue, or sends an item now.
    #[tauri::command]
    pub fn update_team_queue(id: String, item: Option<String>, text: Option<String>, remove: Option<bool>, delta: Option<i32>, paused: Option<bool>, send_now: Option<bool>, app: AppHandle) -> Result<Vec<Queued>, String> {
        let shared = load(&id)?;
        let mut send = None;
        let queue = {
            let mut task = lock(&shared)?;
            if let Some(paused) = paused {
                task.queue_paused = paused;
            }
            if let Some(item) = item {
                let index = task.queue.iter().position(|queued| queued.id == item).ok_or("That message already went out")?;
                if let Some(text) = text.filter(|text| !text.trim().is_empty()) {
                    task.queue[index].text = text.trim().into();
                }
                if remove == Some(true) {
                    task.queue.remove(index);
                } else if send_now == Some(true) {
                    send = Some(task.queue.remove(index));
                } else if let Some(delta) = delta {
                    let target = (index as i32 + delta).clamp(0, task.queue.len() as i32 - 1) as usize;
                    let moved = task.queue.remove(index);
                    task.queue.insert(target, moved);
                }
            }
            save(&task)?;
            emit(&app, &id, "queue", json!({ "queue": task.queue, "paused": task.queue_paused }));
            task.queue.clone()
        };
        if let Some(item) = send {
            // Sent now: posted at once, and each member takes it when its current turn ends.
            let post = Post { to: item.to.clone(), at: now(), kind: message_kind(), ..Post::notice(item.text) };
            {
                let mut task = lock(&shared)?;
                task.posts.push(post.clone());
                task.updated_at = now();
                save(&task)?;
            }
            emit(&app, &id, "post", json!({ "post": post }));
            tauri::async_runtime::spawn(route(app.clone(), id.clone(), item.to));
        } else if paused == Some(false) {
            drain(&app, &id);
        }
        Ok(queue)
    }

    /// Runs the repository's setup script again in a member's worktree.
    #[tauri::command]
    pub async fn rerun_team_setup(id: String, handle: String, app: AppHandle) -> Result<(), String> {
        let shared = load(&id)?;
        let (root, tree) = {
            let task = lock(&shared)?;
            let member = task.members.iter().find(|m| m.handle == handle).ok_or("No such member")?;
            (PathBuf::from(&task.project_path), member.worktree.clone().ok_or("That member has no worktree")?)
        };
        if team_tools::script(&root, "setup").is_none() {
            return Err("This repository has no setup script in .neru/environment.json or .traycer/environment.json".into());
        }
        run_setup(&app, &id, &handle, &root, &tree).await;
        Ok(())
    }

    /// Posts the user's message and starts the turns it asks for: `to`, else the handles it
    /// mentions, else whoever spoke last.
    #[tauri::command]
    pub fn send_team_message(id: String, text: String, to: Vec<String>, app: AppHandle) -> Result<Post, String> {
        if text.trim().is_empty() {
            return Err("Write a message first".into());
        }
        let shared = load(&id)?;
        let (post, targets) = {
            let mut task = lock(&shared)?;
            if task.members.is_empty() {
                return Err("Add an agent to the task first".into());
            }
            let handles: Vec<String> = task.members.iter().map(|m| m.handle.clone()).collect();
            let mut targets: Vec<String> = to.into_iter().filter(|h| h == "all" || handles.contains(h)).collect();
            if targets.iter().any(|h| h == "all") {
                targets = handles.clone();
            }
            if targets.is_empty() {
                targets = mentions(&text, &handles, "you", true).0;
            }
            if targets.is_empty() {
                let last = task.posts.iter().rev().find(|p| handles.contains(&p.author) && p.kind == "message").map(|p| p.author.clone());
                targets = vec![last.unwrap_or_else(|| handles[0].clone())];
            }
            // Busy members get it when they are free, in order; the queue can be edited meanwhile.
            if targets.iter().any(|handle| is_running(&id, handle)) || (task.queue_paused && !task.queue.is_empty()) {
                let item = Queued { id: Uuid::new_v4().to_string(), text: text.trim().into(), to: targets.clone(), at: now() };
                task.queue.push(item.clone());
                save(&task)?;
                emit(&app, &id, "queue", json!({ "queue": task.queue, "paused": task.queue_paused }));
                return Ok(Post { id: item.id, to: item.to, kind: "queued".into(), ..Post::notice(item.text) });
            }
            let post = Post { changes: None, status: None, images: vec![], id: Uuid::new_v4().to_string(), author: "you".into(), to: targets.clone(), text: text.trim().into(), steps: vec![], at: now(), kind: message_kind() };
            task.posts.push(post.clone());
            task.updated_at = now();
            save(&task)?;
            (post, targets)
        };
        emit(&app, &id, "post", json!({ "post": post }));
        tauri::async_runtime::spawn(route(app, id, targets));
        Ok(post)
    }

    /// Stops one member, or every member of the task.
    #[tauri::command]
    pub fn stop_team(id: String, handle: Option<String>) -> Result<(), String> {
        if handle.is_none() {
            STOPPED.lock().map_err(|e| e.to_string())?.insert(id.clone(), now());
        }
        let running = RUNNING.lock().map_err(|e| e.to_string())?;
        for (key, cancel) in running.iter() {
            let (task, member) = key.split_once('\n').unwrap_or_default();
            if task == id && handle.as_ref().is_none_or(|h| h == member) {
                cancel.notify_waiters();
                cancel.notify_one();
            }
        }
        if let Some(cancel) = ROUTING.lock().map_err(|e| e.to_string())?.get(&id) {
            cancel.notify_one();
        }
        Ok(())
    }

    /// Opens a terminal window running the agent's own sign-in; Neru never sees the credentials.
    #[tauri::command]
    pub fn team_agent_login(kind: String) -> Result<(), String> {
        let kind = Kind::parse(&kind).ok_or("Unknown agent")?;
        let program = team_agents::program(kind).ok_or_else(|| format!("Install {} first: {}", kind.name(), kind.install()))?;
        let args = kind.login().split_whitespace().skip(1).collect::<Vec<_>>().join(" ");
        let line = format!("\"{}\" {args}", program.display());
        #[cfg(windows)]
        let result = {
            use std::os::windows::process::CommandExt;
            // cmd parses its own quotes, so the line goes through untouched.
            std::process::Command::new("cmd").raw_arg(format!("/C start \"\" cmd /K {line}")).spawn()
        };
        #[cfg(target_os = "macos")]
        let result = std::process::Command::new("osascript")
            .args(["-e", &format!("tell application \"Terminal\" to do script \"{}\"", line.replace('"', "\\\""))])
            .spawn();
        #[cfg(all(unix, not(target_os = "macos")))]
        let result = std::process::Command::new("x-terminal-emulator").args(["-e", "sh", "-c", &line]).spawn();
        result.map(|_| ()).map_err(|e| format!("Could not open a terminal: {e}. Run {} yourself.", kind.login()))
    }

    /// Keeps a limited member's request where it is instead of handing it on.
    #[tauri::command]
    pub fn cancel_team_routing(id: String) -> Result<(), String> {
        if let Some(cancel) = ROUTING.lock().map_err(|e| e.to_string())?.get(&id) {
            cancel.notify_one();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post(author: &str, text: &str) -> Post {
        Post { changes: None, status: None, images: vec![], id: Uuid::new_v4().to_string(), author: author.into(), to: vec![], text: text.into(), steps: vec![], at: 0, kind: message_kind() }
    }

    #[test]
    fn agents_hand_off_only_at_line_start_and_never_inside_code() {
        let handles = vec!["claude".to_string(), "codex".to_string(), "gemini".to_string()];
        let reply = "I fixed it, as @codex suggested.\n```\n@gemini not this\n```\n@codex please review src/x.rs\n- @gemini (fyi) the API changed";
        let (next, fyi) = mentions(reply, &handles, "claude", false);
        assert_eq!(next, vec!["codex"]);
        assert_eq!(fyi, vec!["gemini"]);
        // Agents cannot hand a turn to themselves.
        assert!(mentions("@claude again", &handles, "claude", false).0.is_empty());
    }

    #[test]
    fn people_mention_anywhere_but_emails_do_not_count() {
        let handles = vec!["claude".to_string(), "codex".to_string()];
        assert_eq!(mentions("what do @Codex and @claude think? mail me at a@codex", &handles, "you", true).0, vec!["codex", "claude"]);
        assert_eq!(mentions("@all review this", &handles, "you", true).0, vec!["claude", "codex"]);
        assert!(mentions("no one", &handles, "you", true).0.is_empty());
        // Leading mentions pick who answers; later ones are only part of the request.
        assert_eq!(mentions("@claude check it, then hand to @codex", &handles, "you", true).0, vec!["claude"]);
        assert_eq!(mentions("@Codex, @claude compare", &handles, "you", true).0, vec!["codex", "claude"]);
    }

    #[test]
    fn digest_skips_own_posts_and_keeps_the_newest_when_long() {
        let posts = vec![post("you", "start"), post("claude", "mine"), post("codex", "codex said this")];
        let text = digest(&posts, 0, "claude", false);
        assert!(text.contains("The user") && text.contains("@codex") && !text.contains("mine"));
        assert!(!digest(&posts, 2, "claude", false).contains("start"));
        assert!(digest(&posts, 0, "claude", true).contains("mine"));
        let long: Vec<Post> = (0..40).map(|n| post("codex", &format!("{n} {}", "x".repeat(1000)))).collect();
        let text = digest(&long, 0, "claude", false);
        assert!(text.starts_with("(") && text.contains("39 x") && !text.contains("\n0 x"));
        assert!(text.len() < DIGEST_CHARS + 2_000);
    }

    #[test]
    fn handles_are_unique_and_prompts_name_the_team() {
        let mut members = Vec::new();
        for kind in ["claude", "claude", "codex"] {
            let member = new_member(&members, NewMember { kind: kind.into(), model: String::new(), mode: String::new(), effort: String::new() }).unwrap();
            members.push(member);
        }
        let handles: Vec<&str> = members.iter().map(|m| m.handle.as_str()).collect();
        assert_eq!(handles, ["claude", "claude-2", "codex"]);
        assert!(new_member(&members, NewMember { kind: "nope".into(), model: String::new(), mode: String::new(), effort: String::new() }).is_err());
        let task = Task {
            id: Uuid::new_v4().to_string(),
            title: "t".into(),
            project_path: "/w".into(),
            created_at: 0,
            updated_at: 0,
            members: members.clone(),
            posts: vec![post("you", "hello team")],
            route_on_limit: true,
            ..Default::default()
        };
        let prompt = prompt_for(&task, &members[2], Path::new("/t"));
        assert!(prompt.contains("You are @codex") && prompt.contains("@claude-2 (Claude Code)") && prompt.contains("hello team"));
        assert!(prompt.contains("/t/thread.md"));
    }

    #[test]
    fn artifacts_stay_in_their_folder_and_keep_earlier_copies() {
        let id = Uuid::new_v4().to_string();
        assert!(artifact_path(&id, "../task.json").is_err());
        assert!(artifact_path(&id, "/etc/passwd").is_err());
        assert!(artifact_path(&id, "specs/plan.md").is_ok());
        if std::env::var_os("NERU_DATA_DIR").is_none() {
            // SAFETY: tests that use the data folder all point it at a temporary directory.
            unsafe { std::env::set_var("NERU_DATA_DIR", std::env::temp_dir().join(format!("neru-team-data-{id}"))) };
        }
        commands::write_team_artifact(id.clone(), "specs/plan.md".into(), "v1".into()).unwrap();
        let list = commands::write_team_artifact(id.clone(), "specs/plan.md".into(), "v2".into()).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].path, "specs/plan.md");
        assert_eq!(list[0].versions.len(), 1);
        assert_eq!(commands::read_team_artifact(id.clone(), list[0].versions[0].clone()).unwrap(), "v1");
        assert_eq!(commands::read_team_artifact(id.clone(), "specs/plan.md".into()).unwrap(), "v2");
        assert!(commands::write_team_artifact(id.clone(), ".history/x".into(), "no".into()).is_err());
        let _ = fs::remove_dir_all(folder(&id).unwrap());
    }

    #[test]
    fn subscription_limits_are_told_apart_from_other_failures() {
        assert!(is_limit("You've hit your usage limit. Upgrade to Pro"));
        assert!(is_limit("Claude AI usage limit reached|1791284400"));
        assert!(!is_limit("file not found"));
        assert!(friendly("claude", "Invalid API key · Please run /login").contains("not signed in"));
    }
}
