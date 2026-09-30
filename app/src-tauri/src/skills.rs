//! Skills are markdown workflows the model can load by name, and the user can run with `/name`.
//! A skill is a `SKILL.md` file in `.neru/skills/<name>/`, `.claude/skills/<name>/`, Neru's
//! personal skills folder, or the skills that ship with Neru (design, engineering method,
//! security; see skills/NOTICE.md). Like Claude, only names and descriptions go in the prompt;
//! the model loads a skill's text, and any reference file it names, when a task matches.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::Serialize;
use tauri::State;

use crate::{AppState, workspace::data_dir};

#[derive(Clone, Debug, PartialEq)]
struct Skill {
    name: String,
    description: String,
    body: String,
    /// "project" (.neru/skills), "claude" (.claude/skills), "personal" (added in Settings) or
    /// "built-in" (ships with Neru).
    source: &'static str,
    folder: PathBuf,
}

/// A skill as Settings lists it.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SkillView {
    pub name: String,
    pub description: String,
    pub source: String,
    pub path: String,
    pub chars: usize,
    /// Built-in skills can be switched off; the rest are removed instead.
    pub enabled: bool,
}

static BUNDLED: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Where the skills that ship with Neru live: the app's resources, or the source tree in development.
pub fn set_bundled_dir(resources: Option<PathBuf>) {
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("skills");
    let found = resources.map(|dir| dir.join("skills")).filter(|dir| dir.is_dir()).unwrap_or(dev);
    let _ = BUNDLED.set(found);
}

fn bundled_dir() -> Option<&'static PathBuf> {
    BUNDLED.get().filter(|dir| dir.is_dir())
}

fn disabled_path() -> Result<PathBuf, String> {
    Ok(data_dir()?.join("skills-disabled.json"))
}

fn disabled() -> Vec<String> {
    disabled_path().ok().and_then(|path| fs::read_to_string(path).ok()).and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
}

