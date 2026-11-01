//! Project checks: `.neru/checks/*.md`, each a pass/fail rule in plain words, run by `neru check`
//! and the Git section's "Run checks". Each check runs as a read-only sub-agent over the project
//! (optionally only when a changed file matches its `glob`) and ends with PASS or FAIL and findings.
//!
//! ```markdown
//! ---
//! name: No secrets in logs
//! description: Log calls never print tokens or passwords
//! glob: src/**/*.rs
//! ---
//! Look at every log or print call in the changed files ...
//! ```

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::{AppState, subagent};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub description: String,
    /// Run only when a changed file matches; every time when None.
    pub glob: Option<String>,
    pub body: String,
    pub path: PathBuf,
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CheckResult {
    pub name: String,
    pub description: String,
    /// "pass", "fail", "skipped" (no changed file matches) or "error".
    pub status: String,
    pub findings: Vec<String>,
}

pub fn folder(root: &Path) -> PathBuf {
    root.join(".neru").join("checks")
}

/// The project's checks, by file name.
pub fn load(root: &Path) -> Vec<Check> {
    let Ok(entries) = std::fs::read_dir(folder(root)) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).filter(|path| path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("md"))).collect();
    paths.sort();
    paths.into_iter().filter_map(|path| Some(parse(&std::fs::read_to_string(&path).ok()?, path))).collect()
}

/// A check file: optional `---` front matter with name, description and glob, then the instructions.
pub fn parse(text: &str, path: PathBuf) -> Check {
    let stem = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_else(|| "check".into());
    let text = text.trim_start_matches('\u{feff}');
    let (front, body) = match text.strip_prefix("---").and_then(|rest| rest.split_once("\n---")) {
        Some((front, body)) => (front, body.split_once('\n').map_or("", |(_, body)| body)),
        None => ("", text),
    };
    let field = |key: &str| {
        front.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            (name.trim().eq_ignore_ascii_case(key)).then(|| value.trim().trim_matches(['"', '\'']).to_string()).filter(|value| !value.is_empty())
        })
    };
    Check {
        name: field("name").unwrap_or(stem),
        description: field("description").unwrap_or_default(),
        glob: field("glob"),
        body: body.trim().to_string(),
        path,
    }
}

/// Whether a check runs for these changed files.
pub fn applies(check: &Check, changed: &[String]) -> bool {
    match &check.glob {
        None => true,
        Some(glob) => glob.split(',').map(str::trim).filter(|glob| !glob.is_empty()).any(|glob| changed.iter().any(|file| crate::policy::glob_covers(glob, &file.replace('\\', "/")))),
    }
}

/// Files changed on this branch and in the working tree, relative to the project root.
pub fn changed_files(root: &Path) -> Vec<String> {
    let mut files: Vec<String> = Vec::new();
    let mut add = |text: String| files.extend(text.lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_string));
    if let Ok(text) = crate::git::git(root, &["diff", "--name-only", "HEAD"]) {
        add(text);
    }
    if let Ok(text) = crate::git::git(root, &["ls-files", "--others", "--exclude-standard"]) {
        add(text);
    }
    for base in ["origin/main", "origin/master", "main", "master"] {
        if let Ok(text) = crate::git::git(root, &["diff", "--name-only", &format!("{base}...HEAD")]) {
            add(text);
            break;
        }
    }
    files.sort();
    files.dedup();
    files
}

/// What the read-only sub-agent is asked.
pub fn prompt(check: &Check, changed: &[String]) -> String {
    let scope = if changed.is_empty() {
        "Check the whole project.".to_string()
    } else {
        let listed: Vec<&str> = changed.iter().take(200).map(String::as_str).collect();
        format!("Focus on these changed files (and what they touch):\n{}", listed.join("\n"))
    };
    format!(
        "You are running the project check \"{name}\"{description}. Decide whether the project passes it. Read the code; you cannot change anything.\n\nThe check:\n{body}\n\n{scope}\n\nUnder ## Answer, put exactly one line `RESULT: PASS` or `RESULT: FAIL`, then one finding per line starting with `- ` (file:line and what is wrong). A check that cannot be decided fails.",
        name = check.name,
        description = if check.description.is_empty() { String::new() } else { format!(" ({})", check.description) },
        body = check.body.chars().take(8_000).collect::<String>(),
    )
}

/// PASS or FAIL and the findings in a sub-agent's report. No verdict counts as a failure.
pub fn verdict(report: &str) -> (bool, Vec<String>) {
    let mut passed = None;
    let mut findings = Vec::new();
    for line in report.lines().map(str::trim) {
        let upper = line.trim_matches(['*', '`', '#', ' ']).to_uppercase();
        if let Some(result) = upper.strip_prefix("RESULT:") {
            let result = result.trim();
            if passed.is_none() {
                passed = Some(result.starts_with("PASS"));
            }
            continue;
        }
        if passed.is_some() {
            if let Some(finding) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
                if !finding.eq_ignore_ascii_case("none") && findings.len() < 30 {
                    findings.push(finding.to_string());
                }
            }
            if line.starts_with("## ") && !line.eq_ignore_ascii_case("## answer") {
                break;
            }
        }
    }
    match passed {
        Some(passed) => (passed, findings),
        None => (false, vec!["The check gave no RESULT line.".into()]),
    }
}

