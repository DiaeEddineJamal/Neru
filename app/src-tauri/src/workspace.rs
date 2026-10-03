use std::{
    fs,
    path::{Path, PathBuf},
};

use ignore::WalkBuilder;
use serde::{Deserialize, Serialize};
use similar::TextDiff;
use tauri::State;

use crate::{AppState, PendingAction, sessions};

const MAX_READ: u64 = 1_000_000;

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    pub name: String,
    pub path: String,
    pub git: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub path: String,
    pub line: usize,
    /// Where the match starts in `preview`, in characters, and how long it is.
    pub column: usize,
    pub length: usize,
    pub preview: String,
}

/// Answer to the Search view: the hits plus how the index did.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    pub hits: Vec<SearchHit>,
    /// Files actually read, out of `total_files` in the project.
    pub files_scanned: usize,
    pub total_files: usize,
    pub truncated: bool,
    pub ms: u64,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct EditProposal {
    pub path: String,
    pub diff: String,
    pub original: String,
    pub content: String,
    /// Removes the file instead of writing `content`.
    #[serde(default)]
    pub delete: bool,
    /// A deleted file that is not UTF-8 text; the checkpoint keeps its bytes.
    #[serde(default)]
    pub binary: bool,
    /// Folder and move operations: "mkdir", "move", or "delete_dir". Empty for file writes and deletes.
    #[serde(default)]
    pub op: String,
    /// Destination of a move.
    #[serde(default)]
    pub to: String,
}

/// A path as people write it: without Windows' `\\?\` prefix, which canonicalize adds.
pub(crate) fn shown(path: &Path) -> String {
    let text = path.display().to_string();
    text.strip_prefix(r"\\?\UNC\").map(|rest| format!(r"\\{rest}")).unwrap_or_else(|| text.trim_start_matches(r"\\?\").to_string())
}

pub fn data_dir() -> Result<PathBuf, String> {
    let project_data = PathBuf::from(r"D:\Neru\.local\data");
    let path = std::env::var_os("NERU_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            project_data
                .parent()
                .is_some_and(|p| p.exists())
                .then_some(project_data)
        })
        .or_else(|| dirs::data_local_dir().map(|p| p.join("Neru")))
        .ok_or("Unable to locate application data directory")?;
    fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    Ok(path)
}

/// The folder the shown session works in: its Git worktree when it has one, else the project.
/// Never call while holding a session runtime lock.
pub fn project_root(state: &AppState) -> Result<PathBuf, String> {
    if let Some(root) = sessions::active_work_root(state) {
        return Ok(root);
    }
    state
        .root
        .lock()
        .map_err(|e| e.to_string())?
        .clone()
        .ok_or_else(|| "Open a project first".into())
}

pub fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

pub fn resolve_existing(root: &Path, relative: &str) -> Result<PathBuf, String> {
    // A folder added with --add-dir is reached by its absolute path.
    if let Some(dir) = crate::run_options::added_dir_for(Path::new(relative)) {
        if dir != root {
            return resolve_existing(&dir, relative);
        }
    }
    let candidate = root.join(relative);
    let canonical = candidate.canonicalize().map_err(|e| e.to_string())?;
    if !canonical.starts_with(root) {
        return Err("Path escapes the open project".into());
    }
    Ok(canonical)
}

/// A path inside the project that may not exist yet, including new folders along the way
/// (`src/features/auth/login.ts` when `features/` is new). `..` and absolute paths are refused.
pub fn resolve_new(root: &Path, relative: &str) -> Result<PathBuf, String> {
    if let Some(dir) = crate::run_options::added_dir_for(Path::new(relative)) {
        if dir != root {
            let canonical = Path::new(relative).canonicalize().ok();
            let inside = canonical.as_deref().and_then(|path| path.strip_prefix(&dir).ok()).or_else(|| Path::new(relative).strip_prefix(&dir).ok());
            if let Some(inside) = inside.map(Path::to_path_buf) {
                return resolve_new(&dir, &inside.to_string_lossy());
            }
        }
    }
    let candidate = root.join(relative);
    if candidate.exists() {
        return resolve_existing(root, relative);
    }
    let mut parts = Vec::new();
    for component in Path::new(relative).components() {
        match component {
            std::path::Component::Normal(part) => parts.push(part.to_os_string()),
            std::path::Component::CurDir => {}
            _ => return Err("Use a path relative to the project, without ..".into()),
        }
    }
    let name = parts.pop().ok_or("Invalid file name")?;
    // Walk up to the deepest folder that exists, then check it is inside the project.
    let mut existing = root.to_path_buf();
    let mut missing = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        let next = existing.join(part);
        if next.exists() {
            existing = next;
        } else {
            missing = parts[index..].to_vec();
            break;
        }
    }
    let base = existing.canonicalize().map_err(|e| e.to_string())?;
    if !base.starts_with(root) {
        return Err("Path escapes the open project".into());
    }
    if !base.is_dir() {
        return Err(format!("{} is a file, not a folder", relative_path(root, &base)));
    }
    let mut path = base;
    for part in missing {
        path.push(part);
    }
    path.push(name);
    Ok(path)
}