fn personal_dir() -> Result<PathBuf, String> {
    let dir = data_dir()?.join("skills");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

pub fn catalog(root: &Path) -> String {
    let off = disabled();
    let skills: Vec<Skill> = list(root).into_iter().filter(|skill| !(skill.source == "built-in" && off.contains(&skill.name))).collect();
    if skills.is_empty() {
        return String::new();
    }
    let lines = skills
        .iter()
        .map(|skill| format!("- {}: {}", skill.name, skill.description))
        .collect::<Vec<_>>()
        .join("\n");
    format!("\n\nSkills: expert workflows you can load with read_skill. Before starting a task, check this list; when a skill matches (building UI, debugging, reviewing, security, planning, writing prose), load it first and follow it. Skills written for other agents may name their tools: TodoWrite means update_todos, Task or a subagent means task, Read means read_file, Edit means propose_edit, Write means propose_write_file, Bash means run_shell_command, Grep means search_text, Glob means find_files. Skip steps that need tools you do not have.\n{lines}")
}

/// A skill's instructions, or with `file` one of its reference documents. The instructions end
/// with a list of the reference files the skill folder holds.
pub fn read(root: &Path, name: &str, file: Option<&str>) -> Result<String, String> {
    let skill = list(root).into_iter().find(|skill| skill.name == name).ok_or_else(|| format!("No skill named {name}. Check the Skills list in the system prompt."))?;
    if let Some(file) = file.map(str::trim).filter(|file| !file.is_empty()) {
        let relative = file.replace('\\', "/");
        if relative.split('/').any(|part| part == ".." || part.is_empty() && relative.starts_with('/')) || Path::new(&relative).is_absolute() {
            return Err("file must be a path inside the skill folder".into());
        }
        let path = skill.folder.join(&relative);
        let text = fs::read_to_string(&path).map_err(|_| format!("{name} has no file {relative}. Its files are listed at the end of the skill."))?;
        return Ok(text.chars().take(40_000).collect());
    }
    let mut files = Vec::new();
    collect_files(&skill.folder, &skill.folder, &mut files);
    files.retain(|file| file != "SKILL.md" && !file.ends_with("LICENSE.txt") && file != "SOURCE.txt");
    let mut text: String = skill.body.chars().take(30_000).collect();
    if !files.is_empty() {
        files.sort();
        text.push_str(&format!("\n\n[Reference files in this skill; load one with read_skill name={name} file=<path>]\n{}", files.iter().take(80).map(|file| format!("- {file}")).collect::<Vec<_>>().join("\n")));
    }
    Ok(text)
}

fn collect_files(base: &Path, dir: &Path, into: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(base, &path, into);
        } else if let Ok(relative) = path.strip_prefix(base) {
            into.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
}

/// Name, description, and body for slash-command registration.
pub fn slash_entries(root: &Path) -> Vec<(String, String, String)> {
    let off = disabled();
    list(root).into_iter().filter(|skill| !(skill.source == "built-in" && off.contains(&skill.name))).map(|skill| (skill.name, skill.description, skill.body)).collect()
}

fn list(root: &Path) -> Vec<Skill> {
    let mut skills = Vec::new();
    if !root.as_os_str().is_empty() {
        read_tree(&root.join(".neru").join("skills"), "project", &mut skills);
        read_tree(&root.join(".claude").join("skills"), "claude", &mut skills);
    }
    if let Ok(dir) = data_dir() {
        read_tree(&dir.join("skills"), "personal", &mut skills);
    }
    // Last, so a project or personal skill with the same name takes its place.
    if let Some(dir) = bundled_dir() {
        read_tree(dir, "built-in", &mut skills);
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

fn read_tree(folder: &Path, source: &'static str, into: &mut Vec<Skill>) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let file = if path.is_dir() { path.join("SKILL.md") } else { continue };
        if !file.is_file() {
            continue;
        }
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        if text.len() > 40_000 {
            continue;
        }
        let Some(dir_name) = path.file_name().map(|name| name.to_string_lossy().to_lowercase()) else {
            continue;
        };
        let (description, body) = parse_front(&text);
        let name = front_name(&text).unwrap_or(dir_name);
        if !valid(&name) || into.iter().any(|skill| skill.name == name) {
            continue;
        }
        into.push(Skill {
            name,
            description: if description.is_empty() { "Project skill".into() } else { description },
            body,
            source,
            folder: path.clone(),
        });
    }
}

/// A skill name from free text: lowercase letters, digits, and dashes.
fn slug(text: &str) -> String {
    let mut name = String::new();
    for c in text.trim().to_lowercase().chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            name.push(c);
        } else if !name.ends_with('-') && !name.is_empty() {
            name.push('-');
        }
    }
    name.trim_end_matches('-').chars().take(40).collect()
}

