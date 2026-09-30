//! Chat sessions. Each session has its own runtime (conversation, transcript, pending approval,
//! applied edits, running flag and cancel handle), so several can run at once while the window
//! shows one of them. Sessions are saved as JSON under the data directory; a session may live in
//! its own Git worktree so parallel work never collides on disk.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;
use uuid::Uuid;

use crate::{
    AppState, PendingAction,
    agent::PendingView,
    git,
    workspace::{data_dir, restore_checkpoint_file},
};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptEntry {
    pub id: String,
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_paths: Vec<String>,
    /// Images sent with a user message, as data URLs, so the conversation shows them again.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<crate::web::Source>,
    /// For user messages: where this message sits in the model conversation, so it can be rewound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_at: Option<usize>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Worktree {
    pub path: String,
    pub branch: String,
    pub base: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub project_path: String,
    pub updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<Worktree>,
    /// Filled in when listing; never meaningful on disk.
    #[serde(default)]
    pub running: bool,
    /// The title was written by the model or the user, so it is not replaced again.
    #[serde(default)]
    pub titled: bool,
}

/// An edit Neru applied, and how long the transcript was at the time, for rewinding code.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditRecord {
    pub checkpoint: String,
    pub transcript_len: usize,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SessionFile {
    summary: SessionSummary,
    #[serde(default)]
    conversation: Vec<Value>,
    #[serde(default)]
    transcript: Vec<TranscriptEntry>,
    pending: Option<PendingAction>,
    /// Tool calls from the same turn as the pending action, run once it is settled.
    #[serde(default)]
    queued: Vec<Value>,
    #[serde(default)]
    todos: Vec<crate::extras::Todo>,
    #[serde(default)]
    edits: Vec<EditRecord>,
}

pub struct Runtime {
    pub summary: SessionSummary,
    pub conversation: Vec<Value>,
    pub transcript: Vec<TranscriptEntry>,
    pub pending: Option<PendingAction>,
    /// Tool calls the model made in the same turn as `pending`; they run after it instead of
    /// being thrown away, so a multi-file build does not regenerate every file.
    pub queued: Vec<Value>,
    /// The agent's visible to-do list for this session.
    pub todos: Vec<crate::extras::Todo>,
    /// Messages the user sent while a reply was running, handed to the agent at its next step.
    pub steer: Vec<String>,
    pub edits: Vec<EditRecord>,
    pub running: bool,
    pub cancel: Arc<tokio::sync::Notify>,
}

pub type Shared = Arc<Mutex<Runtime>>;
pub type SessionMap = Mutex<HashMap<String, Shared>>;

impl Runtime {
    fn from_file(file: SessionFile) -> Self {
        Self {
            summary: file.summary,
            conversation: file.conversation,
            transcript: file.transcript,
            pending: file.pending,
            queued: file.queued,
            todos: file.todos,
            steer: Vec::new(),
            edits: file.edits,
            running: false,
            cancel: Arc::new(tokio::sync::Notify::new()),
        }
    }

    /// The folder this session works in: its worktree when it has one, else the project.
    pub fn work_root(&self) -> PathBuf {
        self.summary
            .worktree
            .as_ref()
            .map(|tree| PathBuf::from(&tree.path))
            .filter(|path| path.is_dir())
            .unwrap_or_else(|| PathBuf::from(&self.summary.project_path))
    }

    pub fn push_visible(
        &mut self,
        role: &str,
        content: String,
        steps: Vec<String>,
        context_paths: Vec<String>,
        sources: Vec<crate::web::Source>,
        conversation_at: Option<usize>,
    ) {
        self.transcript.push(TranscriptEntry {
            id: Uuid::new_v4().to_string(),
            role: role.into(),
            content,
            steps,
            context_paths,
            images: Vec::new(),
            sources,
            conversation_at,
        });
    }