pub fn read_limited(path: &Path) -> Result<String, String> {
    let metadata = fs::metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() {
        return Err("Not a file".into());
    }
    if metadata.len() > MAX_READ {
        return Err("File exceeds the 1 MB reading limit".into());
    }
    fs::read_to_string(path).map_err(|e| e.to_string())
}

fn info(root: &Path) -> ProjectInfo {
    ProjectInfo {
        name: root
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string(),
        path: root.to_string_lossy().to_string(),
        git: root.join(".git").exists(),
    }
}

#[tauri::command]
pub fn open_project(path: String, state: State<'_, AppState>) -> Result<ProjectInfo, String> {
    // Responses already running in other sessions keep going; they carry their own folder.
    let root = PathBuf::from(&path)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if !root.is_dir() {
        return Err("Select a directory".into());
    }
    *state.root.lock().map_err(|e| e.to_string())? = Some(root.clone());
    *state.active_session.lock().map_err(|e| e.to_string())? = None;
    crate::index::start(&root);
    sessions::restore_for_root(&state, &root)?;
    let recent_path = data_dir()?.join("recent-projects.json");
    let mut recent: Vec<String> = fs::read_to_string(&recent_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    recent.retain(|item| item != &root.to_string_lossy());
    recent.insert(0, root.to_string_lossy().to_string());
    recent.truncate(12);
    fs::write(
        recent_path,
        serde_json::to_vec_pretty(&recent).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(info(&root))
}

#[tauri::command]
pub fn current_project(state: State<'_, AppState>) -> Result<Option<ProjectInfo>, String> {
    Ok(state
        .root
        .lock()
        .map_err(|e| e.to_string())?
        .as_ref()
        .map(|p| info(p)))
}

#[tauri::command]
pub fn recent_projects() -> Result<Vec<String>, String> {
    let path = data_dir()?.join("recent-projects.json");
    Ok(fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default())
}

#[tauri::command]
pub fn list_directory(
    path: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<FileEntry>, String> {
    let root = project_root(&state)?;
    let dir = resolve_existing(&root, path.as_deref().unwrap_or(""))?;
    if !dir.is_dir() {
        return Err("Not a directory".into());
    }
    let mut entries = Vec::new();
    for item in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let item = item.map_err(|e| e.to_string())?;
        let name = item.file_name().to_string_lossy().to_string();
        if crate::index::is_skipped_dir(&name) {
            continue;
        }
        let meta = item.metadata().map_err(|e| e.to_string())?;
        entries.push(FileEntry {
            name,
            path: relative_path(&root, &item.path()),
            is_dir: meta.is_dir(),
            size: meta.len(),
        });
        if entries.len() >= 2000 {
            break;
        }
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

#[tauri::command]
pub fn read_file(path: String, state: State<'_, AppState>) -> Result<String, String> {
    let root = project_root(&state)?;
    read_limited(&resolve_existing(&root, &path)?)
}

/// Project search for the Search view: plain text by default, or a regex; case and whole-word
/// options; ignores what Git ignores. Runs off the main thread on the project index.
#[tauri::command]
pub async fn search_text(
    query: String,
    case_sensitive: Option<bool>,
    whole_word: Option<bool>,
    regex: Option<bool>,
    glob: Option<String>,
    state: State<'_, AppState>,
) -> Result<SearchResults, String> {
    let root = project_root(&state)?;
    let query = query.trim().to_string();
    if query.chars().count() < 2 {
        return Err("Type at least two characters to search".into());
    }
    let mut request = crate::index::Query::literal(&query, 500);
    request.regex = regex.unwrap_or(false);
    request.case_sensitive = case_sensitive.unwrap_or(false);
    request.whole_word = whole_word.unwrap_or(false);
    request.glob = glob.map(|g| g.trim().to_string()).filter(|g| !g.is_empty());
    // Validate here so a bad pattern is reported before any thread starts.
    request.matcher()?;
    let output = tauri::async_runtime::spawn_blocking(move || crate::index::search(&root, &request))
        .await
        .map_err(|e| e.to_string())??;
    Ok(SearchResults { hits: output.hits, files_scanned: output.files_scanned, total_files: output.total_files, truncated: output.truncated, ms: output.ms })
}

/// Matches of `matcher` in one file's text, one hit per line, appended to `hits` up to `limit`.
pub fn line_hits(path: &str, text: &str, matcher: &regex::Regex, hits: &mut Vec<SearchHit>, limit: usize) {
    for (i, line) in text.lines().enumerate() {
        let Some(found) = matcher.find(line) else { continue };
        // Show the match with some context before it, even on long minified lines.
        let start = line[..found.start()].char_indices().rev().nth(60).map_or(0, |(at, _)| at);
        let preview: String = line[start..].trim_end().chars().take(200).collect();
        let offset = line[start..found.start()].chars().count();
        let trimmed = preview.len() - preview.trim_start().len();
        hits.push(SearchHit {
            path: path.to_string(),
            line: i + 1,
            column: offset.saturating_sub(trimmed),
            length: line[found.start()..found.end()].chars().count(),
            preview: preview.trim_start().to_string(),
        });
        if hits.len() >= limit {
            return;
        }
    }
}

/// The unindexed search: walks the project and reads every text file. Used when the index is not
/// available, and by tests to check the index against.
pub fn search_project(root: &Path, matcher: &regex::Regex, limit: usize) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    let mut builder = WalkBuilder::new(root);
    builder.max_filesize(Some(MAX_READ)).follow_links(false).hidden(false).require_git(false);
    builder.filter_entry(|entry| !(entry.file_type().is_some_and(|t| t.is_dir()) && entry.depth() > 0 && crate::index::is_skipped_dir(&entry.file_name().to_string_lossy())));
    for result in builder.build() {
        let Ok(entry) = result else { continue };
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Ok(bytes) = fs::read(entry.path()) else { continue };
        if bytes[..bytes.len().min(8192)].contains(&0) {
            continue;
        }
        line_hits(&relative_path(root, entry.path()), &String::from_utf8_lossy(&bytes), matcher, &mut hits, limit);
        if hits.len() >= limit {
            return hits;
        }
    }
    hits
}

#[tauri::command]
pub fn list_project_files(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let root = project_root(&state)?;
    if let Some(handle) = crate::index::ready(&root, std::time::Duration::from_secs(15)) {
        return Ok(handle.all_paths(20_000));
    }
    let mut files = Vec::new();
    for entry in WalkBuilder::new(&root).follow_links(false).build() {
        let Ok(entry) = entry else { continue };
        if entry.file_type().is_some_and(|kind| kind.is_file()) {
            files.push(relative_path(&root, entry.path()));
            if files.len() == 2000 {
                break;
            }
        }
    }
    files.sort();
    Ok(files)
}

/// A rewritten file keeps the style of the one it replaces: CRLF line endings when the original
/// used them throughout, and its byte order mark.
pub fn keep_file_style(original: &str, mut content: String) -> String {
    if original.is_empty() {
        return content;
    }
    let crlf = original.matches("\r\n").count();
    let lf = original.matches('\n').count() - crlf;
    if crlf > 0 && crlf >= lf && !content.contains('\r') {
        content = content.replace('\n', "\r\n");
    }
    if original.starts_with('\u{feff}') && !content.starts_with('\u{feff}') {
        content.insert(0, '\u{feff}');
    }
    content
}

/// Writes through a temp file in the same folder and renames it over the target, so a crash or a
/// reader never sees a half-written file. Falls back to a direct write if the rename is refused.
pub fn write_atomic(path: &Path, content: &[u8]) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().ok_or("Invalid file path")?;
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let temp = dir.join(format!(".{name}.neru-{}-{}.tmp", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));
    let attempt = fs::write(&temp, content).and_then(|_| {
        if let Ok(meta) = fs::metadata(path) {
            let _ = fs::set_permissions(&temp, meta.permissions());
        }
        fs::rename(&temp, path)
    });
    if attempt.is_err() {
        let _ = fs::remove_file(&temp);
        return fs::write(path, content).map_err(|e| e.to_string());
    }
    Ok(())
}

pub fn make_proposal(root: &Path, path: &str, content: String) -> Result<EditProposal, String> {
    if content.len() > 1_000_000 {
        return Err("Proposed file exceeds 1 MB".into());
    }
    let target = resolve_new(root, path)?;
    if target.is_dir() {
        return Err("Cannot overwrite a directory".into());
    }
    let original = if target.exists() {
        read_limited(&target)?
    } else {
        String::new()
    };
    let relative = relative_path(root, &target);
    let content = keep_file_style(&original, content);
    let diff = TextDiff::from_lines(&original, &content)
        .unified_diff()
        .header(&format!("a/{relative}"), &format!("b/{relative}"))
        .to_string();
    Ok(EditProposal {
        path: relative,
        diff,
        original,
        content,
        delete: false,
        binary: false,
        op: String::new(),
        to: String::new(),
    })
}

/// A proposal to delete one project file. The diff shows every line removed, like a
/// deletion in Cursor or Claude Code, and the checkpoint lets rewind bring it back.
pub fn make_delete_proposal(root: &Path, path: &str) -> Result<EditProposal, String> {
    let target = resolve_existing(root, path)?;
    if target == root {
        return Err("Cannot delete the project root".into());
    }
    if target.is_dir() {
        let relative = relative_path(root, &target);
        let (files, bytes) = tree_size(&target);
        if files > MAX_TREE_FILES || bytes > MAX_TREE_BYTES {
            return Err(format!("{relative} holds {files} files ({} MB), too many to checkpoint. Remove it with run_shell_command instead", bytes / 1_000_000));
        }
        let listing = list_tree(root, &target, 40);
        let diff = format!("--- a/{relative}/\n+++ /dev/null\nFolder deleted with {files} {}:\n{listing}", if files == 1 { "file" } else { "files" });
        return Ok(EditProposal { path: relative, diff, original: String::new(), content: String::new(), delete: true, binary: false, op: "delete_dir".into(), to: String::new() });
    }
    let relative = relative_path(root, &target);
    let (original, binary) = match read_limited(&target) {
        Ok(text) => (text, false),
        Err(_) => (String::new(), true),
    };
    let diff = if binary {
        format!("--- a/{relative}\n+++ /dev/null\nBinary file deleted\n")
    } else {
        TextDiff::from_lines(original.as_str(), "")
            .unified_diff()
            .header(&format!("a/{relative}"), "/dev/null")
            .to_string()
    };
    Ok(EditProposal { path: relative, diff, original, content: String::new(), delete: true, binary, op: String::new(), to: String::new() })
}

const MAX_TREE_FILES: usize = 5_000;
const MAX_TREE_BYTES: u64 = 200_000_000;

fn tree_size(dir: &Path) -> (usize, u64) {
    let mut files = 0;
    let mut bytes = 0;
    for entry in walk(dir) {
        if let Ok(meta) = fs::metadata(&entry) {
            if meta.is_file() {
                files += 1;
                bytes += meta.len();
            }
        }
    }
    (files, bytes)
}

/// Every file and folder under `dir` (not following links), depth first.
fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let Ok(entries) = fs::read_dir(&current) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                stack.push(path.clone());
            }
            found.push(path);
        }
        if found.len() > MAX_TREE_FILES * 2 {
            break;
        }
    }
    found
}

