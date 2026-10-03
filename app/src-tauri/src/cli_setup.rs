//! Settings → CLI: finds the `neru` command and puts the CLI bundled with this app on the PATH.
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CliStatus {
    pub on_path: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    /// The neru-cli binary next to the app's executable, when this build ships one.
    pub bundled: Option<String>,
    pub platform: String,
    /// Something the user should know after an install, such as opening a new terminal.
    pub note: Option<String>,
}

#[cfg(windows)]
const NAMES: &[&str] = &["neru.exe", "neru.cmd", "neru.bat"];
#[cfg(not(windows))]
const NAMES: &[&str] = &["neru"];

pub(crate) fn command(program: &Path) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    command
}

/// Runs a command and returns its stdout, or None if it fails or takes longer than `limit`.
pub(crate) fn output_within(mut command: Command, limit: Duration) -> Option<String> {
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < limit => std::thread::sleep(Duration::from_millis(40)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut text = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut text).ok()?;
    Some(text)
}

fn bundled_cli() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let path = dir.join(if cfg!(windows) {
        "neru-cli.exe"
    } else {
        "neru-cli"
    });
    path.is_file().then_some(path)
}

/// The PATH a new terminal would get: this process's, plus the per-user one (Windows) or the login shell's.
pub(crate) fn search_path() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    #[cfg(windows)]
    if let Some(user) = user_path() {
        dirs.extend(std::env::split_paths(&expand_vars(&user)));
    }
    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let mut login = Command::new(shell);
        login.args(["-lc", "printf %s \"$PATH\""]);
        if let Some(path) = output_within(login, Duration::from_secs(3)) {
            dirs.extend(std::env::split_paths(path.trim()));
        }
    }
    dirs
}

/// The desktop app's own executable is also called neru; running it with --version would open a window.
fn is_desktop_app(path: &Path) -> bool {
    let cli = if cfg!(windows) {
        "neru-cli.exe"
    } else {
        "neru-cli"
    };
    let Ok(real) = path.canonicalize() else {
        return false;
    };
    let current = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.canonicalize().ok());
    let launcher = real
        .extension()
        .is_some_and(|ext| ext == "cmd" || ext == "bat");
    let sibling = real.parent().is_some_and(|dir| dir.join(cli).is_file());
    Some(&real) == current.as_ref()
        || (sibling && !launcher && real.file_name().is_some_and(|name| name != cli))
}

fn find_on_path() -> Option<PathBuf> {
    search_path()
        .into_iter()
        .filter(|dir| !dir.as_os_str().is_empty())
        .find_map(|dir| {
            NAMES
                .iter()
                .map(|name| dir.join(name))
                .find(|path| path.is_file() && !is_desktop_app(path))
        })
}

/// Finds `name` the way a new terminal would, including npm's `.cmd` shims on Windows, then in
/// `extra` folders that installers use but often leave off the PATH.
pub(crate) fn which(name: &str, extra: &[PathBuf]) -> Option<PathBuf> {
    let files: Vec<String> = if cfg!(windows) {
        ["exe", "cmd", "bat"].iter().map(|ext| format!("{name}.{ext}")).collect()
    } else {
        vec![name.to_string()]
    };
    search_path()
        .into_iter()
        .chain(extra.iter().cloned())
        .filter(|dir| !dir.as_os_str().is_empty())
        .find_map(|dir| files.iter().map(|file| dir.join(file)).find(|path| path.is_file()))
}

fn version_of(path: &Path) -> Option<String> {
    let mut run = command(path);
    run.arg("--version");
    let text = output_within(run, Duration::from_secs(3))?;
    let line = text.lines().find(|line| !line.trim().is_empty())?.trim();
    Some(
        line.strip_prefix("neru ")
            .unwrap_or(line)
            .trim()
            .to_string(),
    )
}

fn status(note: Option<String>) -> CliStatus {
    let found = find_on_path();
    CliStatus {
        on_path: found.is_some(),
        version: found.as_deref().and_then(version_of),
        path: found.map(|p| p.display().to_string()),
        bundled: bundled_cli().map(|p| p.display().to_string()),
        platform: std::env::consts::OS.to_string(),
        note,
    }
}

#[tauri::command]
pub async fn cli_status() -> CliStatus {
    tauri::async_runtime::spawn_blocking(|| status(None))
        .await
        .unwrap_or_else(|_| status(None))
}

#[tauri::command]
pub async fn cli_install_path() -> Result<CliStatus, String> {
    tauri::async_runtime::spawn_blocking(install)
        .await
        .map_err(|e| e.to_string())?
}