/// Gives a plain Markdown file the front matter skills need, using its heading and first line.
fn with_front_matter(text: &str, name: &str) -> String {
    if front_name(text).is_some() || text.trim_start_matches('\u{feff}').starts_with("---") {
        return text.to_string();
    }
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let heading = lines.clone().find(|line| line.starts_with('#')).map(|line| line.trim_start_matches('#').trim().to_string());
    let first = lines.find(|line| !line.starts_with('#')).unwrap_or("").chars().take(200).collect::<String>();
    let description = if first.is_empty() { heading.unwrap_or_else(|| name.replace('-', " ")) } else { first };
    format!("---\nname: {name}\ndescription: {}\n---\n\n{text}", description.replace('\n', " "))
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(from).map_err(|e| e.to_string())?.flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Copies one skill (a SKILL.md or any .md file, or a folder with SKILL.md) into the personal folder.
fn import_one(source: &Path, into: &Path) -> Result<String, String> {
    let (skill_file, folder) = if source.is_dir() {
        let file = source.join("SKILL.md");
        if !file.is_file() {
            return Err(format!("{} has no SKILL.md", source.display()));
        }
        (file, Some(source.to_path_buf()))
    } else {
        let ext = source.extension().and_then(|ext| ext.to_str()).unwrap_or("").to_lowercase();
        if !matches!(ext.as_str(), "md" | "markdown" | "txt") {
            return Err(format!("{} is not a Markdown skill file", source.display()));
        }
        (source.to_path_buf(), None)
    };
    let text = fs::read_to_string(&skill_file).map_err(|e| format!("{}: {e}", skill_file.display()))?;
    if text.len() > 40_000 {
        return Err(format!("{} is over 40 KB; keep skills short and link longer references from the folder", skill_file.display()));
    }
    let stem = if let Some(folder) = &folder {
        folder.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default()
    } else {
        let stem = skill_file.file_stem().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
        if stem.eq_ignore_ascii_case("skill") {
            skill_file.parent().and_then(|dir| dir.file_name()).map(|name| name.to_string_lossy().to_string()).unwrap_or(stem)
        } else {
            stem
        }
    };
    let name = front_name(&text).filter(|name| valid(name)).unwrap_or_else(|| slug(&stem));
    if !valid(&name) {
        return Err(format!("Could not name the skill in {}", skill_file.display()));
    }
    let destination = into.join(&name);
    if destination.exists() {
        fs::remove_dir_all(&destination).map_err(|e| e.to_string())?;
    }
    match folder {
        Some(folder) => copy_dir(&folder, &destination)?,
        None => fs::create_dir_all(&destination).map_err(|e| e.to_string())?,
    }
    fs::write(destination.join("SKILL.md"), with_front_matter(&text, &name)).map_err(|e| e.to_string())?;
    Ok(name)
}

fn views(root: &Path) -> Vec<SkillView> {
    let off = disabled();
    list(root)
        .into_iter()
        .map(|skill| SkillView {
            enabled: !(skill.source == "built-in" && off.contains(&skill.name)),
            chars: skill.body.chars().count(),
            name: skill.name,
            description: skill.description,
            source: skill.source.to_string(),
            path: skill.folder.to_string_lossy().to_string(),
        })
        .collect()
}

fn open_root(state: &State<'_, AppState>) -> PathBuf {
    state.root.lock().ok().and_then(|root| root.clone()).unwrap_or_default()
}

/// Every skill the model can load: the open project's, then the ones added in Settings.
#[tauri::command]
pub fn list_skills(state: State<'_, AppState>) -> Vec<SkillView> {
    views(&open_root(&state))
}

/// Adds skill files or folders to the personal skills folder, available in every project.
#[tauri::command]
pub fn import_skills(paths: Vec<String>, state: State<'_, AppState>) -> Result<Vec<SkillView>, String> {
    let into = personal_dir()?;
    let mut errors = Vec::new();
    for path in paths {
        if let Err(error) = import_one(Path::new(&path), &into) {
            errors.push(error);
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("\n"));
    }
    Ok(views(&open_root(&state)))
}

/// Removes a skill added in Settings. Project skills live in the repository and stay.
#[tauri::command]
pub fn remove_skill(name: String, state: State<'_, AppState>) -> Result<Vec<SkillView>, String> {
    if !valid(&name) {
        return Err("Invalid skill name".into());
    }
    let dir = personal_dir()?.join(&name);
    if dir.is_dir() {
        fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    Ok(views(&open_root(&state)))
}

/// Switches a built-in skill on or off for every project.
#[tauri::command]
pub fn set_skill_enabled(name: String, enabled: bool, state: State<'_, AppState>) -> Result<Vec<SkillView>, String> {
    let mut off = disabled();
    off.retain(|item| item != &name);
    if !enabled {
        off.push(name);
    }
    fs::write(disabled_path()?, serde_json::to_string_pretty(&off).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(views(&open_root(&state)))
}

/// Opens the personal skills folder in the file manager.
#[tauri::command]
pub fn open_skills_folder() -> Result<(), String> {
    let dir = personal_dir()?;
    #[cfg(windows)]
    let program = "explorer";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let program = "xdg-open";
    std::process::Command::new(program).arg(&dir).spawn().map_err(|e| e.to_string())?;
    Ok(())
}

fn front_name(text: &str) -> Option<String> {
    let rest = text.trim_start_matches('\u{feff}').strip_prefix("---")?;
    let end = rest.find("\n---")?;
    front_value(&rest[..end], "name").map(|value| value.to_lowercase())
}

/// One key of YAML front matter: plain, "double" or 'single' quoted, or a `>`/`|` block whose
/// indented lines follow. Enough YAML for skill headers.
pub(crate) fn front_value(front: &str, key: &str) -> Option<String> {
    let lines: Vec<&str> = front.lines().collect();
    let prefix = format!("{key}:");
    let at = lines.iter().position(|line| line.trim_start() == line.trim_start() && line.starts_with(&prefix))?;
    let first = lines[at][prefix.len()..].trim();
    let value = if first.is_empty() || matches!(first, ">" | "|" | ">-" | "|-" | ">+" | "|+") {
        lines[at + 1..].iter().take_while(|line| line.starts_with(' ') || line.starts_with('\t') || line.trim().is_empty()).map(|line| line.trim()).collect::<Vec<_>>().join(" ").trim().to_string()
    } else if let Some(inner) = first.strip_prefix('"').and_then(|rest| rest.strip_suffix('"')) {
        inner.replace("\\\"", "\"")
    } else if let Some(inner) = first.strip_prefix('\'').and_then(|rest| rest.strip_suffix('\'')) {
        inner.replace("''", "'")
    } else {
        first.to_string()
    };
    Some(value).filter(|value| !value.is_empty())
}

fn parse_front(text: &str) -> (String, String) {
    let text = text.trim_start_matches('\u{feff}');
    if let Some(rest) = text.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            let front = &rest[..end];
            let body = rest[end + 4..].trim_start_matches(['\r', '\n']).to_string();
            return (front_value(front, "description").unwrap_or_default(), body);
        }
    }
    (String::new(), text.to_string())
}

fn valid(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 40
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_quoted_and_folded_descriptions() {
        assert_eq!(front_value("name: a\ndescription: \"Use it, \\\"now\\\"\"", "description").as_deref(), Some("Use it, \"now\""));
        assert_eq!(front_value("description: >\n  Use when\n  debugging\nlicense: MIT", "description").as_deref(), Some("Use when debugging"));
        assert_eq!(front_value("description: 'It''s fine'", "description").as_deref(), Some("It's fine"));
    }

    #[test]
    fn bundled_skills_all_parse() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("skills");
        let mut skills = Vec::new();
        read_tree(&dir, "built-in", &mut skills);
        assert!(skills.len() >= 19, "only {} bundled skills parsed", skills.len());
        for skill in &skills {
            assert!(!skill.description.is_empty() && skill.description != "Project skill", "{} has no description", skill.name);
            assert!(skill.body.len() > 200, "{} has no body", skill.name);
        }
    }

    #[test]
    fn reference_files_stay_inside_the_skill() {
        let _ = BUNDLED.set(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("skills"));
        let root = PathBuf::new();
        let text = read(&root, "impeccable", None).unwrap();
        assert!(text.contains("reference/"));
        assert!(read(&root, "impeccable", Some("reference/polish.md")).unwrap().len() > 100);
        assert!(read(&root, "impeccable", Some("../frontend-design/SKILL.md")).is_err());
    }

    #[test]
    fn reads_a_skill_folder() {
        let root = std::env::temp_dir().join(format!("neru-skills-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let skill = root.join(".neru").join("skills").join("review-pr");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "---\nname: review-pr\ndescription: Review a pull request\n---\nLook for bugs in $ARGUMENTS").unwrap();
        let found: Vec<Skill> = list(&root).into_iter().filter(|skill| skill.source == "project").collect();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "review-pr");
        assert!(catalog(&root).contains("review-pr"));
        assert!(read(&root, "review-pr", None).unwrap().contains("Look for bugs"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn imports_plain_markdown_as_a_skill() {
        let base = std::env::temp_dir().join(format!("neru-skill-import-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let file = base.join("React Testing Guide.md");
        fs::write(&file, "# React testing\nUse Testing Library queries by role.\n").unwrap();
        let into = base.join("personal");
        assert_eq!(import_one(&file, &into).unwrap(), "react-testing-guide");
        let text = fs::read_to_string(into.join("react-testing-guide").join("SKILL.md")).unwrap();
        assert!(text.starts_with("---\nname: react-testing-guide\ndescription: Use Testing Library"));
        assert!(import_one(&base.join("missing.png"), &into).is_err());
        let _ = fs::remove_dir_all(&base);
    }
}
