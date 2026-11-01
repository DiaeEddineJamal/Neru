//! Reviewable documents in a session: a plan, a walkthrough, or Spec mode's requirements, design
//! and tasks. The agent saves one with `write_document`; the window opens it in the artifact editor,
//! where the user edits it or comments on its lines, and the comments go back to the agent as a
//! message. Documents live with the session in Neru's data folder, not in the project, so writing
//! one never needs an approval.

use std::{
    fs,
    path::PathBuf,
    time::UNIX_EPOCH,
};

use serde::Serialize;

use crate::workspace::data_dir;

pub(crate) const TOOL: &str = r#"{"type":"function","function":{"name":"write_document","description":"Save a Markdown document the user reviews in a side panel: a plan, a walkthrough of what you changed, or Spec mode's requirements, design and tasks. Writing the same name again replaces it (the earlier copy is kept). The user can edit it and comment on its lines; their comments come back to you as a message. Use it instead of long Markdown in your reply when the user should review or keep the text.","parameters":{"type":"object","properties":{"name":{"type":"string","description":"Short lowercase name such as plan, walkthrough, requirements, design or tasks"},"content":{"type":"string","description":"The whole document in Markdown, starting with a # title"}},"required":["name","content"]}}}"#;

/// Spec mode's instruction: three reviewed documents before any code.
pub(crate) const SPEC_MODE: &str = "Current mode: Spec. Before any code changes, work through three documents, each saved with write_document and approved by the user: 1) name \"requirements\": user stories, each with numbered acceptance criteria in EARS form (WHEN … THE SYSTEM SHALL …), plus what is out of scope; 2) name \"design\": architecture, components and interfaces, data models, error handling and the testing approach, grounded in the code you inspect; 3) name \"tasks\": a numbered checklist (- [ ] 1. …) of small, testable coding steps in order, each naming the requirements it covers. Write one document per turn, then stop and ask the user to review it. When they comment, revise that document with write_document. Move to the next document only after the user approves the current one. Read and inspect only; do not propose edits or commands. When the user approves the tasks, Neru switches to an editing mode: then do the tasks in order and tick each one off by saving tasks again with it checked (- [x]).";

const MAX_BYTES: usize = 400_000;
const HISTORY: &str = ".history";

#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub name: String,
    pub title: String,
    pub size: u64,
    pub modified: u64,
    /// Checklist items in the document, done and total, for Spec mode's tasks.
    pub done: usize,
    pub total: usize,
}

fn folder(session: &str) -> Result<PathBuf, String> {
    uuid::Uuid::parse_str(session).map_err(|_| "Invalid session id")?;
    Ok(data_dir()?.join("sessions").join(format!("{session}.docs")))
}

/// A document name as a file stem: lowercase letters, digits and dashes.
pub(crate) fn slug(name: &str) -> Result<String, String> {
    let name = name.trim().trim_end_matches(".md").to_lowercase();
    let mut slug = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_end_matches('-').chars().take(48).collect();
    if slug.is_empty() { Err("Give the document a short name, such as plan or design".into()) } else { Ok(slug) }
}

/// The first `# ` heading, or the name.
pub(crate) fn title(name: &str, text: &str) -> String {
    text.lines()
        .find_map(|line| line.trim().strip_prefix("# ").map(|title| title.trim().to_string()))
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| {
            let mut chars = name.chars();
            chars.next().map(|first| first.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
        })
}

/// Checklist items `- [ ]` and `- [x]`: done and total.
pub(crate) fn progress(text: &str) -> (usize, usize) {
    let mut done = 0;
    let mut total = 0;
    for line in text.lines() {
        let line = line.trim_start();
        let Some(rest) = line.strip_prefix("- [").or_else(|| line.strip_prefix("* [")) else { continue };
        match rest.chars().next() {
            Some('x' | 'X') if rest[1..].starts_with(']') => {
                done += 1;
                total += 1;
            }
            Some(' ') if rest[1..].starts_with(']') => total += 1,
            _ => {}
        }
    }
    (done, total)
}

fn describe(name: &str, path: &std::path::Path) -> Option<Document> {
    let meta = fs::metadata(path).ok()?;
    let text = fs::read_to_string(path).ok()?;
    let (done, total) = progress(&text);
    Some(Document {
        name: name.to_string(),
        title: title(name, &text),
        size: meta.len(),
        modified: meta.modified().ok().and_then(|time| time.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as u64),
        done,
        total,
    })
}

