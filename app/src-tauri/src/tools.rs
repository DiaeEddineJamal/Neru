//! The agent's read-only tools (running on the project index) and the text edit engine.

use std::{
    fs,
    io::{BufRead, BufReader},
    path::Path,
    time::Duration,
};

use serde_json::Value;

use crate::{
    git, index, skills,
    workspace::{read_limited, relative_path, resolve_existing},
};

/// Longest unranged read, in lines. Like Cursor and Claude Code, big files are read in windows so a
/// single file cannot fill the context; the model asks for the next range when it needs it.
pub(crate) const READ_LINES: usize = 1_000;
/// Files one `read_files` call may return, and the characters they may add up to.
const READ_FILES_MAX: usize = 20;
const READ_FILES_CHARS: usize = 36_000;

pub(crate) const READ_TOOLS: &str = r#"[
    {"type":"function","function":{"name":"project_map","description":"Compact map of the project or one folder: folders, file names and the top-level symbols (functions, classes, types) of each file. Call it first on an unfamiliar codebase instead of listing folders one by one. Pass path for a folder's detail, or a file for its outline with line numbers.","parameters":{"type":"object","properties":{"path":{"type":"string","description":"Optional folder or file"}},"required":[]}}},
    {"type":"function","function":{"name":"list_directory","description":"List one project directory: folders first with file counts, files with line counts. Ignored files and build folders are hidden. Use a relative path, empty for the project root.","parameters":{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}}},
    {"type":"function","function":{"name":"read_file","description":"Read a project text file. Files over 1000 lines come back in windows: pass start_line and end_line (1-based) to read a specific range. Search first and read only the parts you need in large files.","parameters":{"type":"object","properties":{"path":{"type":"string"},"start_line":{"type":"integer"},"end_line":{"type":"integer"}},"required":["path"]}}},
    {"type":"function","function":{"name":"read_files","description":"Read several project files in one call, each optionally limited to a line range. Use this instead of many read_file calls whenever you need more than one file.","parameters":{"type":"object","properties":{"files":{"type":"array","items":{"type":"object","properties":{"path":{"type":"string"},"start_line":{"type":"integer"},"end_line":{"type":"integer"}},"required":["path"]}}},"required":["files"]}}},
    {"type":"function","function":{"name":"search_text","description":"Search file contents across the project (fast, respects .gitignore). Case-insensitive literal text by default; set regex for a pattern. Returns matching lines grouped by file. Narrow with glob, e.g. src/**/*.ts.","parameters":{"type":"object","properties":{"query":{"type":"string"},"regex":{"type":"boolean"},"case_sensitive":{"type":"boolean"},"whole_word":{"type":"boolean"},"glob":{"type":"string"},"max_results":{"type":"integer"}},"required":["query"]}}},
    {"type":"function","function":{"name":"find_symbol","description":"Find where a function, class, type, constant or heading is defined, by name (partial names work). Returns path:line.","parameters":{"type":"object","properties":{"name":{"type":"string"}},"required":["name"]}}},
    {"type":"function","function":{"name":"find_files","description":"Find project files by path pattern. * matches within a folder, ** across folders (e.g. src/**/*.tsx, *config*). Respects gitignore.","parameters":{"type":"object","properties":{"pattern":{"type":"string"}},"required":["pattern"]}}},
    {"type":"function","function":{"name":"git_status","description":"Inspect the current Git branch and changes.","parameters":{"type":"object","properties":{},"required":[]}}},
    {"type":"function","function":{"name":"git_diff","description":"Read the current staged or unstaged Git diff for one path or all changed files.","parameters":{"type":"object","properties":{"path":{"type":"string"},"staged":{"type":"boolean"}},"required":[]}}},
    {"type":"function","function":{"name":"read_skill","description":"Load a skill's instructions by name (see the Skills list). Pass file to load one of the reference documents the skill lists.","parameters":{"type":"object","properties":{"name":{"type":"string"},"file":{"type":"string","description":"Optional path of a reference file inside the skill, e.g. reference/polish.md"}},"required":["name"]}}},
    {"type":"function","function":{"name":"add_review_comment","description":"Pin a review comment on a file line. Use this for every code-review finding instead of only mentioning it in prose.","parameters":{"type":"object","properties":{"path":{"type":"string"},"line":{"type":"integer"},"text":{"type":"string"}},"required":["path","line","text"]}}},
    {"type":"function","function":{"name":"open_preview","description":"Open a URL in Neru's preview pane. Use http://127.0.0.1:PORT for the app you started, or an https documentation page.","parameters":{"type":"object","properties":{"url":{"type":"string"}},"required":["url"]}}}
]"#;

