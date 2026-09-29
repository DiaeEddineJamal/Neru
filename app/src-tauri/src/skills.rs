//! Skills are markdown workflows the model can load by name, and the user can run with `/name`.
//! A skill is a `SKILL.md` file in `.neru/skills/<name>/`, `.claude/skills/<name>/`, or Neru's
//! personal skills folder.

use std::{fs, path::Path};

use crate::workspace::data_dir;

#[derive(Clone, Debug, PartialEq)]
struct Skill {
    name: String,
    description: String,
    body: String,
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
    read_tree(&root.join(".neru").join("skills"), &mut skills);
    read_tree(&root.join(".claude").join("skills"), &mut skills);
    if let Ok(dir) = data_dir() {
        read_tree(&dir.join("skills"), &mut skills);
    }
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

fn read_tree(folder: &Path, into: &mut Vec<Skill>) {
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
        });
    }
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
}