fn install() -> Result<CliStatus, String> {
    let bundled = bundled_cli()
        .ok_or("This build has no bundled CLI; install it with one of the commands below.")?;
    let note = link(&bundled)?;
    Ok(status(note))
}

/// The per-user Path from the registry, unexpanded.
#[cfg(windows)]
fn user_path() -> Option<String> {
    let mut query = command(Path::new("reg"));
    query.args(["query", r"HKCU\Environment", "/v", "Path"]);
    let text = output_within(query, Duration::from_secs(3))?;
    text.lines().find_map(|line| {
        let line = line.trim();
        let rest = line
            .strip_prefix("Path")
            .or_else(|| line.strip_prefix("PATH"))?
            .trim_start();
        let (_, value) = rest.split_once("REG_")?;
        Some(
            value
                .split_once(char::is_whitespace)
                .map(|(_, v)| v.trim().to_string())
                .unwrap_or_default(),
        )
    })
}

#[cfg(windows)]
fn expand_vars(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        let Some(len) = rest[start + 1..].find('%') else {
            break;
        };
        out.push_str(&rest[..start]);
        let name = &rest[start + 1..start + 1 + len];
        out.push_str(&std::env::var(name).unwrap_or_else(|_| format!("%{name}%")));
        rest = &rest[start + len + 2..];
    }
    out.push_str(rest);
    out
}

/// Windows: a neru.cmd launcher in <app>\bin, as the installer writes, and that folder on the user's Path.
#[cfg(windows)]
fn link(bundled: &Path) -> Result<Option<String>, String> {
    let bin = bundled
        .parent()
        .ok_or("Could not find the app folder.")?
        .join("bin");
    std::fs::create_dir_all(&bin)
        .map_err(|e| format!("Could not create {}: {e}", bin.display()))?;
    std::fs::write(
        bin.join("neru.cmd"),
        "@echo off\r\n\"%~dp0..\\neru-cli.exe\" %*\r\n",
    )
    .map_err(|e| format!("Could not write neru.cmd: {e}"))?;
    // Read and write the raw value so %VARIABLES% in it survive, then broadcast the change to new terminals.
    let script = r#"$ErrorActionPreference = 'Stop'
$dir = $env:NERU_BIN_DIR
$key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
$path = [string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
if (-not (($path -split ';') | Where-Object { $_.TrimEnd('\') -ieq $dir.TrimEnd('\') })) {
  $next = if ($path.Trim()) { $path.TrimEnd(';') + ';' + $dir } else { $dir }
  $key.SetValue('Path', $next, [Microsoft.Win32.RegistryValueKind]::ExpandString)
}
$key.Close()
[Environment]::SetEnvironmentVariable('NERU_PATH_REFRESH', '1', 'User')
[Environment]::SetEnvironmentVariable('NERU_PATH_REFRESH', $null, 'User')
"#;
    let mut run = command(Path::new("powershell"));
    run.args([
        "-NoProfile",
        "-NonInteractive",
        "-ExecutionPolicy",
        "Bypass",
        "-Command",
        script,
    ])
    .env("NERU_BIN_DIR", &bin);
    let out = run
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Could not run PowerShell: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "Could not update your Path: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(Some("Open a new terminal, then type neru.".into()))
}

/// macOS and Linux: ~/.local/bin/neru, a symlink to the bundled binary.
#[cfg(not(windows))]
fn link(bundled: &Path) -> Result<Option<String>, String> {
    let home = dirs::home_dir().ok_or("Could not find your home folder.")?;
    let bin = home.join(".local").join("bin");
    std::fs::create_dir_all(&bin)
        .map_err(|e| format!("Could not create {}: {e}", bin.display()))?;
    let target = bin.join("neru");
    if let Ok(meta) = std::fs::symlink_metadata(&target) {
        if !meta.file_type().is_symlink() {
            return Err(format!("{} already exists and is not a link to Neru. Remove it or install with one of the commands below.", target.display()));
        }
        std::fs::remove_file(&target)
            .map_err(|e| format!("Could not replace {}: {e}", target.display()))?;
    }
    std::os::unix::fs::symlink(bundled, &target)
        .map_err(|e| format!("Could not link {}: {e}", target.display()))?;
    let on_path = search_path().iter().any(|dir| dir == &bin);
    Ok(Some(if on_path {
        "Open a new terminal, then type neru.".into()
    } else {
        "~/.local/bin is not on your PATH. Add export PATH=\"$HOME/.local/bin:$PATH\" to your shell profile, then open a new terminal.".into()
    }))
}
