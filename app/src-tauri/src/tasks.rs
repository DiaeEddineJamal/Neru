//! Chooses the build, test, or lint command for the project that is open.

use std::{fs, path::Path};

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