    /// Writes the session to disk, naming it after its first request.
    pub fn save(&mut self) -> Result<(), String> {
        self.summary.updated_at = now();
        if self.summary.title == "New chat" {
            if let Some(first) = self.transcript.iter().find(|item| item.role == "user") {
                self.summary.title = provisional_title(&first.content);
            }
        }
        let file = SessionFile {
            summary: SessionSummary {
                running: false,
                ..self.summary.clone()
            },
            conversation: self.conversation.clone(),
            transcript: self.transcript.clone(),
            pending: self.pending.clone(),
            queued: self.queued.clone(),
            todos: self.todos.clone(),
            edits: self.edits.clone(),
        };
        write(&file)
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        let (pending, pending_from_agent) = self
            .pending
            .as_ref()
            .map(pending_view)
            .map(|(view, from_agent)| (Some(view), from_agent))
            .unwrap_or((None, false));
        SessionSnapshot {
            todos: self.todos.clone(),
            session: SessionSummary {
                running: self.running,
                ..self.summary.clone()
            },
            messages: self.transcript.clone(),
            pending,
            pending_from_agent,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    pub session: SessionSummary,
    pub messages: Vec<TranscriptEntry>,
    pub pending: Option<PendingView>,
    pub pending_from_agent: bool,
    pub todos: Vec<crate::extras::Todo>,
}

pub fn lock(shared: &Shared) -> Result<MutexGuard<'_, Runtime>, String> {
    shared.lock().map_err(|e| e.to_string())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn dir() -> Result<PathBuf, String> {
    let path = data_dir()?.join("sessions");
    fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    Ok(path)
}

fn path(id: &str) -> Result<PathBuf, String> {
    Uuid::parse_str(id).map_err(|_| "Invalid session id")?;
    Ok(dir()?.join(format!("{id}.json")))
}

fn read(id: &str) -> Result<SessionFile, String> {
    serde_json::from_slice(&fs::read(path(id)?).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

fn write(session: &SessionFile) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(session).map_err(|e| e.to_string())?;
    let target = path(&session.summary.id)?;
    let temp = target.with_extension("json.tmp");
    fs::write(&temp, bytes).map_err(|e| e.to_string())?;
    fs::rename(&temp, &target).map_err(|e| e.to_string())
}

fn all_for(root: &Path) -> Result<Vec<SessionSummary>, String> {
    let root = root.to_string_lossy();
    Ok(all_sessions()?
        .into_iter()
        .filter(|summary| summary.project_path == root)
        .collect())
}

fn all_sessions() -> Result<Vec<SessionSummary>, String> {
    let mut summaries = Vec::new();
    for entry in fs::read_dir(dir()?).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry
            .path()
            .extension()
            .is_none_or(|extension| extension != "json")
        {
            continue;
        }
        let Ok(bytes) = fs::read(entry.path()) else {
            continue;
        };
        let Ok(file) = serde_json::from_slice::<SessionFile>(&bytes) else {
            continue;
        };
        summaries.push(file.summary);
    }
    summaries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(summaries)
}

pub(crate) fn pending_view(action: &PendingAction) -> (PendingView, bool) {
    match action {
        PendingAction::Edit {
            proposal,
            tool_call_id,
        } => (
            PendingView {
                kind: crate::workspace::proposal_kind(proposal).into(),
                label: crate::workspace::proposal_label(proposal),
                diff: Some(proposal.diff.clone()),
                questions: None,
            },
            tool_call_id.is_some(),
        ),
        // The plan's markdown travels in `diff`.
        PendingAction::Plan { plan, tool_call_id } => (
            PendingView { kind: "plan".into(), label: "Plan".into(), diff: Some(plan.clone()), questions: None },
            tool_call_id.is_some(),
        ),
        PendingAction::Question { questions, tool_call_id } => (
            PendingView {
                kind: "question".into(),
                label: questions.iter().map(|question| question.question.as_str()).collect::<Vec<_>>().join(" · "),
                diff: None,
                questions: Some(questions.clone()),
            },
            tool_call_id.is_some(),
        ),
        PendingAction::Task { task, tool_call_id } => (
            PendingView {
                kind: "task".into(),
                label: format!("npm run {task}"),
                diff: None,
                questions: None,
            },
            tool_call_id.is_some(),
        ),
        PendingAction::Command {
            command,
            tool_call_id,
            background,
            ..
        } => (
            PendingView {
                kind: "task".into(),
                label: if *background { format!("{command} (in the background)") } else { command.clone() },
                diff: None,
                questions: None,
            },
            tool_call_id.is_some(),
        ),
        PendingAction::Mcp {
            server,
            tool,
            arguments,
            tool_call_id,
        } => (
            PendingView {
                kind: "task".into(),
                label: format!(
                    "{server} · {tool} {}",
                    serde_json::to_string(arguments)
                        .unwrap_or_default()
                        .chars()
                        .take(400)
                        .collect::<String>()
                ),
                diff: None,
                questions: None,
            },
            tool_call_id.is_some(),
        ),
    }
}

/// The open project's own folder (worktree sessions still group under it).
pub fn main_root(state: &AppState) -> Result<PathBuf, String> {
    state
        .root
        .lock()
        .map_err(|e| e.to_string())?
        .clone()
        .ok_or_else(|| "Open a project first".into())
}

/// A session's runtime, loading it from disk the first time it is used.
pub fn runtime(state: &AppState, id: &str) -> Result<Shared, String> {
    let mut map = state.sessions.lock().map_err(|e| e.to_string())?;
    if let Some(shared) = map.get(id) {
        return Ok(shared.clone());
    }
    let shared = Arc::new(Mutex::new(Runtime::from_file(read(id)?)));
    map.insert(id.to_string(), shared.clone());
    Ok(shared)
}

/// The runtime of the session shown in the window.
pub fn active(state: &AppState) -> Result<Shared, String> {
    let id = state
        .active_session
        .lock()
        .map_err(|e| e.to_string())?
        .clone()
        .ok_or("No active session")?;
    runtime(state, &id)
}

/// Where tools and the explorer work for the shown session. Never call while holding a runtime lock.
pub fn active_work_root(state: &AppState) -> Option<PathBuf> {
    let shared = active(state).ok()?;
    let runtime = shared.lock().ok()?;
    runtime.summary.worktree.is_some().then(|| runtime.work_root())
}

fn activate(state: &AppState, shared: &Shared) -> Result<SessionSnapshot, String> {
    let runtime = lock(shared)?;
    *state.active_session.lock().map_err(|e| e.to_string())? = Some(runtime.summary.id.clone());
    Ok(runtime.snapshot())
}

fn create_worktree(root: &Path, id: &str) -> Result<Worktree, String> {
    if !root.join(".git").exists() {
        return Err("Worktree sessions need a Git repository".into());
    }
    let short = &id[..8];
    let name = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".into());
    let base = git::git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
        .map_err(|e| format!("The repository has no commit to branch from yet ({e})"))?
        .trim()
        .to_string();
    let branch = format!("neru/{short}");
    let folder = data_dir()?.join("worktrees").join(format!("{name}-{short}"));
    fs::create_dir_all(folder.parent().ok_or("Invalid worktree folder")?)
        .map_err(|e| e.to_string())?;
    let folder_text = folder.to_string_lossy().to_string();
    git::git(
        root,
        &["worktree", "add", "-b", &branch, &folder_text, "HEAD"],
    )?;
    Ok(Worktree {
        path: folder_text,
        branch,
        base,
    })
}

pub fn create_for_root(
    state: &AppState,
    root: &Path,
    worktree: bool,
) -> Result<SessionSnapshot, String> {
    let id = Uuid::new_v4().to_string();
    let worktree = if worktree {
        Some(create_worktree(root, &id)?)
    } else {
        None
    };
    let mut runtime = Runtime::from_file(SessionFile {
        summary: SessionSummary {
            id: id.clone(),
            title: "New chat".into(),
            project_path: root.to_string_lossy().to_string(),
            updated_at: now(),
            worktree,
            running: false,
            titled: false,
        },
        conversation: vec![],
        transcript: vec![],
        pending: None,
        queued: vec![],
        todos: vec![],
        edits: vec![],
    });
    runtime.save()?;
    let shared = Arc::new(Mutex::new(runtime));
    state
        .sessions
        .lock()
        .map_err(|e| e.to_string())?
        .insert(id, shared.clone());
    activate(state, &shared)
}

pub fn restore_for_root(state: &AppState, root: &Path) -> Result<SessionSnapshot, String> {
    if let Some(latest) = all_for(root)?.first() {
        return activate(state, &runtime(state, &latest.id)?);
    }
    create_for_root(state, root, false)
}

/// A chat with no folder. These stay out of every project's session list.
fn is_loose_chat(project_path: &str) -> bool {
    project_path.is_empty()
}

fn owned(state: &AppState, id: &str) -> Result<Shared, String> {
    let shared = runtime(state, id)?;
    let path = lock(&shared)?.summary.project_path.clone();
    if is_loose_chat(&path) {
        return Ok(shared);
    }
    let root = main_root(state)?;
    if path != root.to_string_lossy() {
        return Err("Session belongs to another project".into());
    }
    Ok(shared)
}

#[tauri::command]
pub fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionSummary>, String> {
    let mut list = all_for(&main_root(&state)?)?;
    mark_running(&state, &mut list);
    Ok(list)
}

fn mark_running(state: &AppState, list: &mut [SessionSummary]) {
    let Ok(map) = state.sessions.lock() else {
        return;
    };
    for summary in list {
        summary.running = map
            .get(&summary.id)
            .and_then(|shared| shared.lock().ok().map(|runtime| runtime.running))
            .unwrap_or(false);
    }
}

#[tauri::command]
pub fn list_all_sessions(state: State<'_, AppState>) -> Result<Vec<SessionSummary>, String> {
    let mut list = all_sessions()?;
    mark_running(&state, &mut list);
    Ok(list)
}

#[tauri::command]
pub fn current_session(state: State<'_, AppState>) -> Result<Option<SessionSnapshot>, String> {
    let Ok(root) = main_root(&state) else {
        return Ok(None);
    };
    if let Ok(shared) = active(&state) {
        return Ok(Some(lock(&shared)?.snapshot()));
    }
    Ok(Some(restore_for_root(&state, &root)?))
}

#[tauri::command]
pub fn session_snapshot(id: String, state: State<'_, AppState>) -> Result<SessionSnapshot, String> {
    Ok(lock(&runtime(&state, &id)?)?.snapshot())
}

#[tauri::command]
pub fn create_session(
    worktree: Option<bool>,
    state: State<'_, AppState>,
) -> Result<SessionSnapshot, String> {
    let root = main_root(&state)?;
    create_for_root(&state, &root, worktree.unwrap_or(false))
}

/// A conversation that is not attached to any folder, the way a chat is separate from a project.
#[tauri::command]
pub fn create_chat_session(state: State<'_, AppState>) -> Result<SessionSnapshot, String> {
    create_for_root(&state, Path::new(""), false)
}

/// A copy of a session's conversation in a new session, so a different approach can be tried without
/// losing the first. The copy works in the project folder itself: a worktree stays with its own session.
#[tauri::command]
pub fn fork_session(id: String, state: State<'_, AppState>) -> Result<SessionSnapshot, String> {
    let source = owned(&state, &id)?;
    let (summary, conversation, transcript, todos) = {
        let runtime = lock(&source)?;
        if runtime.running {
            return Err("Stop the reply, or wait for it to finish, before forking this session".into());
        }
        (runtime.summary.clone(), runtime.conversation.clone(), runtime.transcript.clone(), runtime.todos.clone())
    };
    let new_id = Uuid::new_v4().to_string();
    let title: String = format!("{} (fork)", summary.title.trim_end_matches(" (fork)")).chars().take(100).collect();
    let mut runtime = Runtime::from_file(SessionFile {
        summary: SessionSummary {
            id: new_id.clone(),
            title,
            project_path: summary.project_path,
            updated_at: now(),
            worktree: None,
            running: false,
            titled: true,
        },
        conversation,
        transcript,
        pending: None,
        queued: vec![],
        todos,
        edits: vec![],
    });
    runtime.save()?;
    let shared = Arc::new(Mutex::new(runtime));
    state.sessions.lock().map_err(|e| e.to_string())?.insert(new_id, shared.clone());
    activate(&state, &shared)
}

#[tauri::command]
pub fn select_session(id: String, state: State<'_, AppState>) -> Result<SessionSnapshot, String> {
    let shared = owned(&state, &id)?;
    activate(&state, &shared)
}

#[tauri::command]
pub fn rename_session(
    id: String,
    title: String,
    state: State<'_, AppState>,
) -> Result<SessionSummary, String> {
    let shared = owned(&state, &id)?;
    let title = title.trim();
    if title.is_empty() || title.chars().count() > 100 {
        return Err("Use a title of 1–100 characters".into());
    }
    let mut runtime = lock(&shared)?;
    runtime.summary.title = title.into();
    runtime.summary.titled = true;
    runtime.save()?;
    Ok(runtime.summary.clone())
}

/// A stand-in title until the model names the session: the request's first sentence, cut at a word.
pub fn provisional_title(request: &str) -> String {
    let text = request.split_whitespace().collect::<Vec<_>>().join(" ");
    let sentence = text.split(['.', '?', '!', '\n']).next().unwrap_or(&text).trim();
    let sentence = if sentence.is_empty() { text.as_str() } else { sentence };
    if sentence.chars().count() <= 52 {
        return sentence.to_string();
    }
    let cut: String = sentence.chars().take(52).collect();
    let cut = cut.rsplit_once(' ').map_or(cut.as_str(), |(head, _)| head);
    format!("{cut}…")
}

/// Cleans a model-written title: one line, no quotes, labels, or trailing punctuation.
pub fn clean_title(raw: &str) -> Option<String> {
    let line = raw.lines().map(str::trim).find(|line| !line.is_empty())?;
    let line = line.trim_start_matches(['#', '*', '-', ' ']);
    let line = line.strip_prefix("Title:").or_else(|| line.strip_prefix("title:")).unwrap_or(line);
    let title = line.trim().trim_matches(['"', '\'', '`', '*', '“', '”']).trim_end_matches(['.', '!', '?', ':']).trim();
    let title: String = title.chars().take(60).collect();
    (title.chars().count() >= 2).then_some(title)
}

#[tauri::command]
pub fn delete_session(id: String, state: State<'_, AppState>) -> Result<SessionSnapshot, String> {
    let shared = owned(&state, &id)?;
    let (worktree, loose, hook_root) = {
        let runtime = lock(&shared)?;
        if runtime.running {
            return Err("Stop this session's response first".into());
        }
        (
            runtime.summary.worktree.clone(),
            is_loose_chat(&runtime.summary.project_path),
            (!runtime.summary.project_path.is_empty()).then(|| runtime.work_root()),
        )
    };
    if let Some(root) = hook_root {
        // Off this thread: a slow hook must not hold up the window.
        let id = id.clone();
        std::thread::spawn(move || crate::hooks::session_end(&root, &id, "session_deleted"));
    }
    fs::remove_file(path(&id)?).map_err(|e| e.to_string())?;
    state.sessions.lock().map_err(|e| e.to_string())?.remove(&id);
    crate::shells::kill_session(&id);
    if let Some(tree) = worktree {
        if let Ok(root) = main_root(&state) {
            // A worktree with uncommitted work is kept; Git refuses to remove it without --force.
            let _ = git::git(&root, &["worktree", "remove", &tree.path]);
        }
    }
    if loose {
        if let Some(next) = all_sessions()?.into_iter().find(|item| is_loose_chat(&item.project_path)) {
            return activate(&state, &runtime(&state, &next.id)?);
        }
    }
    let active = state
        .active_session
        .lock()
        .map_err(|e| e.to_string())?
        .clone();
    match active {
        Some(current) if current != id => Ok(lock(&runtime(&state, &current)?)?.snapshot()),
        _ => {
            *state.active_session.lock().map_err(|e| e.to_string())? = None;
            restore_for_root(&state, &main_root(&state)?)
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RewindResult {
    pub snapshot: SessionSnapshot,
    /// The rewound message, so it can be edited and sent again.
    pub prompt: String,
    pub context_paths: Vec<String>,
    /// Files put back as they were before the message.
    pub restored: Vec<String>,
}

/// Rewinds the shown session to just before its `user_index`-th request (0-based), dropping
/// everything after it; with `restore_code`, also undoes the edits Neru applied since, newest first.
#[tauri::command]
pub fn rewind_session(
    user_index: usize,
    restore_code: bool,
    state: State<'_, AppState>,
) -> Result<RewindResult, String> {
    rewind(&state, user_index, restore_code)
}

pub fn rewind(state: &AppState, user_index: usize, restore_code: bool) -> Result<RewindResult, String> {
    let shared = active(state)?;
    let mut runtime = lock(&shared)?;
    if runtime.running {
        return Err("Stop the response before rewinding".into());
    }
    let position = runtime
        .transcript
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.role == "user")
        .nth(user_index)
        .map(|(index, _)| index)
        .ok_or("That message is no longer in this session")?;
    let entry = runtime.transcript[position].clone();
    let at = entry
        .conversation_at
        .ok_or("This message was sent before rewind was available, so it cannot be rewound")?;
    let (later, earlier): (Vec<_>, Vec<_>) = runtime
        .edits
        .iter()
        .cloned()
        .partition(|record| record.transcript_len > position);
    let mut restored = Vec::new();
    if restore_code {
        let root = runtime.work_root();
        for record in later.iter().rev() {
            restored.push(restore_checkpoint_file(&root, &record.checkpoint)?);
        }
        restored.dedup();
    }
    runtime.edits = earlier;
    runtime.transcript.truncate(position);
    runtime.conversation.truncate(at);
    runtime.pending = None;
    runtime.queued.clear();
    runtime.save()?;
    Ok(RewindResult {
        snapshot: runtime.snapshot(),
        prompt: entry.content,
        context_paths: entry.context_paths,
        restored,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_project() -> PathBuf {
        let root = std::env::temp_dir().join(format!("neru-session-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        root.canonicalize().unwrap()
    }

    #[test]
    fn session_survives_reload_with_transcript_and_pending_approval() {
        let root = temp_project();
        let state = AppState::default();
        *state.root.lock().unwrap() = Some(root.clone());
        let created = create_for_root(&state, &root, false).unwrap();
        {
            let shared = active(&state).unwrap();
            let mut runtime = shared.lock().unwrap();
            runtime.push_visible("user", "Inspect this project".into(), vec![], vec![], vec![], Some(2));
            runtime.pending = Some(PendingAction::Task {
                task: "test".into(),
                tool_call_id: Some("call-1".into()),
            });
            runtime.save().unwrap();
        }

        let restored = read(&created.session.id).unwrap();
        assert_eq!(restored.summary.title, "Inspect this project");
        assert_eq!(restored.transcript.len(), 1);
        assert_eq!(restored.transcript[0].conversation_at, Some(2));
        assert!(matches!(restored.pending, Some(PendingAction::Task { .. })));

        fs::remove_file(path(&created.session.id).unwrap()).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn sessions_keep_separate_runtimes() {
        let root = temp_project();
        let state = AppState::default();
        *state.root.lock().unwrap() = Some(root.clone());
        let first = create_for_root(&state, &root, false).unwrap();
        active(&state).unwrap().lock().unwrap().push_visible("user", "First".into(), vec![], vec![], vec![], Some(2));
        let second = create_for_root(&state, &root, false).unwrap();
        active(&state).unwrap().lock().unwrap().push_visible("user", "Second".into(), vec![], vec![], vec![], Some(2));

        let shown = activate(&state, &runtime(&state, &first.session.id).unwrap()).unwrap();
        assert_eq!(shown.messages[0].content, "First");
        let other = runtime(&state, &second.session.id).unwrap();
        assert_eq!(other.lock().unwrap().transcript[0].content, "Second");

        fs::remove_file(path(&first.session.id).unwrap()).unwrap();
        fs::remove_file(path(&second.session.id).unwrap()).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn rewind_drops_later_messages_and_restores_edits() {
        let root = temp_project();
        let state = AppState::default();
        *state.root.lock().unwrap() = Some(root.clone());
        let created = create_for_root(&state, &root, false).unwrap();
        let file = root.join("notes.txt");
        fs::write(&file, "before").unwrap();
        {
            let shared = active(&state).unwrap();
            let mut runtime = shared.lock().unwrap();
            runtime.conversation = vec![json_msg("system"), json_msg("mode")];
            runtime.push_visible("user", "first".into(), vec![], vec![], vec![], Some(2));
            runtime.conversation.push(json_msg("user"));
            runtime.push_visible("assistant", "ok".into(), vec![], vec![], vec![], None);
            runtime.push_visible("user", "change notes".into(), vec![], vec!["notes.txt".into()], vec![], Some(3));
            runtime.conversation.push(json_msg("user"));
            let checkpoint = crate::workspace::write_checkpoint(&root, "notes.txt", "before", true).unwrap();
            fs::write(&file, "after").unwrap();
            let len = runtime.transcript.len();
            runtime.edits.push(EditRecord { checkpoint, transcript_len: len });
        }
        let result = rewind(&state, 1, true).unwrap();
        assert_eq!(result.prompt, "change notes");
        assert_eq!(result.restored, vec!["notes.txt".to_string()]);
        assert_eq!(result.snapshot.messages.len(), 2);
        assert_eq!(fs::read_to_string(&file).unwrap(), "before");
        assert_eq!(active(&state).unwrap().lock().unwrap().conversation.len(), 3);
        assert!(rewind(&state, 5, false).is_err());

        fs::remove_file(path(&created.session.id).unwrap()).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    fn json_msg(role: &str) -> Value {
        serde_json::json!({ "role": role, "content": role })
    }
}

#[cfg(test)]
mod title_tests {
    use super::{clean_title, provisional_title};

    #[test]
    fn titles_read_like_tasks() {
        assert_eq!(clean_title("\"Fix Login Redirect Loop.\"\n"), Some("Fix Login Redirect Loop".into()));
        assert_eq!(clean_title("Title: Add Dark Mode"), Some("Add Dark Mode".into()));
        assert_eq!(clean_title("   "), None);
        assert_eq!(provisional_title("Please fix the sidebar. It overlaps the footer"), "Please fix the sidebar");
        assert!(provisional_title(&"word ".repeat(40)).ends_with('…'));
    }
}