fn list_tree(root: &Path, dir: &Path, limit: usize) -> String {
    let mut files: Vec<String> = walk(dir).into_iter().filter(|path| path.is_file()).map(|path| format!("-{}", relative_path(root, &path))).collect();
    files.sort();
    let more = files.len().saturating_sub(limit);
    files.truncate(limit);
    if more > 0 {
        files.push(format!("… and {more} more"));
    }
    files.join("\n")
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(from).map_err(|e| e.to_string())?.flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// How the window shows a pending proposal: "edit", "delete", "move", or "mkdir".
pub fn proposal_kind(proposal: &EditProposal) -> &'static str {
    match proposal.op.as_str() {
        "move" => "move",
        "mkdir" => "mkdir",
        _ if proposal.delete => "delete",
        _ => "edit",
    }
}

pub fn proposal_label(proposal: &EditProposal) -> String {
    if proposal.op == "move" { format!("{} → {}", proposal.path, proposal.to) } else { proposal.path.clone() }
}

/// A proposal to create a folder (and any missing parents).
pub fn make_folder_proposal(root: &Path, path: &str) -> Result<EditProposal, String> {
    let target = resolve_new(root, path)?;
    if target.exists() {
        return Err(format!("{path} already exists"));
    }
    let relative = relative_path(root, &target);
    let diff = format!("--- /dev/null\n+++ b/{relative}/\nNew folder\n");
    Ok(EditProposal { path: relative, diff, original: String::new(), content: String::new(), delete: false, binary: false, op: "mkdir".into(), to: String::new() })
}

