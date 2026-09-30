//! Custom slash commands: Markdown prompt templates in `.neru/commands/` (or `.claude/commands/`) in
//! the project, and in Neru's data folder for every project, in Claude Code's format. Subfolders
//! namespace commands (`frontend/test.md` is `/frontend:test`). Front matter may set
//! `description`, `argument-hint`, `model` and `allowed-tools`. [`expand`] fills in `$ARGUMENTS`,
//! `$1`…`$9` and `@path` file references.

use std::{fs, path::Path};

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::{AppState, workspace::{data_dir, project_root}};

#[derive(Serialize, Deserialize, Debug, PartialEq, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    pub template: String,
    /// "project", "personal" or "skill".
    pub source: String,
    /// Front matter `argument-hint`, shown after the name, e.g. `[issue-number]`.
    #[serde(default)]
    pub argument_hint: String,
    /// Front matter `model`: the model the command prefers, if any.
    #[serde(default)]
    pub model: String,
    /// Front matter `allowed-tools`, as written (e.g. `Bash(git status:*)`).
    #[serde(default)]
    pub allowed_tools: Vec<String>,
}

/// What a command file's front matter says.
#[derive(Default, Debug, PartialEq)]
struct Meta {
    description: String,
    argument_hint: String,
    model: String,
    allowed_tools: Vec<String>,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 40 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn unquote(value: &str) -> String {
    let value = value.trim();
    let value = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\''))).unwrap_or(value);
    value.trim().to_string()
}

/// `a, Bash(git add:*), [b, c]` into items, keeping commas inside parentheses.
fn split_list(value: &str) -> Vec<String> {
    let value = value.trim();
    let value = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')).unwrap_or(value);
    let (mut items, mut current, mut depth) = (Vec::new(), String::new(), 0i32);
    for c in value.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if c == ',' && depth <= 0 {
            items.push(unquote(&current));
            current.clear();
        } else {
            current.push(c);
        }
    }
    items.push(unquote(&current));
    items.retain(|item| !item.is_empty());
    items
}

/// Splits optional `---` front matter from the template body.
fn parse_meta(text: &str) -> (Meta, String) {
    let text = text.trim_start_matches('\u{feff}');
    if let Some(rest) = text.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            let front = &rest[..end];
            let body = rest[end + 4..].trim_start_matches(['\r', '\n']).to_string();
            let mut meta = Meta::default();
            let mut list_key: Option<&str> = None;
            for line in front.lines() {
                let trimmed = line.trim();
                // YAML block lists: `allowed-tools:` followed by `- Bash(...)` lines.
                if let (Some("allowed-tools"), Some(item)) = (list_key, trimmed.strip_prefix("- ")) {
                    meta.allowed_tools.push(unquote(item));
                    continue;
                }
                list_key = None;
                let Some((key, value)) = trimmed.split_once(':') else { continue };
                match key.trim() {
                    "description" => meta.description = unquote(value),
                    "argument-hint" => meta.argument_hint = unquote(value),
                    "model" => meta.model = unquote(value),
                    "allowed-tools" if value.trim().is_empty() => list_key = Some("allowed-tools"),
                    "allowed-tools" => meta.allowed_tools = split_list(value),
                    _ => {}
                }
            }
            return (meta, body);
        }
    }
    let description = text
        .lines()
        .map(|line| line.trim().trim_start_matches('#').trim())
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .chars()
        .take(120)
        .collect();
    (Meta { description, ..Meta::default() }, text.to_string())
}

#[cfg(test)]
fn parse(text: &str) -> (String, String) {
    let (meta, body) = parse_meta(text);
    (meta.description, body)
}

fn read_folder(folder: &Path, source: &str, into: &mut Vec<SlashCommand>) {
    read_namespace(folder, "", source, into, 0);
}

fn read_namespace(folder: &Path, prefix: &str, source: &str, into: &mut Vec<SlashCommand>, depth: usize) {
    let Ok(entries) = fs::read_dir(folder) else { return };
    let mut entries: Vec<_> = entries.flatten().map(|entry| entry.path()).collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            let Some(space) = path.file_name().map(|name| name.to_string_lossy().to_lowercase()) else { continue };
            if depth < 3 && valid_name(&space) {
                read_namespace(&path, &format!("{prefix}{space}:"), source, into, depth + 1);
            }
            continue;
        }
        if path.extension().is_none_or(|ext| ext != "md") {
            continue;
        }
        let Some(stem) = path.file_stem().map(|stem| stem.to_string_lossy().to_lowercase()) else { continue };
        let name = format!("{prefix}{stem}");
        if !valid_name(&stem) || name.len() > 80 || into.iter().any(|command| command.name == name) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else { continue };
        if text.len() > 40_000 {
            continue;
        }
        let (meta, template) = parse_meta(&text);
        into.push(SlashCommand {
            name,
            description: meta.description,
            template,
            source: source.into(),
            argument_hint: meta.argument_hint,
            model: meta.model,
            allowed_tools: meta.allowed_tools,
        });
    }
}

