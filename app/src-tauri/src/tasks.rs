//! Chooses the build, test, or lint command for the project that is open.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{LazyLock, Mutex},
};

use serde_json::Value;

pub fn command(root: &Path, task: &str) -> Result<String, String> {
    if !matches!(task, "build" | "test" | "lint") {
        return Err("Only build, test, and lint are supported".into());
    }
    if let Some(script) = npm_script(root, task)? {
        return Ok(format!("npm run {script}"));
    }
    if root.join("Cargo.toml").is_file() {
        return Ok(match task {
            "build" => "cargo build",
            "test" => "cargo test",
            "lint" => "cargo clippy -- -D warnings",
            _ => unreachable!(),
        }
        .into());
    }
    if root.join("go.mod").is_file() {
        return Ok(match task {
            "build" => "go build ./...",
            "test" => "go test ./...",
            "lint" => "go vet ./...",
            _ => unreachable!(),
        }
        .into());
    }
    if root.join("pyproject.toml").is_file()
        || root.join("pytest.ini").is_file()
        || root.join("setup.py").is_file()
        || root.join("requirements.txt").is_file()
    {
        return Ok(match task {
            "build" => "python -m build",
            "test" => "python -m pytest",
            "lint" => "python -m ruff check .",
            _ => unreachable!(),
        }
        .into());
    }
    if root.join("pom.xml").is_file() {
        return Ok(match task {
            "build" => "mvn -q -DskipTests package",
            "test" => "mvn -q test",
            "lint" => "mvn -q verify",
            _ => unreachable!(),
        }
        .into());
    }
    if root.join("build.gradle").is_file() || root.join("build.gradle.kts").is_file() {
        return Ok(match task {
            "build" => "gradle build -x test",
            "test" => "gradle test",
            "lint" => "gradle check",
            _ => unreachable!(),
        }
        .into());
    }
    if let Some(make) = make_target(root, task) {
        return Ok(format!("make {make}"));
    }
    Err(format!("No {task} command for this project"))
}

fn npm_script(root: &Path, task: &str) -> Result<Option<String>, String> {
    let path = root.join("package.json");
    if !path.is_file() {
        return Ok(None);
    }
    let package = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let json: Value = serde_json::from_str(&package).map_err(|e| e.to_string())?;
    let names = match task {
        "lint" => ["lint", "check", "typecheck"],
        other => [other, other, other],
    };
    Ok(names.into_iter().find(|name| json["scripts"][name].is_string()).map(str::to_string))
}

fn make_target(root: &Path, task: &str) -> Option<String> {
    let text = fs::read_to_string(root.join("Makefile")).ok()?;
    let names = match task {
        "lint" => ["lint", "check"],
        other => [other, other],
    };
    names
        .into_iter()
        .find(|name| {
            text.lines().any(|line| {
                let head = line.split(':').next().unwrap_or("").trim();
                head == *name
            })
        })
        .map(str::to_string)
}

// ---------------------------------------------------------------------------------------------
// Test until green
// ---------------------------------------------------------------------------------------------

/// Runs so far per (project, task) while the model works toward a passing run.
static UNTIL_GREEN: LazyLock<Mutex<HashMap<(PathBuf, String), u8>>> = LazyLock::new(Default::default);
/// Runs a model gets to make a failing task pass before Neru tells it to stop and report.
pub const GREEN_ATTEMPTS: u8 = 3;

/// Records whether the next run of `task` is part of a fix-until-green loop.
pub fn want_green(root: &Path, task: &str, on: bool) {
    let mut loops = UNTIL_GREEN.lock().unwrap_or_else(|p| p.into_inner());
    let key = (root.to_path_buf(), task.to_string());
    if on {
        loops.entry(key).or_insert(0);
    } else {
        loops.remove(&key);
    }
}

/// The lines of build, test or lint output that name a failure, deduplicated, at most `limit`:
/// Rust and TypeScript errors with their locations, failed tests from cargo, pytest, Jest,
/// Vitest and Go, and `file:line:col: error` lines from linters.
pub fn failures(output: &str, limit: usize) -> Vec<String> {
    let lines: Vec<&str> = output.lines().collect();
    let mut found: Vec<String> = Vec::new();
    for (i, raw) in lines.iter().enumerate() {
        let line = raw.trim();
        let failure = if line.starts_with("error[") || line.starts_with("error:") && !line.starts_with("error: could not compile") && !line.starts_with("error: aborting") {
            // rustc: the location follows on a ` --> file:line:col` line.
            let at = lines[i + 1..].iter().take(4).find_map(|next| next.trim().strip_prefix("--> ")).map(|at| format!(" ({at})")).unwrap_or_default();
            Some(format!("{line}{at}"))
        } else if line.starts_with("test ") && line.ends_with("FAILED")
            || ["FAILED ", "--- FAIL:", "FAIL ", "● ", "× ", "✕ "].iter().any(|start| line.starts_with(start))
            || line.contains(": error TS")
            || line.contains("): error ")
            || line.contains(": error:")
            || line.starts_with("thread '") && line.contains("panicked at")
        {
            Some(line.to_string())
        } else {
            None
        };
        if let Some(failure) = failure {
            let failure: String = failure.chars().take(240).collect();
            if !found.contains(&failure) {
                found.push(failure);
            }
        }
        if found.len() >= limit {
            break;
        }
    }
    found
}