/// Tools that only read and never touch shared state, so a turn's calls to them can run together.
pub(crate) fn is_parallel_read(name: &str) -> bool {
    matches!(
        name,
        "list_directory" | "read_file" | "read_files" | "search_text" | "find_files" | "project_map" | "find_symbol" | "git_status" | "git_diff" | "read_skill" | "web_search" | "fetch_url"
    )
}

// ---------------------------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------------------------

/// Lines `start..=end` of a file (1-based), with a header when the result is only part of the file.
pub(crate) fn read_lines(text: &str, start: usize, end: Option<usize>) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len();
    let end = end.unwrap_or(start + READ_LINES - 1).min(total).min(start + 2 * READ_LINES - 1);
    if start == 1 && end >= total {
        return text.chars().take(40_000).collect();
    }
    if start > total {
        return format!("[The file has {total} lines; start_line {start} is past the end.]");
    }
    let body: String = lines[start - 1..end].join("\n").chars().take(40_000).collect();
    let more = if end < total { format!(" Call read_file again with start_line {} for more.", end + 1) } else { String::new() };
    format!("[Lines {start}-{end} of {total}.{more}]\n{body}")
}

/// A window of a file too big to load whole (logs, data dumps), read as a stream.
fn read_large_range(path: &Path, size: u64, start: usize, end: Option<usize>) -> Result<String, String> {
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let last = end.unwrap_or(start + READ_LINES - 1).min(start + 2 * READ_LINES - 1);
    let mut body = String::new();
    let mut at = 0usize;
    let mut reader = BufReader::new(file);
    let mut buffer = Vec::new();
    loop {
        buffer.clear();
        if reader.read_until(b'\n', &mut buffer).map_err(|e| e.to_string())? == 0 {
            break;
        }
        at += 1;
        if at < start {
            continue;
        }
        if at > last || body.len() > 40_000 {
            return Ok(format!("[Lines {start}-{}. The file is {} MB; call read_file again with start_line {at} for more.]\n{body}", at - 1, size / 1_000_000));
        }
        body.push_str(String::from_utf8_lossy(&buffer).trim_end_matches(['\r', '\n']));
        body.push('\n');
    }
    if at < start {
        return Ok(format!("[The file has {at} lines; start_line {start} is past the end.]"));
    }
    Ok(format!("[Lines {start}-{at}, the end of a {} MB file.]\n{body}", size / 1_000_000))
}

fn read_one(root: &Path, path: &str, start: usize, end: Option<usize>) -> Result<String, String> {
    let full = resolve_existing(root, path)?;
    let size = fs::metadata(&full).map(|m| m.len()).map_err(|e| e.to_string())?;
    if size > 1_000_000 && full.is_file() && size < 200_000_000 {
        return read_large_range(&full, size, start, end);
    }
    let text = read_limited(&full).map_err(|e| if e.contains("valid UTF-8") { format!("{path} is not a UTF-8 text file (binary?)") } else { e })?;
    Ok(read_lines(&text, start, end))
}

fn line_arg(args: &Value, key: &str) -> Option<usize> {
    args[key].as_u64().map(|n| n as usize)
}

fn read_files(root: &Path, args: &Value) -> Result<String, String> {
    let list = args["files"].as_array().or_else(|| args["paths"].as_array()).ok_or("Missing files")?;
    if list.is_empty() {
        return Err("files is empty".into());
    }
    let mut out = String::new();
    let mut done = 0;
    for item in list.iter().take(READ_FILES_MAX) {
        let (path, start, end) = match item {
            Value::String(path) => (path.clone(), 1, None),
            other => (other["path"].as_str().unwrap_or("").to_string(), line_arg(other, "start_line").map_or(1, |n| n.max(1)), line_arg(other, "end_line")),
        };
        if path.is_empty() {
            out.push_str("=== (missing path) ===\nError: each item needs a path\n\n");
            continue;
        }
        let body = read_one(root, &path, start, end).unwrap_or_else(|e| format!("Error: {e}"));
        if out.len() + body.len() > READ_FILES_CHARS && done > 0 {
            let left: Vec<String> = list.iter().skip(done).filter_map(|item| item.as_str().map(str::to_string).or_else(|| item["path"].as_str().map(str::to_string))).collect();
            out.push_str(&format!("[Output limit reached after {done} files. Read these next: {}]", left.join(", ")));
            return Ok(out);
        }
        out.push_str(&format!("=== {path} ===\n{body}\n\n"));
        done += 1;
    }
    if list.len() > READ_FILES_MAX {
        out.push_str(&format!("[Only the first {READ_FILES_MAX} files were read; ask for the other {} again.]", list.len() - READ_FILES_MAX));
    }
    Ok(out.trim_end().to_string())
}

