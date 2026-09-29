//! Custom slash commands: Markdown prompt templates in `.neru/commands/` (or `.claude/commands/`) in
//! the project, and in Neru's data folder for every project. `$ARGUMENTS` is replaced with whatever
//! follows the command name.

use std::{fs, path::Path};

use serde::Serialize;
use tauri::State;

use crate::{AppState, workspace::{data_dir, project_root}};

#[derive(Serialize, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    pub template: String,
    /// "project" or "personal".
    pub source: String,
}

fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 40 && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Splits optional `---` front matter (for `description:`) from the template body.
fn parse(text: &str) -> (String, String) {
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
    let description = text
        .lines()
        .map(|line| line.trim().trim_start_matches('#').trim())
        .find(|line| !line.is_empty())
        .unwrap_or("")
        .chars()
        .take(120)
        .collect();
    (description, text.to_string())
}

fn read_folder(folder: &Path, source: &str, into: &mut Vec<SlashCommand>) {
    let Ok(entries) = fs::read_dir(folder) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|ext| ext != "md") {
            continue;
        }
        let Some(name) = path.file_stem().map(|stem| stem.to_string_lossy().to_lowercase()) else { continue };
        if !valid_name(&name) || into.iter().any(|command| command.name == name) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else { continue };
        if text.len() > 40_000 {
            continue;
        }
        let (description, template) = parse(&text);
        into.push(SlashCommand { name, description, template, source: source.into() });
    }
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
            commands.push(SlashCommand { name, description, template, source: "skill".into() });
        }
    }
    commands.sort_by(|a, b| a.name.cmp(&b.name));
    commands
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
}