/// A proposal to rename or move a file or folder; missing destination folders are created.
pub fn make_move_proposal(root: &Path, from: &str, to: &str) -> Result<EditProposal, String> {
    let source = resolve_existing(root, from)?;
    if source == root {
        return Err("Cannot move the project root".into());
    }
    let target = resolve_new(root, to)?;
    if target.exists() {
        return Err(format!("{to} already exists; delete it first or pick another name"));
    }
    if target.starts_with(&source) {
        return Err("Cannot move a folder into itself".into());
    }
    let (from_rel, to_rel) = (relative_path(root, &source), relative_path(root, &target));
    let kind = if source.is_dir() { "folder" } else { "file" };
    let diff = format!("--- a/{from_rel}\n+++ b/{to_rel}\nRename {kind} {from_rel} → {to_rel}\n");
    Ok(EditProposal { path: from_rel, diff, original: String::new(), content: String::new(), delete: false, binary: false, op: "move".into(), to: to_rel })
}

/// Checkpoint for a folder operation, so rewind and undo can reverse it.
fn write_op_checkpoint(root: &Path, proposal: &EditProposal) -> Result<String, String> {
    let checkpoint_dir = data_dir()?.join("checkpoints");
    fs::create_dir_all(&checkpoint_dir).map_err(|e| e.to_string())?;
    let id = uuid::Uuid::new_v4().to_string();
    if proposal.op == "delete_dir" {
        copy_tree(&resolve_existing(root, &proposal.path)?, &checkpoint_dir.join(format!("{id}.tree")))?;
    }
    let checkpoint = serde_json::json!({"project": root, "path": proposal.path, "op": proposal.op, "to": proposal.to, "original": "", "existed": proposal.op != "mkdir"});
    fs::write(checkpoint_dir.join(format!("{id}.json")), checkpoint.to_string()).map_err(|e| e.to_string())?;
    Ok(id)
}

fn apply_op(root: &Path, proposal: &EditProposal) -> Result<(String, String), String> {
    match proposal.op.as_str() {
        "mkdir" => {
            let target = resolve_new(root, &proposal.path)?;
            if target.exists() {
                return Err("Folder already exists".into());
            }
            let checkpoint = write_op_checkpoint(root, proposal)?;
            fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            Ok((checkpoint, String::new()))
        }
        "move" => {
            let source = resolve_existing(root, &proposal.path)?;
            let target = resolve_new(root, &proposal.to)?;
            if target.exists() {
                return Err(format!("{} already exists", proposal.to));
            }
            let checkpoint = write_op_checkpoint(root, proposal)?;
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::rename(&source, &target).map_err(|e| e.to_string())?;
            remove_empty_parents(root, &source);
            Ok((checkpoint, String::new()))
        }
        "delete_dir" => {
            let target = resolve_existing(root, &proposal.path)?;
            if target == root || !target.is_dir() {
                return Err("Folder no longer exists".into());
            }
            let checkpoint = write_op_checkpoint(root, proposal)?;
            fs::remove_dir_all(&target).map_err(|e| e.to_string())?;
            remove_empty_parents(root, &target);
            Ok((checkpoint, String::new()))
        }
        other => Err(format!("Unknown file operation {other}")),
    }
}