fn list_directory(root: &Path, path: &str) -> Result<String, String> {
    let dir = resolve_existing(root, path)?;
    if !dir.is_dir() {
        return Err("Not a directory".into());
    }
    let handle = index::ready(root, Duration::from_secs(5));
    let mut folders: Vec<(String, usize)> = Vec::new();
    let mut files: Vec<(String, String)> = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|e| e.to_string())?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        if is_dir && index::is_skipped_dir(&name) {
            continue;
        }
        let rel = relative_path(root, &entry.path());
        match (&handle, is_dir) {
            (Some(handle), true) => {
                let (_, count, _) = handle.describe(&rel);
                let empty = count == 0 && fs::read_dir(entry.path()).is_ok_and(|mut d| d.next().is_none());
                if count > 0 || empty {
                    folders.push((name, count));
                }
            }
            (Some(handle), false) => {
                let (lines, _, known) = handle.describe(&rel);
                if known {
                    files.push((name, lines.filter(|&n| n > 0).map_or_else(|| entry.metadata().map(|m| format!("{} KB", m.len().div_ceil(1024))).unwrap_or_default(), |n| format!("{n} line{}", if n == 1 { "" } else { "s" }))));
                }
            }
            (None, true) => folders.push((name, 0)),
            (None, false) => files.push((name, entry.metadata().map(|m| format!("{} KB", m.len().div_ceil(1024))).unwrap_or_default())),
        }
    }
    folders.sort_by_key(|(name, _)| name.to_lowercase());
    files.sort_by_key(|(name, _)| name.to_lowercase());
    let total = folders.len() + files.len();
    let mut lines: Vec<String> = folders
        .iter()
        .map(|(name, count)| if handle.is_some() && *count > 0 { format!("{name}/ ({count} files)") } else { format!("{name}/") })
        .chain(files.iter().map(|(name, size)| if size.is_empty() { name.clone() } else { format!("{name} ({size})") }))
        .collect();
    lines.truncate(300);
    if total > 300 {
        lines.push(format!("… {} more entries; use find_files or project_map with a narrower path", total - 300));
    }
    Ok(if lines.is_empty() { "(empty)".into() } else { lines.join("\n") })
}

fn search_text(root: &Path, args: &Value) -> Result<String, String> {
    let text = args["query"].as_str().ok_or("Missing query")?.trim();
    if text.chars().count() < 2 {
        return Err("Query is too short".into());
    }
    let mut query = index::Query::literal(text, args["max_results"].as_u64().map_or(40, |n| n.clamp(1, 200) as usize));
    query.regex = args["regex"].as_bool().unwrap_or(false);
    query.case_sensitive = args["case_sensitive"].as_bool().unwrap_or(false);
    query.whole_word = args["whole_word"].as_bool().unwrap_or(false);
    query.glob = args["glob"].as_str().map(str::trim).filter(|g| !g.is_empty()).map(str::to_string);
    let output = index::search(root, &query)?;
    if output.hits.is_empty() {
        return Ok(format!("No matches for “{text}”{}.", if output.total_files > 0 { format!(" in {} files", output.total_files) } else { String::new() }));
    }
    let mut body = String::new();
    let mut current = "";
    let mut files = 0;
    for hit in &output.hits {
        if hit.path != current {
            current = &hit.path;
            files += 1;
            body.push_str(&format!("{current}\n"));
        }
        let line: String = hit.preview.chars().take(160).collect();
        body.push_str(&format!("  {}: {}\n", hit.line, line));
    }
    let more = if output.truncated { format!(" (stopped at the limit of {}; narrow with glob, regex or a more specific query)", query.limit) } else { String::new() };
    Ok(format!("{} matches in {files} files{more}\n{body}", output.hits.len()))
}

