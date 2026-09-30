//! `neru update`: finds out how this copy of the CLI was installed and updates it the same way.
//! Script installs (install.sh / install.ps1) are replaced in place from the GitHub release, after
//! checking the archive's SHA-256; npm, winget and the desktop app are pointed at their own updater.

use std::{
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

use sha2::{Digest, Sha256};

use crate::cli_ui::{BOLD, CYAN, DIM, GREEN, MOSS, RED, RESET, YELLOW};

const REPO: &str = "DiaeEddineJamal/Neru";
const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, PartialEq)]
pub enum InstallKind {
    /// install.sh or install.ps1: `neru` in ~/.neru/cli or %LOCALAPPDATA%\Neru\cli.
    Script,
    Npm,
    Winget,
    /// `neru-cli` beside the desktop app, which updates itself.
    App,
    /// A build from source (`cargo run`, target/…).
    Source,
}

pub fn install_kind() -> InstallKind {
    let exe = std::env::current_exe().ok().and_then(|path| path.canonicalize().ok()).unwrap_or_default();
    let text = exe.to_string_lossy().replace('\\', "/").to_lowercase();
    let name = exe.file_stem().map(|stem| stem.to_string_lossy().to_lowercase()).unwrap_or_default();
    if std::env::var("NERU_INSTALL_KIND").is_ok_and(|kind| kind == "npm") || text.contains("/node_modules/") {
        InstallKind::Npm
    } else if text.contains("/winget/packages/") || text.contains("/winget/links/") {
        InstallKind::Winget
    } else if cfg!(debug_assertions) || text.contains("/target/debug/") || text.contains("/target/release/") {
        InstallKind::Source
    } else if name == "neru-cli" {
        InstallKind::App
    } else {
        InstallKind::Script
    }
}

pub fn target() -> Option<(&'static str, &'static str)> {
    // (Rust target triple, archive extension) as the release workflow names the assets.
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64" | "aarch64") => ("x86_64-pc-windows-msvc", "zip"),
        ("macos", "aarch64") => ("aarch64-apple-darwin", "tar.gz"),
        ("macos", "x86_64") => ("x86_64-apple-darwin", "tar.gz"),
        ("linux", "x86_64") => ("x86_64-unknown-linux-gnu", "tar.gz"),
        _ => return None,
    })
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder().user_agent(format!("neru-cli/{VERSION}")).build().map_err(|e| e.to_string())
}

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())
}

/// The newest release's version, such as "0.5.0".
pub fn latest_version() -> Result<String, String> {
    runtime()?.block_on(async {
        let response = client()?
            .get(format!("https://api.github.com/repos/{REPO}/releases/latest"))
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| format!("Could not reach GitHub: {e}"))?;
        if !response.status().is_success() {
            return Err(format!("GitHub answered {} for the latest release", response.status()));
        }
        let body: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
        body["tag_name"].as_str().map(|tag| tag.trim_start_matches('v').to_string()).ok_or_else(|| "The latest release has no tag".to_string())
    })
}

fn parse(version: &str) -> Vec<u64> {
    version.split(['.', '-']).map(|part| part.parse().unwrap_or(0)).collect()
}

pub fn newer(candidate: &str, current: &str) -> bool {
    parse(candidate) > parse(current)
}