#[tauri::command]
pub fn propose_file(
    path: String,
    content: String,
    state: State<'_, AppState>,
) -> Result<EditProposal, String> {
    let root = project_root(&state)?;
    let proposal = make_proposal(&root, &path, content)?;
    let shared = sessions::active(&state)?;
    let mut runtime = sessions::lock(&shared)?;
    if runtime.running {
        return Err("Wait for this session's response first".into());
    }
    runtime.pending = Some(PendingAction::Edit {
        proposal: proposal.clone(),
        tool_call_id: None,
    });
    runtime.save()?;
    Ok(proposal)
}

/// Saves what a file looked like before Neru changed it; returns the checkpoint id.
pub fn write_checkpoint(
    root: &Path,
    path: &str,
    original: &str,
    existed: bool,
) -> Result<String, String> {
    let checkpoint_dir = data_dir()?.join("checkpoints");
    fs::create_dir_all(&checkpoint_dir).map_err(|e| e.to_string())?;
    let id = uuid::Uuid::new_v4().to_string();
    let checkpoint = serde_json::json!({"project": root, "path": path, "original": original, "existed": existed});
    fs::write(checkpoint_dir.join(format!("{id}.json")), checkpoint.to_string())
        .map_err(|e| e.to_string())?;
    Ok(id)
}

/// Checkpoint for a deleted binary file: its bytes are kept beside the record.
fn write_binary_checkpoint(root: &Path, path: &str, file: &Path) -> Result<String, String> {
    let checkpoint_dir = data_dir()?.join("checkpoints");
    fs::create_dir_all(&checkpoint_dir).map_err(|e| e.to_string())?;
    let id = uuid::Uuid::new_v4().to_string();
    fs::copy(file, checkpoint_dir.join(format!("{id}.bin"))).map_err(|e| e.to_string())?;
    let checkpoint = serde_json::json!({"project": root, "path": path, "original": "", "existed": true, "binary": true});
    fs::write(checkpoint_dir.join(format!("{id}.json")), checkpoint.to_string()).map_err(|e| e.to_string())?;
    Ok(id)
}

/// Removes folders a deletion left empty, up to (not including) the project root.
fn remove_empty_parents(root: &Path, file: &Path) {
    let mut dir = file.parent();
    while let Some(current) = dir {
        if current == root || !current.starts_with(root) {
            break;
        }
        if fs::remove_dir(current).is_err() {
            break;
        }
        dir = current.parent();
    }
}

/// Puts a file back as a checkpoint recorded it; returns the file's relative path.
pub fn restore_checkpoint_file(root: &Path, id: &str) -> Result<String, String> {
    if !id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return Err("Invalid checkpoint".into());
    }
    let text = fs::read_to_string(data_dir()?.join("checkpoints").join(format!("{id}.json")))
        .map_err(|e| e.to_string())?;
    let data: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if data["project"].as_str() != Some(root.to_string_lossy().as_ref()) {
        return Err("Checkpoint belongs to another folder".into());
    }
    let path = data["path"].as_str().ok_or("Invalid checkpoint path")?;
    match data["op"].as_str().unwrap_or("") {
        "mkdir" => {
            // Only an empty folder is removed; anything added since stays.
            if let Ok(target) = resolve_existing(root, path) {
                let _ = fs::remove_dir(target);
            }
            crate::index::note_changed(root, &[path.to_string()]);
            return Ok(path.to_string());
        }
        "move" => {
            let to = data["to"].as_str().ok_or("Invalid checkpoint")?;
            let moved = resolve_existing(root, to)?;
            let back = resolve_new(root, path)?;
            if back.exists() {
                return Err(format!("{path} exists again; cannot move {to} back"));
            }
            if let Some(parent) = back.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            fs::rename(&moved, &back).map_err(|e| e.to_string())?;
            remove_empty_parents(root, &moved);
            crate::index::note_changed(root, &[path.to_string(), to.to_string()]);
            return Ok(path.to_string());
        }
        "delete_dir" => {
            let back = resolve_new(root, path)?;
            copy_tree(&data_dir()?.join("checkpoints").join(format!("{id}.tree")), &back)?;
            crate::index::note_changed(root, &[path.to_string()]);
            return Ok(path.to_string());
        }
        _ => {}
    }
    let target = resolve_new(root, path)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    if data["binary"].as_bool() == Some(true) {
        fs::copy(data_dir()?.join("checkpoints").join(format!("{id}.bin")), &target).map_err(|e| e.to_string())?;
    } else if data["existed"].as_bool() == Some(true) {
        write_atomic(&target, data["original"].as_str().ok_or("Invalid checkpoint")?.as_bytes())?;
    } else if target.exists() {
        fs::remove_file(target).map_err(|e| e.to_string())?;
    }
    crate::index::note_changed(root, &[path.to_string()]);
    Ok(path.to_string())
}