/// Splits command arguments like a shell: whitespace separates, and "double" or 'single'
/// quotes keep spaces inside one argument.
pub fn split_arguments(args: &str) -> Vec<String> {
    let (mut items, mut current, mut quote, mut started) = (Vec::new(), String::new(), None::<char>, false);
    for c in args.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => current.push(c),
            None if c == '"' || c == '\'' => {
                quote = Some(c);
                started = true;
            }
            None if c.is_whitespace() => {
                if started {
                    items.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            None => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        items.push(current);
    }
    items
}

const INCLUDE_FILE_CHARS: usize = 20_000;
const INCLUDE_TOTAL_CHARS: usize = 60_000;

/// Fills in a command template: `$ARGUMENTS` is everything after the command name, `$1`…`$9` the
/// quote-aware positional arguments (empty when missing), and each `@path` naming a file in the
/// project `root` has that file's contents attached below the prompt. A template with no
/// placeholders gets the arguments appended after a blank line.
pub fn expand(template: &str, args: &str, root: &Path) -> String {
    let args = args.trim();
    let positional = split_arguments(args);
    let mut text = String::with_capacity(template.len() + args.len());
    let mut placeholders = false;
    let mut chars = template.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if c == '$' {
            if template[at..].starts_with("$ARGUMENTS") {
                text.push_str(args);
                placeholders = true;
                for _ in 0.."ARGUMENTS".len() {
                    chars.next();
                }
                continue;
            }
            let digit = template[at + 1..].chars().next().filter(|d| ('1'..='9').contains(d));
            let followed = template[at + 1..].chars().nth(1).is_some_and(|next| next.is_ascii_digit());
            if let (Some(digit), false) = (digit, followed) {
                let index = digit as usize - '1' as usize;
                text.push_str(positional.get(index).map(String::as_str).unwrap_or(""));
                placeholders = true;
                chars.next();
                continue;
            }
        }
        text.push(c);
    }
    if !placeholders && !args.is_empty() {
        text = if text.trim().is_empty() { args.to_string() } else { format!("{}\n\n{args}", text.trim_end()) };
    }
    text.push_str(&included_files(&text, root));
    text
}

/// The contents of files referenced as `@path` in `text`, as blocks to append to the prompt.
fn included_files(text: &str, root: &Path) -> String {
    let Ok(root) = root.canonicalize() else { return String::new() };
    let mut out = String::new();
    let mut seen: Vec<String> = Vec::new();
    let mut in_fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
        }
        if in_fence {
            continue;
        }
        for word in line.split_whitespace() {
            let Some(path) = word.strip_prefix('@') else { continue };
            let path = path.trim_end_matches(['.', ',', ';', ':', ')', '!', '?', '"', '\'', '`']);
            if path.is_empty() || seen.iter().any(|item| item == path) || seen.len() >= 20 {
                continue;
            }
            let Ok(file) = crate::workspace::resolve_existing(&root, path) else { continue };
            if !file.is_file() {
                continue;
            }
            let Ok(body) = crate::workspace::read_limited(&file) else { continue };
            let remaining = INCLUDE_TOTAL_CHARS.saturating_sub(out.chars().count());
            if remaining < 200 {
                break;
            }
            seen.push(path.to_string());
            let shown = path.replace('\\', "/");
            out.push_str(&format!("\n\nContents of {shown}:\n```\n{}\n```", body.chars().take(INCLUDE_FILE_CHARS.min(remaining)).collect::<String>().trim_end()));
        }
    }
    out
}

#[tauri::command]
pub fn list_commands(state: State<'_, AppState>) -> Vec<SlashCommand> {
    let mut commands = Vec::new();
    if let Ok(root) = project_root(&state) {
        read_folder(&root.join(".neru").join("commands"), "project", &mut commands);
        read_folder(&root.join(".claude").join("commands"), "project", &mut commands);
    }
    if let Ok(dir) = data_dir() {
        read_folder(&dir.join("commands"), "personal", &mut commands);
    }
    if let Ok(root) = project_root(&state) {
        for (name, description, template) in crate::skills::slash_entries(&root) {
            if commands.iter().any(|command| command.name == name) {
                continue;
            }
            commands.push(SlashCommand { name, description, template, source: "skill".into(), ..SlashCommand::default() });
        }
    }
    commands.sort_by(|a, b| a.name.cmp(&b.name));
    commands
}