fn find_files(root: &Path, args: &Value) -> Result<String, String> {
    let pattern = args["pattern"].as_str().ok_or("Missing pattern")?.trim();
    if pattern.is_empty() {
        return Err("Pattern is empty".into());
    }
    let (matches, count) = match index::ready(root, Duration::from_secs(30)) {
        Some(handle) => handle.files_matching(pattern, 200),
        None => {
            let mut found = Vec::new();
            for entry in ignore::WalkBuilder::new(root).follow_links(false).build().flatten() {
                if entry.file_type().is_some_and(|kind| kind.is_file()) {
                    let path = relative_path(root, entry.path());
                    if index::path_matches(pattern, &path) {
                        found.push(path);
                    }
                }
            }
            found.sort();
            let count = found.len();
            found.truncate(200);
            (found, count)
        }
    };
    Ok(if matches.is_empty() {
        "No files match.".into()
    } else if count > matches.len() {
        format!("{}\n… {} more; use a narrower pattern", matches.join("\n"), count - matches.len())
    } else {
        matches.join("\n")
    })
}

fn project_map(root: &Path, args: &Value) -> Result<String, String> {
    let handle = index::ready(root, Duration::from_secs(30)).ok_or("The project index is still building; try list_directory")?;
    let path = args["path"].as_str().unwrap_or("").trim().trim_matches(['/', '\\']).replace('\\', "/");
    let path = if path == "." { String::new() } else { path };
    if !path.is_empty() {
        if let Some(outline) = handle.outline(&path) {
            return Ok(outline);
        }
    }
    let budget = args["max_chars"].as_u64().map_or(if path.is_empty() { 9_000 } else { 14_000 }, |n| n.clamp(1_500, 30_000) as usize);
    let map = handle.map(&path, budget);
    if map.is_empty() {
        return Err(format!("Nothing indexed under “{path}”. Check the path with list_directory."));
    }
    Ok(map)
}

fn find_symbol(root: &Path, args: &Value) -> Result<String, String> {
    let name = args["name"].as_str().or_else(|| args["query"].as_str()).ok_or("Missing name")?.trim();
    if name.is_empty() {
        return Err("Name is empty".into());
    }
    let handle = index::ready(root, Duration::from_secs(30)).ok_or("The project index is still building; try search_text")?;
    let found = handle.symbols(name, 40);
    if found.is_empty() {
        return Ok(format!("No symbol named like “{name}”. Try search_text for usages or other spellings."));
    }
    Ok(found.iter().map(|(path, symbol)| format!("{path}:{} {} {}", symbol.line, symbol.kind, symbol.name)).collect::<Vec<_>>().join("\n"))
}

pub(crate) fn execute_read_tool(root: &Path, name: &str, args: &Value) -> Result<String, String> {
    match name {
        "list_directory" => list_directory(root, args["path"].as_str().ok_or("Missing path")?),
        "read_file" => {
            let path = args["path"].as_str().ok_or("Missing path")?;
            read_one(root, path, line_arg(args, "start_line").map_or(1, |n| n.max(1)), line_arg(args, "end_line"))
        }
        "read_files" => read_files(root, args),
        "search_text" => search_text(root, args),
        "find_files" => find_files(root, args),
        "project_map" => project_map(root, args),
        "find_symbol" => find_symbol(root, args),
        "git_status" => serde_json::to_string(&git::status(root)?).map_err(|e| e.to_string()),
        "git_diff" => {
            let path = args["path"].as_str().unwrap_or("");
            let staged = args["staged"].as_bool().unwrap_or(false);
            let diff = match (staged, path.is_empty()) {
                (true, true) => git::git(root, &["diff", "--cached", "--"])?,
                (true, false) => git::git(root, &["diff", "--cached", "--", path])?,
                (false, true) => git::git(root, &["diff", "--"])?,
                (false, false) => git::git(root, &["diff", "--", path])?,
            };
            Ok(diff.chars().take(40_000).collect())
        }
        "read_skill" => {
            let name = args["name"].as_str().ok_or("Missing skill name")?;
            skills::read(root, name, args["file"].as_str())
        }
        _ => Err("Unknown tool".into()),
    }
}

// ---------------------------------------------------------------------------------------------
// Editing
// ---------------------------------------------------------------------------------------------

/// One replacement of a `propose_edit` call.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EditSpec {
    pub old: String,
    pub new: String,
    pub all: bool,
}