async fn download(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let response = client.get(url).send().await.map_err(|e| format!("Download failed: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("Download failed ({}) for {url}", response.status()));
    }
    Ok(response.bytes().await.map_err(|e| e.to_string())?.to_vec())
}

/// Downloads, verifies and unpacks the release archive into a fresh folder beside `install_dir`.
fn fetch(version: &str, install_dir: &Path) -> Result<PathBuf, String> {
    let (triple, extension) = target().ok_or("There is no prebuilt CLI for this platform yet. Build it from source: see the README.")?;
    let asset = format!("neru-cli-{triple}.{extension}");
    let base = format!("https://github.com/{REPO}/releases/download/v{version}");
    let (archive, sums) = runtime()?.block_on(async {
        let client = client()?;
        let archive = download(&client, &format!("{base}/{asset}")).await?;
        let sums = download(&client, &format!("{base}/{asset}.sha256")).await?;
        Ok::<_, String>((archive, sums))
    })?;
    let expected = String::from_utf8_lossy(&sums).split_whitespace().next().unwrap_or("").to_lowercase();
    let actual: String = Sha256::digest(&archive).iter().map(|byte| format!("{byte:02x}")).collect();
    if expected.len() != 64 || expected != actual {
        return Err("The download did not match its checksum, so nothing was changed. Try again later.".into());
    }
    let staging = install_dir.with_file_name(format!("{}.new", install_dir.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_else(|| "cli".into())));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let file = staging.join(&asset);
    std::fs::write(&file, &archive).map_err(|e| e.to_string())?;
    // bsdtar (Windows 10+, macOS) and GNU tar both unpack these; bsdtar also reads zip.
    let status = Command::new("tar").arg("-xf").arg(&file).arg("-C").arg(&staging).status().map_err(|e| format!("Could not run tar: {e}"))?;
    let _ = std::fs::remove_file(&file);
    if !status.success() {
        return Err("Could not unpack the update".into());
    }
    Ok(staging)
}

/// Swaps the unpacked update into place. The running executable is renamed out of the way first,
/// which Windows allows even while it runs.
fn install(staging: &Path, install_dir: &Path, exe: &Path) -> Result<(), String> {
    let name = exe.file_name().ok_or("Unknown executable")?;
    let fresh = staging.join(if cfg!(windows) { "neru.exe" } else { "neru" });
    if !fresh.is_file() {
        return Err("The update archive has no neru executable".into());
    }
    let old = exe.with_extension("old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old).map_err(|e| format!("Could not replace {}: {e}", exe.display()))?;
    if let Err(error) = std::fs::rename(&fresh, install_dir.join(name)) {
        let _ = std::fs::rename(&old, exe);
        return Err(format!("Could not install the update: {error}"));
    }
    let skills = staging.join("skills");
    if skills.is_dir() {
        let _ = std::fs::remove_dir_all(install_dir.join("skills"));
        let _ = std::fs::rename(&skills, install_dir.join("skills"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(install_dir.join(name), std::fs::Permissions::from_mode(0o755));
    }
    #[cfg(not(windows))]
    let _ = std::fs::remove_file(&old);
    let _ = std::fs::remove_dir_all(staging);
    Ok(())
}

/// `neru update`. Returns the process exit code.
pub fn run() -> i32 {
    let plain = !std::io::IsTerminal::is_terminal(&std::io::stdout());
    let say = |text: String| {
        println!("{}", if plain { crate::cli_ui::strip(&text) } else { text });
        let _ = std::io::stdout().flush();
    };
    say(format!("{MOSS}✻{RESET} {BOLD}Neru{RESET} {DIM}{VERSION}{RESET} · checking for updates…"));
    let latest = match latest_version() {
        Ok(version) => version,
        Err(error) => {
            say(format!("{RED}✗{RESET} {error}"));
            return 1;
        }
    };
    if !newer(&latest, VERSION) {
        say(format!("{GREEN}✓{RESET} You have the latest version ({VERSION})."));
        return 0;
    }
    say(format!("  {BOLD}{latest}{RESET} is available {DIM}(you have {VERSION}){RESET}"));
    let how = match install_kind() {
        InstallKind::Npm => Some("npm install -g neru-cli@latest"),
        InstallKind::Winget => Some("winget upgrade Luziv.Neru.CLI"),
        InstallKind::App => {
            say(format!("  This neru came with the Neru app, which updates itself: open it and choose {BOLD}Restart to update{RESET}."));
            Some(if cfg!(windows) { "winget upgrade Luziv.Neru" } else { "" })
        }
        InstallKind::Source => Some("git pull, then cargo build --release --bin neru-cli"),
        InstallKind::Script => None,
    };
    if let Some(command) = how {
        if !command.is_empty() {
            say(format!("  Update with {CYAN}{command}{RESET}"));
        }
        return 0;
    }
    let Some(exe) = std::env::current_exe().ok().and_then(|path| path.canonicalize().ok()) else {
        say(format!("{RED}✗{RESET} Could not find where neru is installed."));
        return 1;
    };
    let Some(dir) = exe.parent().map(Path::to_path_buf) else { return 1 };
    say(format!("  {DIM}Downloading neru {latest}…{RESET}"));
    match fetch(&latest, &dir).and_then(|staging| install(&staging, &dir, &exe)) {
        Ok(()) => {
            say(format!("{GREEN}✓{RESET} Updated to {BOLD}{latest}{RESET}. Run {CYAN}neru{RESET} to start it."));
            0
        }
        Err(error) => {
            say(format!("{RED}✗{RESET} {error}"));
            say(format!("  {YELLOW}You can also reinstall with the command in the README.{RESET}"));
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_numerically() {
        assert!(newer("0.10.0", "0.9.9"));
        assert!(newer("1.0.0", "0.99.0"));
        assert!(!newer("0.4.0", "0.4.0"));
        assert!(!newer("0.3.9", "0.4.0"));
    }
}