/// The prompt a custom command sends: its template expanded with `args` (see [`expand`]).
#[tauri::command]
pub fn expand_command(name: String, args: String, state: State<'_, AppState>) -> Result<String, String> {
    let name = name.trim().trim_start_matches('/').to_lowercase();
    let command = list_commands(state.clone()).into_iter().find(|command| command.name == name).ok_or_else(|| format!("No command named /{name}"))?;
    let root = project_root(&state).unwrap_or_default();
    Ok(expand(&command.template, &args, &root))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_front_matter_and_plain_files() {
        let (description, body) = parse("---\ndescription: \"Write release notes\"\n---\nSummarize $ARGUMENTS");
        assert_eq!(description, "Write release notes");
        assert_eq!(body, "Summarize $ARGUMENTS");
        let (description, _) = parse("# Review the diff\nLook for bugs");
        assert_eq!(description, "Review the diff");
        assert!(valid_name("fix-tests"));
        assert!(!valid_name("../x"));
    }

    #[test]
    fn parses_claude_front_matter() {
        let (meta, body) = parse_meta("---\ndescription: Create a commit\nargument-hint: [message]\nmodel: claude-3-5-haiku\nallowed-tools: Bash(git add:*), Bash(git status:*), Read\n---\nCommit: $ARGUMENTS");
        assert_eq!(meta.description, "Create a commit");
        assert_eq!(meta.argument_hint, "[message]");
        assert_eq!(meta.model, "claude-3-5-haiku");
        assert_eq!(meta.allowed_tools, vec!["Bash(git add:*)", "Bash(git status:*)", "Read"]);
        assert_eq!(body, "Commit: $ARGUMENTS");
        let (meta, _) = parse_meta("---\nallowed-tools:\n  - Bash(npm test:*)\n  - \"Edit\"\ndescription: 'x'\n---\nbody");
        assert_eq!(meta.allowed_tools, vec!["Bash(npm test:*)", "Edit"]);
        assert_eq!(meta.description, "x");
    }

    #[test]
    fn subfolders_namespace_commands() {
        let root = std::env::temp_dir().join(format!("neru-commands-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("frontend")).unwrap();
        fs::write(root.join("review.md"), "Review").unwrap();
        fs::write(root.join("frontend/Test.md"), "---\nargument-hint: [file]\n---\nTest $1").unwrap();
        let mut commands = Vec::new();
        read_folder(&root, "project", &mut commands);
        let names: Vec<&str> = commands.iter().map(|command| command.name.as_str()).collect();
        assert!(names.contains(&"review") && names.contains(&"frontend:test"), "{names:?}");
        assert_eq!(commands.iter().find(|command| command.name == "frontend:test").unwrap().argument_hint, "[file]");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn splits_arguments_like_a_shell() {
        assert_eq!(split_arguments(r#"123 "high priority" 'a b' c"#), vec!["123", "high priority", "a b", "c"]);
        assert_eq!(split_arguments("  "), Vec::<String>::new());
        assert_eq!(split_arguments(r#"x "" y"#), vec!["x", "", "y"]);
    }

    #[test]
    fn expands_arguments_positions_and_files() {
        let root = std::env::temp_dir().join(format!("neru-expand-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "fn main() {}").unwrap();
        assert_eq!(expand("Fix issue #$1 with priority $2: $ARGUMENTS", r#"42 "very high""#, &root), r#"Fix issue #42 with priority very high: 42 "very high""#);
        assert_eq!(expand("Only $3 and $1.", "a", &root), "Only  and a.");
        assert_eq!(expand("Costs $10 total", "", &root), "Costs $10 total");
        assert_eq!(expand("Review the diff", "carefully", &root), "Review the diff\n\ncarefully");
        assert_eq!(expand("Review the diff", "", &root), "Review the diff");
        let text = expand("Explain @src/lib.rs.", "", &root);
        assert!(text.starts_with("Explain @src/lib.rs."));
        assert!(text.contains("Contents of src/lib.rs:\n```\nfn main() {}\n```"));
        let from_args = expand("Look at $ARGUMENTS", "@src/lib.rs @missing.rs @../outside", &root);
        assert_eq!(from_args.matches("Contents of").count(), 1);
        let _ = fs::remove_dir_all(root);
    }
}