/// The replacements a `propose_edit` call asks for: an `edits` list, or the single old/new pair.
pub(crate) fn parse_edits(args: &Value) -> Result<Vec<EditSpec>, String> {
    if let Some(list) = args["edits"].as_array().filter(|list| !list.is_empty()) {
        return list
            .iter()
            .enumerate()
            .map(|(i, item)| {
                Ok(EditSpec {
                    old: item["old_string"].as_str().ok_or(format!("edit {}: missing old_string", i + 1))?.to_string(),
                    new: item["new_string"].as_str().ok_or(format!("edit {}: missing new_string", i + 1))?.to_string(),
                    all: item["replace_all"].as_bool().unwrap_or(false),
                })
            })
            .collect();
    }
    Ok(vec![EditSpec {
        old: args["old_string"].as_str().ok_or("Missing old_string")?.to_string(),
        new: args["new_string"].as_str().ok_or("Missing new_string")?.to_string(),
        all: args["replace_all"].as_bool().unwrap_or(false),
    }])
}

/// Applies several replacements in order to one file's text; the result is all or nothing.
pub(crate) fn apply_edits(original: &str, edits: &[EditSpec]) -> Result<String, String> {
    let mut text = original.to_string();
    for (i, edit) in edits.iter().enumerate() {
        text = apply_snippet(&text, &edit.old, &edit.new, edit.all).map_err(|e| if edits.len() > 1 { format!("edit {} of {}: {e}. No edits were applied", i + 1, edits.len()) } else { e })?;
    }
    Ok(text)
}

