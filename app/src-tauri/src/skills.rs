//! Skills are markdown workflows the model can load by name, and the user can run with `/name`.
//! A skill is a `SKILL.md` file in `.neru/skills/<name>/`, `.claude/skills/<name>/`, or Neru's
//! personal skills folder.

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
    /// "project" (.neru/skills), "claude" (.claude/skills), or "personal" (added in Settings).
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
}

fn personal_dir() -> Result<PathBuf, String> {
    let dir = data_dir()?.join("skills");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

pub fn catalog(root: &Path) -> String {
    let skills = list(root);
    if skills.is_empty() {
        return String::new();
    }
    let lines = skills
        .iter()
        .map(|skill| format!("- {}: {}", skill.name, skill.description))
        .collect::<Vec<_>>()
        .join("\n");
    format!("\n\nSkills you can load with the read_skill tool when one matches the task:\n{lines}")
}

pub fn read(root: &Path, name: &str) -> Result<String, String> {
    list(root)
        .into_iter()
        .find(|skill| skill.name == name)
        .map(|skill| skill.body.chars().take(20_000).collect())
        .ok_or_else(|| format!("No skill named {name}"))
}

/// Name, description, and body for slash-command registration.
pub fn slash_entries(root: &Path) -> Vec<(String, String, String)> {
    list(root).into_iter().map(|skill| (skill.name, skill.description, skill.body)).collect()
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
    list(root)
        .into_iter()
        .map(|skill| SkillView {
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
    rest[..end]
        .lines()
        .find_map(|line| line.trim().strip_prefix("name:"))
        .map(|value| value.trim().trim_matches('"').to_lowercase())
}

fn parse_front(text: &str) -> (String, String) {
    let text = text.trim_start_matches('\u{feff}');
    if let Some(rest) = text.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            let front = &rest[..end];
            let body = rest[end + 4..].trim_start_matches(['\r', '\n']).to_string();
            let description = front
                .lines()
                .find_map(|line| line.trim().strip_prefix("description:"))
                .map(|value| value.trim().trim_matches('"').to_string())
                .unwrap_or_default();
            return (description, body);
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
    fn reads_a_skill_folder() {
        let root = std::env::temp_dir().join(format!("neru-skills-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let skill = root.join(".neru").join("skills").join("review-pr");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "---\nname: review-pr\ndescription: Review a pull request\n---\nLook for bugs in $ARGUMENTS").unwrap();
        let found = list(&root);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "review-pr");
        assert!(catalog(&root).contains("review-pr"));
        assert!(read(&root, "review-pr").unwrap().contains("Look for bugs"));
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