/// Writes an approved (or auto-accepted) proposal after checking the file did not change since.
/// Returns the checkpoint id that can put the file back.
pub fn apply_edit(root: &Path, proposal: &EditProposal) -> Result<(String, String), String> {
    if !proposal.op.is_empty() {
        let applied = apply_op(root, proposal)?;
        crate::index::note_changed(root, &[proposal.path.clone(), proposal.to.clone()]);
        return Ok(applied);
    }
    let target = resolve_new(root, &proposal.path)?;
    if proposal.delete {
        if !target.is_file() {
            return Err("File no longer exists".into());
        }
        let checkpoint = if proposal.binary {
            write_binary_checkpoint(root, &proposal.path, &target)?
        } else {
            if read_limited(&target)? != proposal.original {
                return Err("File changed since proposal; review again".into());
            }
            write_checkpoint(root, &proposal.path, &proposal.original, true)?
        };
        fs::remove_file(&target).map_err(|e| e.to_string())?;
        remove_empty_parents(root, &target);
        crate::index::note_changed(root, &[proposal.path.clone()]);
        return Ok((checkpoint, String::new()));
    }
    let current = if target.exists() {
        read_limited(&target)?
    } else {
        String::new()
    };
    if current != proposal.original {
        return Err("File changed since proposal; review again".into());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let checkpoint = write_checkpoint(root, &proposal.path, &proposal.original, target.exists())?;
    write_atomic(&target, proposal.content.as_bytes())?;
    crate::index::note_changed(root, &[proposal.path.clone()]);
    let hooks = crate::hooks::after_edit(root, &proposal.path);
    Ok((checkpoint, hooks))
}

/// What a checkpoint recorded: (relative path, original text, whether the file existed).
pub fn read_checkpoint(id: &str) -> Result<(String, String, bool), String> {
    if !id.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return Err("Invalid checkpoint".into());
    }
    let text = fs::read_to_string(data_dir()?.join("checkpoints").join(format!("{id}.json")))
        .map_err(|e| e.to_string())?;
    let data: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if data["op"].as_str().is_some_and(|op| !op.is_empty()) || data["binary"].as_bool() == Some(true) {
        return Err("Folder, move, and binary checkpoints have no text snapshot".into());
    }
    Ok((
        data["path"].as_str().ok_or("Invalid checkpoint path")?.to_string(),
        data["original"].as_str().unwrap_or("").to_string(),
        data["existed"].as_bool().unwrap_or(true),
    ))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionChange {
    pub path: String,
    pub diff: String,
    pub additions: usize,
    pub deletions: usize,
    /// "added", "modified" or "deleted".
    pub status: String,
}

/// Every file Neru changed in the shown session, diffed against how it was before the session's
/// first edit to it.
#[tauri::command]
pub fn session_changes(state: State<'_, AppState>) -> Result<Vec<SessionChange>, String> {
    let root = project_root(&state)?;
    let shared = sessions::active(&state)?;
    let edits = sessions::lock(&shared)?.edits.clone();
    let mut first: Vec<(String, String, bool)> = Vec::new();
    for record in &edits {
        let Ok((path, original, existed)) = read_checkpoint(&record.checkpoint) else { continue };
        if !first.iter().any(|(known, _, _)| known == &path) {
            first.push((path, original, existed));
        }
    }
    let mut changes = Vec::new();
    for (path, original, existed) in first {
        let target = resolve_new(&root, &path)?;
        let now = if target.exists() { read_limited(&target).unwrap_or_default() } else { String::new() };
        if now == original && (existed || !target.exists()) {
            continue;
        }
        let diff = TextDiff::from_lines(&original, &now);
        let (mut additions, mut deletions) = (0, 0);
        for change in diff.iter_all_changes() {
            match change.tag() {
                similar::ChangeTag::Insert => additions += 1,
                similar::ChangeTag::Delete => deletions += 1,
                similar::ChangeTag::Equal => {}
            }
        }
        changes.push(SessionChange {
            status: if !existed { "added" } else if !target.exists() { "deleted" } else { "modified" }.into(),
            diff: diff.unified_diff().header(&format!("a/{path}"), &format!("b/{path}")).to_string(),
            path,
            additions,
            deletions,
        });
    }
    Ok(changes)
}

#[tauri::command]
pub fn apply_pending(state: State<'_, AppState>) -> Result<String, String> {
    let root = project_root(&state)?;
    let shared = sessions::active(&state)?;
    let mut runtime = sessions::lock(&shared)?;
    let action = runtime.pending.clone().ok_or("No pending action")?;
    match action {
        PendingAction::Edit {
            proposal,
            tool_call_id,
        } => {
            let (checkpoint_id, _) = apply_edit(&root, &proposal)?;
            runtime.pending = None;
            let transcript_len = runtime.transcript.len();
            runtime.edits.push(sessions::EditRecord {
                checkpoint: checkpoint_id.clone(),
                transcript_len,
            });
            if let Some(id) = tool_call_id {
                runtime.conversation.push(serde_json::json!({"role":"tool","tool_call_id":id,"content":format!("Applied {}{}.", if proposal.delete { "deletion of " } else { "" }, proposal.path)}));
            }
            runtime.save()?;
            Ok(checkpoint_id)
        }
        _ => Err("Use run_pending_task for this approval".into()),
    }
}

#[tauri::command]
pub fn reject_pending(state: State<'_, AppState>) -> Result<(), String> {
    deny_pending(&state, "User rejected this operation")
}

/// Answers the waiting call with `reason` instead of running it (`neru -p`, which cannot ask).
pub fn deny_pending(state: &AppState, reason: &str) -> Result<(), String> {
    let shared = sessions::active(state)?;
    let mut runtime = sessions::lock(&shared)?;
    if let Some(action) = runtime.pending.take() {
        if let Some(id) = action.tool_call_id() {
            runtime.conversation.push(serde_json::json!({"role":"tool","tool_call_id":id,"content":reason}));
        }
    }
    // Later calls from the same turn never ran; answer them so the conversation stays valid.
    for call in std::mem::take(&mut runtime.queued) {
        if let Some(id) = call["id"].as_str() {
            runtime.conversation.push(serde_json::json!({"role":"tool","tool_call_id":id,"content":"Not run: the user rejected an earlier action in this turn. Ask before trying it again."}));
        }
    }
    runtime.save()
}

#[tauri::command]
pub fn restore_checkpoint(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let root = project_root(&state)?;
    restore_checkpoint_file(&root, &id)?;
    let shared = sessions::active(&state)?;
    let mut runtime = sessions::lock(&shared)?;
    runtime.edits.retain(|record| record.checkpoint != id);
    runtime.save()
}

/// Writes a file the user edited and keeps a checkpoint so it can be undone.
#[tauri::command]
pub fn save_file(path: String, content: String, state: State<'_, AppState>) -> Result<String, String> {
    let root = project_root(&state)?;
    let proposal = make_proposal(&root, &path, content)?;
    let (checkpoint, _) = apply_edit(&root, &proposal)?;
    Ok(checkpoint)
}

/// Opens a project file in Cursor, VS Code, or the system default application.
#[tauri::command]
pub fn open_in_editor(path: String, state: State<'_, AppState>) -> Result<(), String> {
    let root = project_root(&state)?;
    let full = resolve_existing(&root, &path)?;
    let path = full.to_string_lossy().to_string();
    for program in ["cursor", "code"] {
        if std::process::Command::new(program).args(["-g", &path]).spawn().is_ok() {
            return Ok(());
        }
    }
    #[cfg(windows)]
    {
        let quoted = format!("\"{path}\"");
        std::process::Command::new("cmd").args(["/c", "start", "", &quoted]).spawn().map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(&path).spawn().map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open").arg(&path).spawn().map_err(|e| e.to_string())?;
        Ok(())
    }
}


#[cfg(test)]
mod write_tests {
    use super::*;

    #[test]
    fn atomic_writes_replace_files_and_leave_no_temp_behind() {
        let dir = std::env::temp_dir().join(format!("neru-atomic-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("page.html");
        write_atomic(&file, b"one").unwrap();
        write_atomic(&file, b"two, longer").unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "two, longer");
        let names: Vec<String> = fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        assert_eq!(names, ["page.html"], "no temp file is left in the project");
        assert!(write_atomic(&dir.join("missing").join("x.txt"), b"x").is_err());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rewritten_files_keep_their_line_endings_and_bom() {
        assert_eq!(keep_file_style("a\r\nb\r\n", "x\ny\n".into()), "x\r\ny\r\n");
        assert_eq!(keep_file_style("a\nb\n", "x\ny\n".into()), "x\ny\n");
        assert_eq!(keep_file_style("a\r\nb\nc\n", "x\ny\n".into()), "x\ny\n", "mostly-LF files stay LF");
        assert_eq!(keep_file_style("\u{feff}a\n", "b\n".into()), "\u{feff}b\n");
        assert_eq!(keep_file_style("", "new\n".into()), "new\n");
        assert_eq!(keep_file_style("a\r\n", "already\r\nset\r\n".into()), "already\r\nset\r\n");
    }
}

#[cfg(test)]
mod file_op_tests {
    use super::*;

    #[test]
    fn creates_moves_and_deletes_folders_with_undo() {
        let base = std::env::temp_dir().join(format!("neru-ops-{}", uuid::Uuid::new_v4()));
        let root_dir = base.join("project");
        fs::create_dir_all(&root_dir).unwrap();
        if std::env::var_os("NERU_DATA_DIR").is_none() {
            // SAFETY: tests that use the data folder all point it at a temporary directory.
            unsafe { std::env::set_var("NERU_DATA_DIR", base.join("data")) };
        }
        let root = root_dir.canonicalize().unwrap();

        // Writing a file creates the folders it needs.
        let write = make_proposal(&root, "src/features/auth/login.ts", "export {}\n".into()).unwrap();
        apply_edit(&root, &write).unwrap();
        assert!(root.join("src/features/auth/login.ts").is_file());
        assert!(resolve_new(&root, "../outside.txt").is_err());

        // Rename a folder, then undo it.
        let rename = make_move_proposal(&root, "src/features", "src/modules").unwrap();
        let (moved, _) = apply_edit(&root, &rename).unwrap();
        assert!(root.join("src/modules/auth/login.ts").is_file());
        restore_checkpoint_file(&root, &moved).unwrap();
        assert!(root.join("src/features/auth/login.ts").is_file());

        // Delete a whole folder, then bring it back.
        let delete = make_delete_proposal(&root, "src/features").unwrap();
        assert_eq!(proposal_kind(&delete), "delete");
        let (deleted, _) = apply_edit(&root, &delete).unwrap();
        assert!(!root.join("src/features").exists());
        restore_checkpoint_file(&root, &deleted).unwrap();
        assert_eq!(fs::read_to_string(root.join("src/features/auth/login.ts")).unwrap(), "export {}\n");

        // An empty folder, created and undone.
        let folder = make_folder_proposal(&root, "assets/icons").unwrap();
        let (made, _) = apply_edit(&root, &folder).unwrap();
        assert!(root.join("assets/icons").is_dir());
        restore_checkpoint_file(&root, &made).unwrap();
        assert!(!root.join("assets/icons").exists());
        let _ = fs::remove_dir_all(&base);
    }
}

/// Removes a project from Neru's list (the sidebar). Files on disk are not touched, and its
/// sessions stay saved, so opening the folder again brings everything back.
#[tauri::command]
pub fn forget_project(path: String, state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let recent_path = data_dir()?.join("recent-projects.json");
    let mut recent: Vec<String> = fs::read_to_string(&recent_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let same = |item: &str| item == path || PathBuf::from(item).canonicalize().ok() == PathBuf::from(&path).canonicalize().ok();
    recent.retain(|item| !same(item));
    fs::write(&recent_path, serde_json::to_vec_pretty(&recent).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut root = state.root.lock().map_err(|e| e.to_string())?;
    if root.as_ref().is_some_and(|current| same(&current.to_string_lossy())) {
        *root = None;
        *state.active_session.lock().map_err(|e| e.to_string())? = None;
    }
    Ok(recent)
}

/// Shows a file or folder in the system file manager (Explorer, Finder), selected.
/// `path` is project-relative, or absolute for a project folder itself.
#[tauri::command]
pub fn reveal_path(path: String, state: State<'_, AppState>) -> Result<(), String> {
    let target = if Path::new(&path).is_absolute() {
        PathBuf::from(&path)
    } else {
        resolve_existing(&project_root(&state)?, &path)?
    };
    if !target.exists() {
        return Err(format!("{} no longer exists", target.display()));
    }
    // Explorer does not understand the \\?\ prefix that canonical paths carry on Windows.
    let shown = target.display().to_string().trim_start_matches(r"\\?\").to_string();
    #[cfg(windows)]
    let status = {
        use std::os::windows::process::CommandExt;
        std::process::Command::new("explorer.exe").raw_arg(format!("/select,\"{shown}\"")).spawn()
    };
    #[cfg(target_os = "macos")]
    let status = std::process::Command::new("open").arg("-R").arg(&shown).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let status = std::process::Command::new("xdg-open").arg(if target.is_dir() { target.clone() } else { target.parent().map(Path::to_path_buf).unwrap_or(target.clone()) }).spawn();
    status.map(|_| ()).map_err(|e| format!("Could not open the file manager: {e}"))
}

/// File-tree actions the user takes directly (right-click menu). Each keeps a checkpoint like
/// an agent edit. `action` is new_file, new_folder, rename, duplicate or delete.
#[tauri::command]
pub fn file_action(action: String, path: String, to: Option<String>, state: State<'_, AppState>) -> Result<String, String> {
    let root = project_root(&state)?;
    let clean = |value: &str| value.trim().replace('\\', "/").trim_matches('/').to_string();
    let path = clean(&path);
    let proposal = match action.as_str() {
        "new_file" => {
            if root.join(&path).exists() {
                return Err(format!("{path} already exists"));
            }
            make_proposal(&root, &path, String::new())?
        }
        "new_folder" => make_folder_proposal(&root, &path)?,
        "rename" => make_move_proposal(&root, &path, &clean(to.as_deref().ok_or("Enter a new name")?))?,
        "delete" => make_delete_proposal(&root, &path)?,
        "duplicate" => {
            let source = resolve_existing(&root, &path)?;
            if !source.is_file() {
                return Err("Only files can be duplicated".into());
            }
            let text = read_limited(&source)?;
            let (stem, ext) = match path.rsplit_once('.') {
                Some((stem, ext)) if !stem.ends_with('/') && !stem.is_empty() => (stem.to_string(), format!(".{ext}")),
                _ => (path.clone(), String::new()),
            };
            let copy = (1..100).map(|n| if n == 1 { format!("{stem} copy{ext}") } else { format!("{stem} copy {n}{ext}") }).find(|candidate| !root.join(candidate).exists()).ok_or("Too many copies")?;
            make_proposal(&root, &copy, text)?
        }
        _ => return Err("Unknown file action".into()),
    };
    let (checkpoint, _) = apply_edit(&root, &proposal)?;
    Ok(checkpoint)
}