/// Saves a document, keeping the copy it replaces.
pub(crate) fn write(session: &str, name: &str, content: &str) -> Result<Document, String> {
    if content.len() > MAX_BYTES {
        return Err("The document is too long; split it into smaller ones".into());
    }
    let name = slug(name)?;
    let dir = folder(session)?;
    fs::create_dir_all(dir.join(HISTORY)).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{name}.md"));
    if let Ok(previous) = fs::read_to_string(&path) {
        if previous == content {
            return describe(&name, &path).ok_or_else(|| "Could not read the document".to_string());
        }
        let stamp = std::time::SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
        let _ = fs::write(dir.join(HISTORY).join(format!("{name}.md.{stamp}")), previous);
    }
    fs::write(&path, content).map_err(|e| e.to_string())?;
    describe(&name, &path).ok_or_else(|| "Could not read the document".to_string())
}

pub(crate) fn list(session: &str) -> Result<Vec<Document>, String> {
    let dir = folder(session)?;
    let Ok(entries) = fs::read_dir(&dir) else { return Ok(Vec::new()) };
    let mut documents: Vec<Document> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?.strip_suffix(".md")?.to_string();
            describe(&name, &path)
        })
        .collect();
    // Spec documents in their order, then the rest by name.
    let rank = |name: &str| ["requirements", "design", "tasks"].iter().position(|item| *item == name).unwrap_or(3);
    documents.sort_by(|a, b| rank(&a.name).cmp(&rank(&b.name)).then_with(|| a.name.cmp(&b.name)));
    Ok(documents)
}

pub(crate) fn read(session: &str, name: &str) -> Result<String, String> {
    fs::read_to_string(folder(session)?.join(format!("{}.md", slug(name)?))).map_err(|_| "That document is gone".to_string())
}

#[tauri::command]
pub fn list_session_documents(session_id: String) -> Result<Vec<Document>, String> {
    list(&session_id)
}

#[tauri::command]
pub fn read_session_document(session_id: String, name: String) -> Result<String, String> {
    read(&session_id, &name)
}

/// Saves the user's own edits to a document.
#[tauri::command]
pub fn write_session_document(session_id: String, name: String, content: String) -> Result<Vec<Document>, String> {
    write(&session_id, &name, &content)?;
    list(&session_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_become_safe_file_stems() {
        assert_eq!(slug("Plan").unwrap(), "plan");
        assert_eq!(slug("design.md").unwrap(), "design");
        assert_eq!(slug("../../etc/passwd").unwrap(), "etc-passwd");
        assert_eq!(slug("API walk-through!").unwrap(), "api-walk-through");
        assert!(slug("  ..  ").is_err());
        assert!(slug(&"x".repeat(100)).unwrap().len() <= 48);
    }

    #[test]
    fn titles_and_checklists() {
        assert_eq!(title("tasks", "# Tasks for login\n\n- [ ] 1. a"), "Tasks for login");
        assert_eq!(title("design", "No heading here"), "Design");
        assert_eq!(progress("- [x] 1. a\n- [ ] 2. b\n  - [X] 2.1 c\n* [ ] d\n- [link](x)\n- not a task"), (2, 4));
    }

    #[test]
    fn documents_keep_earlier_copies_and_sort_in_spec_order() {
        let session = uuid::Uuid::new_v4().to_string();
        assert!(write("not-a-session", "plan", "x").is_err());
        write(&session, "tasks", "# Tasks\n- [ ] 1. a").unwrap();
        write(&session, "requirements", "# Requirements").unwrap();
        write(&session, "notes", "# Notes").unwrap();
        let saved = write(&session, "tasks", "# Tasks\n- [x] 1. a").unwrap();
        assert_eq!((saved.done, saved.total), (1, 1));
        let names: Vec<String> = list(&session).unwrap().into_iter().map(|doc| doc.name).collect();
        assert_eq!(names, vec!["requirements", "tasks", "notes"]);
        assert_eq!(read(&session, "Tasks").unwrap(), "# Tasks\n- [x] 1. a");
        let history = fs::read_dir(folder(&session).unwrap().join(HISTORY)).unwrap().count();
        assert_eq!(history, 1);
        let _ = fs::remove_dir_all(folder(&session).unwrap());
    }
}