/// The task's output for the model: the failures pulled to the top, and, in a fix-until-green
/// loop, how many runs are left or that it is time to stop.
pub fn report(root: &Path, task: &str, output: String) -> String {
    let code = output.lines().next().and_then(|line| line.strip_prefix("Exit code: ")).and_then(|code| code.trim().parse::<i32>().ok());
    let key = (root.to_path_buf(), task.to_string());
    let mut loops = UNTIL_GREEN.lock().unwrap_or_else(|p| p.into_inner());
    if code == Some(0) {
        return if loops.remove(&key).is_some() { format!("{task} passes now.\n{output}") } else { output };
    }
    let found = failures(&output, 20);
    let mut head = if found.is_empty() { String::new() } else { format!("Failures ({}{}):\n- {}\n", found.len(), if found.len() == 20 { "+" } else { "" }, found.join("\n- ")) };
    if let Some(runs) = loops.get_mut(&key) {
        *runs += 1;
        if *runs >= GREEN_ATTEMPTS {
            loops.remove(&key);
            head.push_str(&format!("Run {GREEN_ATTEMPTS} of {GREEN_ATTEMPTS} still fails. Stop retrying: tell the user what still fails and what you tried.\n"));
        } else {
            head.push_str(&format!("Run {runs} of {GREEN_ATTEMPTS} failed. Fix the failures above, then call run_project_task with task {task} and until_green true again.\n"));
        }
    }
    if head.is_empty() { output } else { format!("{head}\n{output}") }
}

#[cfg(test)]
mod green_tests {
    use super::*;

    #[test]
    fn parses_failures_from_common_tools() {
        let cargo = "Exit code: 101\nerror[E0308]: mismatched types\n  --> src/main.rs:4:5\n   |\ntest tests::adds ... FAILED\nthread 'tests::adds' panicked at src/lib.rs:9:5:\nerror: could not compile `x`";
        let found = failures(cargo, 10);
        assert_eq!(found[0], "error[E0308]: mismatched types (src/main.rs:4:5)");
        assert!(found.contains(&"test tests::adds ... FAILED".to_string()));
        assert!(!found.iter().any(|line| line.contains("could not compile")));
        let other = "src/App.tsx(12,3): error TS2322: Type 'string' is not assignable to type 'number'.\nFAILED tests/test_api.py::test_get - AssertionError\n--- FAIL: TestParse (0.00s)";
        assert_eq!(failures(other, 10).len(), 3);
        assert!(failures("Exit code: 0\nall good", 10).is_empty());
    }

    #[test]
    fn until_green_counts_runs_and_stops() {
        let root = Path::new("/tmp/neru-green-test");
        want_green(root, "test", true);
        let failing = || "Exit code: 1\nFAILED tests/a.py::t - boom".to_string();
        assert!(report(root, "test", failing()).contains("Run 1 of 3 failed"));
        assert!(report(root, "test", failing()).contains("Run 2 of 3 failed"));
        assert!(report(root, "test", failing()).contains("Stop retrying"));
        // The loop ended; a later failure is reported plainly.
        assert!(!report(root, "test", failing()).contains("Run "));
        want_green(root, "test", true);
        assert!(report(root, "test", "Exit code: 0\nok".into()).starts_with("test passes now"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("neru-tasks-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn picks_the_stack_that_is_actually_present() {
        let npm = dir("npm");
        fs::write(npm.join("package.json"), r#"{"scripts":{"test":"vitest","lint":"eslint ."}}"#).unwrap();
        assert_eq!(command(&npm, "test").unwrap(), "npm run test");
        assert_eq!(command(&npm, "lint").unwrap(), "npm run lint");

        let rust = dir("rust");
        fs::write(rust.join("Cargo.toml"), "[package]\nname=\"a\"\nversion=\"0.1.0\"\n").unwrap();
        assert_eq!(command(&rust, "test").unwrap(), "cargo test");

        let python = dir("py");
        fs::write(python.join("pyproject.toml"), "[project]\nname='a'\n").unwrap();
        assert_eq!(command(&python, "lint").unwrap(), "python -m ruff check .");

        let make = dir("make");
        fs::write(make.join("Makefile"), "build:\n\ttrue\n").unwrap();
        assert_eq!(command(&make, "build").unwrap(), "make build");
        assert!(command(&make, "test").is_err());
    }
}