fn normalize_line(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn indent_of(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

/// Replaces `old` with `new` in `original`, requiring a unique match unless `all` is set.
/// Falls back to CRLF line endings when the file uses them and the snippet does not, and then to
/// a match that ignores indentation and spacing differences (the usual way models miscopy code).
pub(crate) fn apply_snippet(original: &str, old: &str, new: &str, all: bool) -> Result<String, String> {
    if old.is_empty() {
        return Err("old_string is empty; use propose_write_file to create a file".into());
    }
    let (old_exact, new_exact) = if !original.contains(old) && original.contains("\r\n") && !old.contains('\r') {
        (old.replace('\n', "\r\n"), new.replace('\n', "\r\n"))
    } else {
        (old.to_string(), new.to_string())
    };
    match original.matches(old_exact.as_str()).count() {
        0 => {}
        1 => return Ok(original.replacen(old_exact.as_str(), &new_exact, 1)),
        _ if all => return Ok(original.replace(old_exact.as_str(), &new_exact)),
        count => return Err(format!("old_string matches {count} places. Include more surrounding lines to make it unique, or set replace_all")),
    }
    if let Some(result) = fuzzy_replace(original, old, new, all)? {
        return Ok(result);
    }
    Err(not_found_message(original, old))
}

/// The whitespace-tolerant fallback of `apply_snippet`.
fn fuzzy_replace(original: &str, old: &str, new: &str, all: bool) -> Result<Option<String>, String> {
    let mut old_lines: Vec<&str> = old.lines().collect();
    let leading_blank = old_lines.iter().take_while(|l| l.trim().is_empty()).count();
    old_lines.drain(..leading_blank);
    while old_lines.last().is_some_and(|l| l.trim().is_empty()) {
        old_lines.pop();
    }
    if old_lines.is_empty() {
        return Ok(None);
    }
    let wanted: Vec<String> = old_lines.iter().map(|l| normalize_line(l)).collect();
    // (start, end without the line break) of each line of the file.
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut start = 0;
    for piece in original.split_inclusive('\n') {
        let content = piece.strip_suffix('\n').map_or(piece, |p| p.strip_suffix('\r').unwrap_or(p));
        spans.push((start, start + content.len()));
        start += piece.len();
    }
    let have: Vec<String> = spans.iter().map(|&(a, b)| normalize_line(&original[a..b])).collect();
    let n = wanted.len();
    if have.len() < n {
        return Ok(None);
    }
    let matches: Vec<usize> = (0..=have.len() - n).filter(|&i| have[i] == wanted[0] && (1..n).all(|k| have[i + k] == wanted[k])).collect();
    if matches.is_empty() {
        return Ok(None);
    }
    if matches.len() > 1 && !all {
        return Err(format!("old_string matches {} places when whitespace is ignored. Include more surrounding lines to make it unique, or set replace_all", matches.len()));
    }
    let crlf = original.contains("\r\n");
    let old_indent = old_lines.iter().find(|l| !l.trim().is_empty()).map_or("", |l| indent_of(l)).to_string();
    let mut body = new;
    if old.starts_with('\n') || old.starts_with("\r\n") {
        body = body.strip_prefix("\r\n").or_else(|| body.strip_prefix('\n')).unwrap_or(body);
    }
    if old.ends_with('\n') {
        body = body.strip_suffix("\r\n").or_else(|| body.strip_suffix('\n')).unwrap_or(body);
    }
    let mut result = original.to_string();
    for &i in matches.iter().rev() {
        let (a, _) = spans[i];
        let (_, mut b) = spans[i + n - 1];
        if body.is_empty() && i + n < spans.len() {
            // Deleting whole lines takes their line break too.
            b = spans[i + n].0;
        }
        let file_indent = indent_of(&original[a..spans[i].1]).to_string();
        let adjusted: Vec<String> = body
            .lines()
            .map(|line| {
                if line.trim().is_empty() {
                    String::new()
                } else if old_indent == file_indent {
                    line.to_string()
                } else if let Some(rest) = line.strip_prefix(old_indent.as_str()) {
                    format!("{file_indent}{rest}")
                } else {
                    line.to_string()
                }
            })
            .collect();
        let replacement = if body.is_empty() { String::new() } else { adjusted.join(if crlf { "\r\n" } else { "\n" }) };
        result.replace_range(a..b, &replacement);
    }
    Ok(Some(result))
}

/// Explains a failed match and shows the closest text in the file, so the model can fix its snippet.
fn not_found_message(original: &str, old: &str) -> String {
    let base = "old_string was not found. Read the file again and copy the snippet exactly, including whitespace";
    let Some(first) = old.lines().map(normalize_line).find(|l| l.len() >= 4) else { return base.into() };
    let lines: Vec<&str> = original.lines().collect();
    let mut best: Option<(f32, usize)> = None;
    for (i, line) in lines.iter().enumerate().take(6_000) {
        let candidate = normalize_line(line);
        if candidate.len() < 4 {
            continue;
        }
        let ratio = similar::TextDiff::from_chars(first.as_str(), candidate.as_str()).ratio();
        if best.is_none_or(|(score, _)| ratio > score) {
            best = Some((ratio, i));
        }
    }
    match best {
        Some((ratio, at)) if ratio >= 0.6 => {
            let count = old.lines().count().clamp(1, 4);
            let shown: Vec<String> = lines[at..(at + count).min(lines.len())].iter().enumerate().map(|(k, line)| format!("{}| {}", at + k + 1, line)).collect();
            format!("{base}. The closest text in the file starts at line {}:\n{}", at + 1, shown.join("\n"))
        }
        _ => format!("{base}. Nothing similar was found; the code may have changed, so read the file again"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn snippet_edits_need_a_unique_match() {
        assert_eq!(apply_snippet("a b a", "b", "c", false).unwrap(), "a c a");
        assert!(apply_snippet("a b a", "a", "c", false).unwrap_err().contains("2 places"));
        assert_eq!(apply_snippet("a b a", "a", "c", true).unwrap(), "c b c");
        assert!(apply_snippet("abc", "zz", "y", false).is_err());
        assert_eq!(apply_snippet("one\r\ntwo\r\n", "one\ntwo", "1\n2", false).unwrap(), "1\r\n2\r\n");
    }

    #[test]
    fn edits_tolerate_indentation_and_trailing_space() {
        let file = "fn main() {\n    let a = 1;   \n    if a > 0 {\n        run();\n    }\n}\n";
        // Model dropped the indentation and the trailing spaces.
        let edited = apply_snippet(file, "let a = 1;\nif a > 0 {\n    run();\n}", "let a = 2;\nif a > 0 {\n    run_twice();\n}", false).unwrap();
        assert_eq!(edited, "fn main() {\n    let a = 2;\n    if a > 0 {\n        run_twice();\n    }\n}\n");
        // Wrong indentation the other way around, CRLF file.
        let crlf = "if (x) {\r\n\tgo();\r\n}\r\n";
        let edited = apply_snippet(crlf, "  if (x) {\n    go();\n  }", "  if (x) {\n    stop();\n  }", false).unwrap();
        assert_eq!(edited, "if (x) {\r\n  stop();\r\n}\r\n", "{edited:?}");
    }

    #[test]
    fn fuzzy_matches_still_have_to_be_unique() {
        let file = "  foo();\n  foo();\n";
        let error = apply_snippet(file, "foo();\nfoo();\nfoo();", "x", false);
        assert!(error.is_err());
        let error = apply_snippet("    call();\n  call();\n", "call();", "x();", false).unwrap_err();
        assert!(error.contains("2 places"), "{error}");
        assert_eq!(apply_snippet("    call();\n  call();\n", "call();", "x();", true).unwrap(), "    x();\n  x();\n");
    }

    #[test]
    fn failed_edits_show_the_closest_lines() {
        let file = "import a from 'a'\nexport function renderTree(items) {\n  return items\n}\n";
        let error = apply_snippet(file, "export function renderTre(items) {\n  return item\n}", "x", false).unwrap_err();
        assert!(error.contains("line 2") && error.contains("renderTree"), "{error}");
        let none = apply_snippet(file, "completely different words here", "x", false).unwrap_err();
        assert!(none.contains("Nothing similar"), "{none}");
    }

    #[test]
    fn several_edits_apply_together_or_not_at_all() {
        let file = "one\ntwo\nthree\n";
        let args = json!({"path":"a","edits":[{"old_string":"one","new_string":"1"},{"old_string":"three","new_string":"3"}]});
        let edits = parse_edits(&args).unwrap();
        assert_eq!(apply_edits(file, &edits).unwrap(), "1\ntwo\n3\n");
        let bad = parse_edits(&json!({"edits":[{"old_string":"one","new_string":"1"},{"old_string":"missing text","new_string":"x"}]})).unwrap();
        let error = apply_edits(file, &bad).unwrap_err();
        assert!(error.contains("edit 2 of 2") && error.contains("No edits were applied"), "{error}");
        assert_eq!(parse_edits(&json!({"old_string":"a","new_string":""})).unwrap()[0].new, "");
        assert!(parse_edits(&json!({"path":"a"})).is_err());
    }

    #[test]
    fn reads_windows_and_batches_files() {
        let root = std::env::temp_dir().join(format!("neru-tools-{}", uuid::Uuid::new_v4())).join("p");
        fs::create_dir_all(root.join("src")).unwrap();
        let root = root.canonicalize().unwrap();
        fs::write(root.join("src/a.txt"), (1..=3000).map(|n| format!("line {n}\n")).collect::<String>()).unwrap();
        fs::write(root.join("src/b.txt"), "hello\n").unwrap();
        let window = execute_read_tool(&root, "read_file", &json!({"path":"src/a.txt","start_line":10,"end_line":12})).unwrap();
        assert_eq!(window, "[Lines 10-12 of 3000. Call read_file again with start_line 13 for more.]\nline 10\nline 11\nline 12");
        let both = execute_read_tool(&root, "read_files", &json!({"files":[{"path":"src/b.txt"},{"path":"src/a.txt","start_line":2999},"src/none.txt"]})).unwrap();
        assert!(both.contains("=== src/b.txt ===\nhello") && both.contains("line 3000") && both.contains("=== src/none.txt ===\nError"), "{both}");
        let listing = execute_read_tool(&root, "list_directory", &json!({"path":""})).unwrap();
        assert_eq!(listing, "src/ (2 files)");
        let inside = execute_read_tool(&root, "list_directory", &json!({"path":"src"})).unwrap();
        assert_eq!(inside, "a.txt (3000 lines)\nb.txt (1 line)");
        let found = execute_read_tool(&root, "search_text", &json!({"query":"line 2999"})).unwrap();
        assert!(found.starts_with("1 matches in 1 files") && found.contains("src/a.txt\n  2999: line 2999"), "{found}");
        assert_eq!(execute_read_tool(&root, "find_files", &json!({"pattern":"*.txt"})).unwrap(), "src/a.txt\nsrc/b.txt");
        assert!(execute_read_tool(&root, "read_file", &json!({"path":"../x"})).is_err());
        let _ = fs::remove_dir_all(root.parent().unwrap());
    }

    #[test]
    fn wildcards_match_paths() {
        use crate::index::path_matches;
        assert!(path_matches("src/**/*.tsx", "src/components/neru/App.tsx"));
        assert!(path_matches("src/**/*.tsx", "src/App.tsx"));
        assert!(!path_matches("src/*.tsx", "src/components/App.tsx"));
        assert!(path_matches("*.rs", "src-tauri/src/agent.rs"));
        assert!(path_matches("config", "vite.config.ts"));
        assert!(!path_matches("config", "src/configure/a.ts"));
    }
}