async fn run_one(app: &AppHandle, session: &str, check: &Check, changed: &[String], root: &Path) -> CheckResult {
    let result = |status: &str, findings: Vec<String>| CheckResult { name: check.name.clone(), description: check.description.clone(), status: status.into(), findings };
    if !applies(check, changed) {
        return result("skipped", vec![format!("No changed file matches {}.", check.glob.as_deref().unwrap_or(""))]);
    }
    let state = app.state::<AppState>();
    let config = match state.provider.lock() {
        Ok(config) => config.clone(),
        Err(error) => return result("error", vec![error.to_string()]),
    };
    let job = subagent::Job {
        app: app.clone(),
        session: session.to_string(),
        id: format!("check-{}", uuid::Uuid::new_v4()),
        client: crate::providers::http(),
        config,
        root: root.to_path_buf(),
        web: false,
        cancel: Arc::new(tokio::sync::Notify::new()),
        role: subagent::Role::Explore,
        description: format!("Check: {}", check.name),
        prompt: prompt(check, changed),
        // Plan keeps the agent to reading, whatever its role.
        permissions: subagent::Permissions { plan: true, project: root.to_path_buf(), ..Default::default() },
    };
    match subagent::run(job).await {
        Ok(outcome) => {
            let (passed, findings) = verdict(&outcome.report);
            result(if passed { "pass" } else { "fail" }, findings)
        }
        Err(error) => result("error", vec![error]),
    }
}

/// Runs the project's checks (or the one named), two at a time.
pub async fn run(app: &AppHandle, session: &str, root: &Path, only: Option<&str>) -> Result<Vec<CheckResult>, String> {
    let checks: Vec<Check> = load(root).into_iter().filter(|check| only.is_none_or(|name| check.name.eq_ignore_ascii_case(name) || check.path.file_stem().is_some_and(|stem| stem.eq_ignore_ascii_case(name)))).collect();
    if checks.is_empty() {
        return Err(match only {
            Some(name) => format!("No check named {name} in .neru/checks"),
            None => "No checks yet: add markdown files to .neru/checks (front matter: name, description, optional glob).".into(),
        });
    }
    let changed = {
        let root = root.to_path_buf();
        tauri::async_runtime::spawn_blocking(move || changed_files(&root)).await.unwrap_or_default()
    };
    let mut results = Vec::new();
    for pair in checks.chunks(2) {
        match pair {
            [first, second] => {
                let (a, b) = tokio::join!(run_one(app, session, first, &changed, root), run_one(app, session, second, &changed, root));
                results.extend([a, b]);
            }
            [only] => results.push(run_one(app, session, only, &changed, root).await),
            _ => {}
        }
    }
    Ok(results)
}

#[tauri::command]
pub async fn run_checks(name: Option<String>, app: AppHandle) -> Result<Vec<CheckResult>, String> {
    let root = crate::workspace::project_root(&app.state::<AppState>())?;
    run(&app, "checks", &root, name.as_deref()).await
}

#[tauri::command]
pub fn list_checks(app: AppHandle) -> Result<Vec<CheckResult>, String> {
    let root = crate::workspace::project_root(&app.state::<AppState>())?;
    Ok(load(&root).into_iter().map(|check| CheckResult { name: check.name, description: check.description, status: "idle".into(), findings: Vec::new() }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_front_matter_and_body() {
        let check = parse("---\nname: No secrets\ndescription: \"Logs never print tokens\"\nglob: src/**/*.rs\n---\nLook at log calls.\n", PathBuf::from("a/secrets.md"));
        assert_eq!(check.name, "No secrets");
        assert_eq!(check.description, "Logs never print tokens");
        assert_eq!(check.glob.as_deref(), Some("src/**/*.rs"));
        assert_eq!(check.body, "Look at log calls.");
        let bare = parse("Every public function has a doc comment.", PathBuf::from("x/docs.md"));
        assert_eq!((bare.name.as_str(), bare.glob, bare.body.as_str()), ("docs", None, "Every public function has a doc comment."));
    }

    #[test]
    fn glob_limits_when_a_check_runs() {
        let check = parse("---\nglob: src/**/*.rs, *.toml\n---\nx", PathBuf::from("c.md"));
        assert!(applies(&check, &["src/a/b.rs".into()]));
        assert!(applies(&check, &["Cargo.toml".into()]));
        assert!(!applies(&check, &["README.md".into()]));
        assert!(applies(&parse("x", PathBuf::from("d.md")), &[]));
    }

    #[test]
    fn reads_verdicts() {
        let report = "## Answer\n**RESULT: FAIL**\n- src/log.rs:12 prints the token\n- src/a.rs:3 too\n## Key files\n- src/log.rs:12 - x\n";
        let (passed, findings) = verdict(report);
        assert!(!passed);
        assert_eq!(findings, vec!["src/log.rs:12 prints the token", "src/a.rs:3 too"]);
        assert_eq!(verdict("## Answer\nRESULT: PASS\n- none\n"), (true, vec![]));
        assert!(!verdict("I looked around and it seems fine.").0);
    }

    #[test]
    fn loads_checks_from_the_folder() {
        let root = std::env::temp_dir().join(format!("neru-checks-{}", std::process::id()));
        std::fs::create_dir_all(folder(&root)).unwrap();
        std::fs::write(folder(&root).join("b.md"), "---\nname: Second\n---\nB").unwrap();
        std::fs::write(folder(&root).join("a.md"), "A").unwrap();
        std::fs::write(folder(&root).join("notes.txt"), "ignored").unwrap();
        let names: Vec<String> = load(&root).into_iter().map(|check| check.name).collect();
        assert_eq!(names, ["a", "Second"]);
        let _ = std::fs::remove_dir_all(root);
    }
}
