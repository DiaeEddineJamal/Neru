//! Team members that are other coding agents: Claude Code, Codex, OpenCode, Gemini CLI and Cursor's
//! agent, each run headless on the user's own sign-in. Neru starts the CLI in the project, writes
//! the prompt to its stdin (no quoting through npm's `.cmd` shims) and turns the JSON lines it
//! prints into [`Event`]s. Neru never reads or stores their credentials.
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};

use serde::Serialize;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Claude,
    Codex,
    OpenCode,
    Gemini,
    Cursor,
    Qwen,
    Copilot,
    Amp,
    Droid,
    Goose,
    Crush,
    Aider,
    Auggie,
    Kiro,
    Continue,
}

pub const KINDS: [Kind; 15] = [
    Kind::Claude,
    Kind::Codex,
    Kind::OpenCode,
    Kind::Gemini,
    Kind::Cursor,
    Kind::Qwen,
    Kind::Copilot,
    Kind::Amp,
    Kind::Droid,
    Kind::Goose,
    Kind::Crush,
    Kind::Aider,
    Kind::Auggie,
    Kind::Kiro,
    Kind::Continue,
];

/// How a CLI takes the turn's prompt.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Input {
    Stdin,
    /// As an argument. Long prompts would overflow a Windows command line (npm shims go through
    /// cmd, 8 KB), so the argument points at a file holding the whole prompt.
    Arg,
}

impl Kind {
    pub fn parse(id: &str) -> Option<Kind> {
        KINDS.into_iter().find(|kind| kind.id() == id)
    }

    pub fn id(self) -> &'static str {
        match self {
            Kind::Claude => "claude",
            Kind::Codex => "codex",
            Kind::OpenCode => "opencode",
            Kind::Gemini => "gemini",
            Kind::Cursor => "cursor",
            Kind::Qwen => "qwen",
            Kind::Copilot => "copilot",
            Kind::Amp => "amp",
            Kind::Droid => "droid",
            Kind::Goose => "goose",
            Kind::Crush => "crush",
            Kind::Aider => "aider",
            Kind::Auggie => "auggie",
            Kind::Kiro => "kiro",
            Kind::Continue => "continue",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Claude => "Claude Code",
            Kind::Codex => "Codex",
            Kind::OpenCode => "OpenCode",
            Kind::Gemini => "Gemini CLI",
            Kind::Cursor => "Cursor Agent",
            Kind::Qwen => "Qwen Code",
            Kind::Copilot => "GitHub Copilot CLI",
            Kind::Amp => "Amp",
            Kind::Droid => "Factory Droid",
            Kind::Goose => "Goose",
            Kind::Crush => "Crush",
            Kind::Aider => "Aider",
            Kind::Auggie => "Auggie",
            Kind::Kiro => "Kiro CLI",
            Kind::Continue => "Continue CLI",
        }
    }

    fn program(self) -> &'static str {
        match self {
            Kind::Claude => "claude",
            Kind::Codex => "codex",
            Kind::OpenCode => "opencode",
            Kind::Gemini => "gemini",
            Kind::Cursor => "cursor-agent",
            Kind::Qwen => "qwen",
            Kind::Copilot => "copilot",
            Kind::Amp => "amp",
            Kind::Droid => "droid",
            Kind::Goose => "goose",
            Kind::Crush => "crush",
            Kind::Aider => "aider",
            Kind::Auggie => "auggie",
            Kind::Kiro => "kiro-cli",
            Kind::Continue => "cn",
        }
    }

    /// What to run in a terminal to sign in with the subscription.
    pub fn login(self) -> &'static str {
        match self {
            Kind::Claude => "claude auth login",
            Kind::Codex => "codex login",
            Kind::OpenCode => "opencode auth login",
            Kind::Gemini => "gemini",
            Kind::Cursor => "cursor-agent login",
            Kind::Qwen => "qwen",
            Kind::Copilot => "copilot",
            Kind::Amp => "amp login",
            Kind::Droid => "droid",
            Kind::Goose => "goose configure",
            Kind::Crush => "crush",
            Kind::Aider => "aider",
            Kind::Auggie => "auggie login",
            Kind::Kiro => "kiro-cli login",
            Kind::Continue => "cn login",
        }
    }

    pub fn install(self) -> &'static str {
        match self {
            Kind::Claude => "npm install -g @anthropic-ai/claude-code",
            Kind::Codex => "npm install -g @openai/codex",
            Kind::OpenCode => "npm install -g opencode-ai",
            Kind::Gemini => "npm install -g @google/gemini-cli",
            Kind::Cursor if cfg!(windows) => "irm 'https://cursor.com/install?win32=true' | iex",
            Kind::Cursor => "curl https://cursor.com/install -fsS | bash",
            Kind::Qwen => "npm install -g @qwen-code/qwen-code",
            Kind::Copilot => "npm install -g @github/copilot",
            Kind::Amp => "npm install -g @sourcegraph/amp",
            Kind::Droid if cfg!(windows) => "irm https://app.factory.ai/cli/windows | iex",
            Kind::Droid => "curl -fsSL https://app.factory.ai/cli | sh",
            Kind::Goose if cfg!(windows) => "winget install Block.Goose",
            Kind::Goose => "curl -fsSL https://github.com/block/goose/releases/download/stable/download_cli.sh | bash",
            Kind::Crush => "npm install -g @charmland/crush",
            Kind::Aider => "python -m pip install aider-install && aider-install",
            Kind::Auggie => "npm install -g @augmentcode/auggie",
            Kind::Kiro => "curl -fsSL https://cli.kiro.dev/install | bash",
            Kind::Continue => "npm install -g @continuedev/cli",
        }
    }

    /// The npm package behind the CLI, for version checks and updates.
    pub fn npm_package(self) -> Option<&'static str> {
        let install = self.install();
        install.strip_prefix("npm install -g ")
    }

    /// Whether the CLI can continue its own session headless. The plain-text CLIs cannot, so they
    /// get the whole thread every turn instead of only what they missed.
    pub fn resumes(self) -> bool {
        matches!(self, Kind::Claude | Kind::Codex | Kind::OpenCode | Kind::Gemini | Kind::Cursor | Kind::Qwen)
    }

    pub fn input(self) -> Input {
        match self {
            Kind::Copilot | Kind::Droid | Kind::Goose | Kind::Aider | Kind::Auggie | Kind::Kiro | Kind::Continue => Input::Arg,
            _ => Input::Stdin,
        }
    }

    /// Whether its output is JSON lines Neru parses; the others reply in plain text.
    pub fn streams_json(self) -> bool {
        matches!(self, Kind::Claude | Kind::Codex | Kind::OpenCode | Kind::Gemini | Kind::Cursor | Kind::Qwen | Kind::Amp)
    }
}

/// How much a member may change, from Neru's permission mode. Headless CLIs cannot stop to ask, so
/// "ask first" runs read-only. Auto lets the CLI's own reviewer decide each action where it has
/// one (Claude Code's auto mode, Codex's --approve-for-me); elsewhere it allows safe commands only.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Access {
    ReadOnly,
    Edits,
    Auto,
    Full,
}

impl Access {
    pub fn from_mode(mode: &str) -> Access {
        match mode {
            "bypass" => Access::Full,
            "auto" => Access::Auto,
            "accept_edits" => Access::Edits,
            _ => Access::ReadOnly,
        }
    }
}

/// Shell commands Auto allows on CLIs without a reviewer of their own: they read, build or test.
const SAFE_COMMANDS: [&str; 16] = [
    "git status", "git diff", "git log", "git show", "git branch", "ls", "dir", "cat", "rg", "grep", "npm test", "npm run", "cargo test",
    "cargo check", "pytest", "go test",
];

/// Settings → Agents choices that outlive a restart: a CLI path per agent.
#[derive(Serialize, serde::Deserialize, Default)]
struct Saved {
    #[serde(default)]
    paths: std::collections::HashMap<String, String>,
}

fn saved_file() -> Option<PathBuf> {
    crate::workspace::data_dir().ok().map(|dir| dir.join("team-agents.json"))
}

fn saved() -> Saved {
    saved_file().and_then(|file| std::fs::read(file).ok()).and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

/// Uses `path` for `kind` instead of the one on PATH, or forgets it with None.
pub fn set_path(kind: Kind, path: Option<String>) -> Result<(), String> {
    let mut stored = saved();
    match path.filter(|path| !path.trim().is_empty()) {
        Some(path) if Path::new(&path).is_file() => {
            stored.paths.insert(kind.id().into(), path);
        }
        Some(path) => return Err(format!("{path} is not a file")),
        None => {
            stored.paths.remove(kind.id());
        }
    }
    let file = saved_file().ok_or("No data folder")?;
    std::fs::write(file, serde_json::to_vec_pretty(&stored).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// The newest version on npm, for CLIs installed from it.
pub fn latest_version(kind: Kind) -> Option<String> {
    let package = kind.npm_package()?;
    let npm = crate::cli_setup::which("npm", &install_dirs())?;
    let mut command = crate::cli_setup::command(&npm);
    command.args(["view", package, "version"]);
    Some(crate::cli_setup::output_within(command, Duration::from_secs(15))?.trim().to_string()).filter(|version| !version.is_empty())
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Agent {
    pub kind: &'static str,
    pub name: &'static str,
    pub path: Option<String>,
    pub version: Option<String>,
    pub signed_in: bool,
    pub login: &'static str,
    pub install: &'static str,
    pub resumes: bool,
    /// Set in Settings → Agents rather than found on PATH.
    pub custom_path: bool,
    pub npm: Option<&'static str>,
}

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_default()
}

/// Folders installers use that a GUI app's PATH often lacks.
fn install_dirs() -> Vec<PathBuf> {
    let home = home();
    let mut dirs = vec![
        home.join(".local").join("bin"),
        home.join(".claude").join("local"),
        home.join(".opencode").join("bin"),
        home.join(".bun").join("bin"),
        home.join(".npm-global").join("bin"),
    ];
    if let Some(roaming) = dirs::data_dir() {
        dirs.push(roaming.join("npm"));
    }
    if cfg!(unix) {
        dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    }
    dirs
}

/// The CLI for `kind`: `NERU_TEAM_<KIND>` when set (tests and CI point it at a stub), then the
/// path chosen in Settings → Agents, else the PATH.
pub fn program(kind: Kind) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(format!("NERU_TEAM_{}", kind.id().to_uppercase())) {
        return Some(PathBuf::from(path));
    }
    if let Some(path) = saved().paths.get(kind.id()).map(PathBuf::from).filter(|path| path.is_file()) {
        return Some(path);
    }
    crate::cli_setup::which(kind.program(), &install_dirs())
}

fn signed_in(kind: Kind) -> bool {
    let home = home();
    let env = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    let has = |path: PathBuf, marker: &str| {
        std::fs::read_to_string(path).is_ok_and(|text| marker.is_empty() || text.contains(marker))
    };
    match kind {
        Kind::Claude => {
            env("ANTHROPIC_API_KEY")
                || home.join(".claude").join(".credentials.json").is_file()
                || has(home.join(".claude.json"), "oauthAccount")
        }
        Kind::Codex => {
            let codex = std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".codex"));
            env("OPENAI_API_KEY") || codex.join("auth.json").is_file()
        }
        // OpenCode's own free models need no account.
        Kind::OpenCode => true,
        // Google retired Gemini CLI's free personal sign-in: Google accounts work only with a Cloud
        // project. Neru's saved Gemini key counts too (see team::gemini_env).
        Kind::Gemini => {
            env("GEMINI_API_KEY")
                || env("GOOGLE_API_KEY")
                || (home.join(".gemini").join("oauth_creds.json").is_file() && (env("GOOGLE_CLOUD_PROJECT") || env("GOOGLE_CLOUD_PROJECT_ID")))
        }
        Kind::Cursor => env("CURSOR_API_KEY") || has(home.join(".cursor").join("cli-config.json"), "authInfo"),
        Kind::Qwen => env("OPENAI_API_KEY") || env("DASHSCOPE_API_KEY") || home.join(".qwen").join("oauth_creds.json").is_file(),
        Kind::Copilot => env("GH_TOKEN") || env("GITHUB_TOKEN") || home.join(".copilot").join("config.json").is_file(),
        Kind::Amp => env("AMP_API_KEY") || has(home.join(".local").join("share").join("amp").join("secrets.json"), ""),
        Kind::Droid => env("FACTORY_API_KEY") || home.join(".factory").join("auth.json").is_file(),
        Kind::Auggie => env("AUGMENT_SESSION_AUTH") || home.join(".augment").join("session.json").is_file(),
        // These run on whatever model keys they are configured with; Neru cannot tell from outside.
        Kind::Goose | Kind::Crush | Kind::Aider | Kind::Kiro | Kind::Continue => true,
    }
}

fn version(path: &Path) -> Option<String> {
    let mut command = crate::cli_setup::command(path);
    command.arg("--version");
    let text = crate::cli_setup::output_within(command, Duration::from_secs(4))?;
    Some(text.lines().find(|line| !line.trim().is_empty())?.trim().to_string())
}

/// Every supported CLI, installed or not. Blocking: it runs each one's `--version`.
pub fn detect() -> Vec<Agent> {
    // npm-installed CLIs take a second or more to print a version, so they all run at once.
    std::thread::scope(|scope| {
        let checks: Vec<_> = KINDS
            .into_iter()
            .map(|kind| {
                scope.spawn(move || {
                    let path = program(kind);
                    Agent {
                        kind: kind.id(),
                        name: kind.name(),
                        version: path.as_deref().and_then(version),
                        signed_in: path.is_some() && signed_in(kind),
                        path: path.map(|path| path.to_string_lossy().into_owned()),
                        login: kind.login(),
                        install: kind.install(),
                        resumes: kind.resumes(),
                        custom_path: saved().paths.contains_key(kind.id()),
                        npm: kind.npm_package(),
                    }
                })
            })
            .collect();
        checks.into_iter().filter_map(|check| check.join().ok()).collect()
    })
}

/// One turn of a member.
pub struct Turn<'a> {
    pub kind: Kind,
    pub model: &'a str,
    /// Reasoning effort ("low" … "max"), empty for the CLI's default.
    pub effort: &'a str,
    pub access: Access,
    pub resume: Option<&'a str>,
    /// Continue a copy of `resume` instead of the session itself (side chats, forked members).
    pub fork: bool,
    pub root: &'a Path,
    /// The task folder, where the shared thread and artifacts live; made writable where the CLI
    /// takes extra folders.
    pub task_dir: &'a Path,
    pub prompt: String,
    /// Extra environment for this CLI, e.g. Gemini's API key and its own home.
    pub env: Vec<(String, String)>,
    /// Neru's browser MCP server for this task: URL and bearer token (see team_mcp).
    pub mcp: Option<(String, String)>,
}

/// The MCP config file Claude Code reads for Neru's browser tools.
pub fn mcp_file(turn: &Turn) -> PathBuf {
    turn.task_dir.join("prompts").join("neru-mcp.json")
}

/// Claude Code's absolute-path rule form: `//d/work/x` for D:\work\x, `//home/x` for /home/x.
fn claude_rule_path(path: &Path) -> String {
    let text = path.to_string_lossy().trim_start_matches(r"\\?\").replace('\\', "/");
    match text.split_once(":/") {
        Some((drive, rest)) if drive.len() == 1 => format!("//{}/{rest}", drive.to_lowercase()),
        _ => format!("/{text}"),
    }
}

/// `--settings` for Claude members: every installed oh-my-claudecode plugin turned off.
fn quiet_plugins() -> String {
    let file = dirs::home_dir().unwrap_or_default().join(".claude").join("settings.json");
    let settings: Value = std::fs::read_to_string(file).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default();
    let off: serde_json::Map<String, Value> = settings["enabledPlugins"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(id, _)| id.starts_with("oh-my-claudecode@"))
        .map(|(id, _)| (id.clone(), Value::Bool(false)))
        .collect();
    if off.is_empty() { String::new() } else { serde_json::json!({ "enabledPlugins": off }).to_string() }
}

pub fn args(turn: &Turn) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    let mut push = |items: &[&str]| args.extend(items.iter().map(|item| item.to_string()));
    let task_dir = turn.task_dir.to_string_lossy();
    let model = turn.model.trim();
    match turn.kind {
        Kind::Claude => {
            // Partial messages stream the reply token by token instead of a block at a time.
            push(&["-p", "--output-format", "stream-json", "--verbose", "--include-partial-messages", "--add-dir", &task_dir]);
            let mcp = mcp_file(turn).to_string_lossy().into_owned();
            if turn.mcp.is_some() {
                push(&["--mcp-config", &mcp]);
            }
            // Plugins that write their own state into the project (oh-my-claudecode's .omc) are
            // off for a member's runs; the user's Claude Code keeps them.
            let quiet = quiet_plugins();
            if !quiet.is_empty() {
                push(&["--settings", &quiet]);
            }
            match turn.access {
                // Read the project, write only the task's artifacts; Neru's browser tools only look.
                Access::ReadOnly => {
                    let artifacts = format!("Edit({}/**)", claude_rule_path(turn.task_dir));
                    push(&["--permission-mode", "default", "--allowedTools", "Read", "Glob", "Grep", "LS", "WebFetch", "WebSearch", "TodoWrite", "mcp__neru", &artifacts]);
                }
                Access::Edits => push(&["--permission-mode", "acceptEdits"]),
                // Claude Code's own classifier approves or blocks each action.
                Access::Auto => push(&["--permission-mode", "auto"]),
                Access::Full => push(&["--permission-mode", "bypassPermissions"]),
            }
            // Neru's browser tools only start, show and read pages: allowed in every mode.
            if turn.mcp.is_some() && turn.access != Access::ReadOnly {
                push(&["--allowedTools", "mcp__neru"]);
            }
            if let Some(id) = turn.resume {
                push(&["--resume", id]);
                if turn.fork {
                    push(&["--fork-session"]);
                }
            }
            if !model.is_empty() {
                push(&["--model", model]);
            }
            if !turn.effort.is_empty() {
                push(&["--effort", turn.effort]);
            }
        }
        Kind::Codex => {
            push(&["exec", "--json", "--skip-git-repo-check"]);
            if let Some((url, _)) = &turn.mcp {
                push(&["-c", &format!("mcp_servers.neru.url=\"{url}\""), "-c", "mcp_servers.neru.bearer_token_env_var=\"NERU_MCP_TOKEN\""]);
            }
            match turn.access {
                // Its sandbox reads everywhere but writes only under -C: the task folder.
                Access::ReadOnly => push(&["-s", "workspace-write", "-C", &task_dir]),
                Access::Edits => push(&["-s", "workspace-write", "--add-dir", &task_dir]),
                // Codex's automatic reviewer decides each approval request.
                Access::Auto => push(&["--approve-for-me", "--add-dir", &task_dir]),
                Access::Full => push(&["--dangerously-bypass-approvals-and-sandbox"]),
            }
            if !model.is_empty() {
                push(&["-m", model]);
            }
            if !turn.effort.is_empty() {
                let effort = format!("model_reasoning_effort=\"{}\"", turn.effort);
                push(&["-c", &effort]);
            }
            if let Some(id) = turn.resume {
                push(&[if turn.fork { "fork" } else { "resume" }, id]);
            }
            push(&["-"]);
        }
        Kind::OpenCode => {
            push(&["run", "--format", "json"]);
            match turn.access {
                Access::ReadOnly => push(&["--agent", "plan"]),
                Access::Edits => {}
                // Approves what is not denied; Neru denies the destructive commands (see auto_env).
                Access::Auto | Access::Full => push(&["--auto"]),
            }
            if let Some(id) = turn.resume {
                push(&["--session", id]);
                if turn.fork {
                    push(&["--fork"]);
                }
            }
            if !model.is_empty() {
                push(&["--model", model]);
            }
        }
        Kind::Gemini => {
            let mode = match turn.access {
                Access::ReadOnly => "default",
                Access::Edits | Access::Auto => "auto_edit",
                Access::Full => "yolo",
            };
            push(&["--output-format", "stream-json", "--approval-mode", mode]);
            push(&["--include-directories", &task_dir]);
            if turn.access == Access::Auto {
                for command in SAFE_COMMANDS {
                    push(&["--allowed-tools", &format!("run_shell_command({command})")]);
                }
            }
            // Gemini CLI cannot copy a session; a fork starts fresh with the thread instead.
            if let Some(id) = turn.resume.filter(|_| !turn.fork) {
                push(&["--resume", id]);
            }
            if !model.is_empty() {
                push(&["--model", model]);
            }
        }
        Kind::Cursor => {
            push(&["-p", "--output-format", "stream-json"]);
            // Cursor Agent cannot copy a session; a fork starts fresh with the thread instead.
            let resume = if turn.fork { None } else { turn.resume };
            match turn.access {
                Access::ReadOnly => push(&["--mode", "ask"]),
                Access::Edits | Access::Auto => {}
                Access::Full => push(&["--force"]),
            }
            if let Some(id) = resume {
                push(&["--resume", id]);
            }
            if !model.is_empty() {
                push(&["--model", model]);
            }
        }
        // A Gemini CLI fork with the same flags and stream.
        Kind::Qwen => {
            let mode = match turn.access {
                Access::ReadOnly => "default",
                Access::Edits | Access::Auto => "auto-edit",
                Access::Full => "yolo",
            };
            push(&["--output-format", "stream-json", "--approval-mode", mode, "--include-directories", &task_dir]);
            if let Some(id) = turn.resume.filter(|_| !turn.fork) {
                push(&["--resume", id]);
            }
            if !model.is_empty() {
                push(&["--model", model]);
            }
        }
        Kind::Copilot => {
            push(&["-p", &instruction(turn), "--add-dir", &task_dir]);
            match turn.access {
                Access::ReadOnly => push(&["--allow-tool", "write", "--deny-tool", "shell"]),
                Access::Edits => push(&["--allow-tool", "write", "--deny-tool", "shell"]),
                Access::Auto => {
                    push(&["--allow-tool", "write"]);
                    for command in SAFE_COMMANDS {
                        push(&["--allow-tool", &format!("shell({command})")]);
                    }
                }
                Access::Full => push(&["--allow-all-tools"]),
            }
            if !model.is_empty() {
                push(&["--model", model]);
            }
        }
        // Amp's stream is Claude Code's format.
        Kind::Amp => {
            push(&["-x", "--stream-json"]);
            if turn.access == Access::Full {
                push(&["--dangerously-allow-all"]);
            }
        }
        Kind::Droid => {
            push(&["exec", &instruction(turn)]);
            match turn.access {
                Access::ReadOnly => {}
                Access::Edits => push(&["--auto", "low"]),
                Access::Auto => push(&["--auto", "medium"]),
                Access::Full => push(&["--auto", "high"]),
            }
            if !model.is_empty() {
                push(&["-m", model]);
            }
        }
        Kind::Goose => push(&["run", "--no-session", "-t", &instruction(turn)]),
        Kind::Crush => {
            push(&["run", "-q"]);
            if turn.access == Access::Full {
                push(&["--yolo"]);
            }
        }
        Kind::Aider => {
            push(&["--message", &instruction(turn), "--no-auto-commits", "--no-check-update", "--no-pretty"]);
            if turn.access == Access::ReadOnly {
                push(&["--dry-run"]);
            } else {
                push(&["--yes-always"]);
            }
            if !model.is_empty() {
                push(&["--model", model]);
            }
        }
        Kind::Auggie => {
            push(&["--print", &instruction(turn)]);
            if !model.is_empty() {
                push(&["--model", model]);
            }
        }
        Kind::Kiro => {
            push(&["chat", "--no-interactive"]);
            if turn.access == Access::Full {
                push(&["--trust-all-tools"]);
            }
            push(&[&instruction(turn)]);
        }
        Kind::Continue => {
            push(&["-p", &instruction(turn)]);
            if turn.access == Access::ReadOnly {
                push(&["--readonly"]);
            }
        }
    }
    args
}

/// The prompt file a CLI that takes its prompt as an argument is pointed at.
pub fn prompt_file(turn: &Turn) -> PathBuf {
    turn.task_dir.join("prompts").join(format!("{}.md", turn.kind.id()))
}

/// The short argument for [`Input::Arg`] CLIs: the whole prompt is in a file.
fn instruction(turn: &Turn) -> String {
    format!("Your full instructions for this turn are in {}. Read that file first and follow it exactly.", prompt_file(turn).display())
}

/// Environment for Auto (approvals configured by environment) and for Neru's browser tools.
fn auto_env(turn: &Turn) -> Vec<(String, String)> {
    let mut env = Vec::new();
    if let Some((_, token)) = &turn.mcp {
        env.push(("NERU_MCP_TOKEN".to_string(), token.clone()));
    }
    match (turn.kind, turn.access) {
        (Kind::OpenCode, access) => {
            let mut config = serde_json::json!({});
            if access == Access::Auto {
                config["permission"] = serde_json::json!({ "bash": { "rm -rf *": "deny", "rm -r *": "deny", "git push*": "deny", "git reset --hard*": "deny", "git clean*": "deny", "del /s*": "deny" } });
            }
            if let Some((url, token)) = &turn.mcp {
                config["mcp"] = serde_json::json!({ "neru": { "type": "remote", "url": url, "headers": { "Authorization": format!("Bearer {token}") } } });
            }
            if config.as_object().is_some_and(|config| !config.is_empty()) {
                env.push(("OPENCODE_CONFIG_CONTENT".into(), config.to_string()));
            }
        }
        (Kind::Goose, access) => env.push(("GOOSE_MODE".into(), if access == Access::ReadOnly { "chat" } else { "auto" }.into())),
        _ => {}
    }
    env
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The CLI's own session id, for resuming next turn.
    Session(String),
    /// A new block of reply text.
    Text(String),
    /// More of the current block (streamed CLIs).
    Delta(String),
    /// A tool the agent used, described for the thread.
    Step(String),
    Usage { input: u64, output: u64, cost: f64 },
    /// How much of a subscription window is used, 0–100.
    Limit { label: String, used: f64, resets_at: Option<u64> },
    /// The agent started generating an image.
    ImageStart,
    Error(String),
    Done,
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_string()
}

fn short(text: &str, limit: usize) -> String {
    let line = text.lines().find(|line| !line.trim().is_empty()).unwrap_or_default().trim();
    if line.chars().count() <= limit {
        return line.to_string();
    }
    format!("{}…", line.chars().take(limit).collect::<String>())
}

/// "Read src/main.rs", "Bash cargo test": a tool and the argument that says what it touched.
fn describe(tool: &str, input: &Value) -> String {
    let arg = ["file_path", "filePath", "path", "command", "pattern", "query", "url", "description", "prompt"]
        .iter()
        .find_map(|key| input.get(*key).and_then(Value::as_str))
        .map(|arg| short(arg, 90))
        .unwrap_or_default();
    format!("{tool} {arg}").trim().to_string()
}

/// Codex wraps commands in the shell it ran them with; the thread shows only the command.
fn shell_command(command: &str) -> String {
    let inner = ["-Command \"", "-c \"", "-lc \"", "-lc '", "-c '"]
        .iter()
        .find_map(|marker| command.find(marker).map(|at| &command[at + marker.len()..]))
        .map(|rest| rest.trim_end_matches(['"', '\'']))
        .unwrap_or(command);
    short(inner, 90)
}

fn u64_at(value: &Value, key: &str) -> u64 {
    value[key].as_u64().unwrap_or(0)
}

/// Claude Code and Cursor Agent print the same stream-json shapes.
/// `partial`: the CLI streams text deltas (Claude Code with --include-partial-messages), so the
/// whole text block in the `assistant` message that follows would repeat them.
fn parse_claude(value: &Value, events: &mut Vec<Event>, partial: bool) {
    match value["type"].as_str().unwrap_or_default() {
        "system" if value["subtype"] == "init" => {
            if let Some(id) = value["session_id"].as_str() {
                events.push(Event::Session(id.into()));
            }
        }
        "stream_event" => {
            let event = &value["event"];
            match event["type"].as_str().unwrap_or_default() {
                // A new text block: a paragraph break from the one before.
                "content_block_start" if event["content_block"]["type"] == "text" => events.push(Event::Text(String::new())),
                "content_block_delta" if event["delta"]["type"] == "text_delta" => events.push(Event::Delta(text(&event["delta"]["text"]))),
                _ => {}
            }
        }
        "assistant" => {
            for block in value["message"]["content"].as_array().into_iter().flatten() {
                match block["type"].as_str().unwrap_or_default() {
                    "text" if !partial && !text(&block["text"]).trim().is_empty() => events.push(Event::Text(text(&block["text"]))),
                    "tool_use" => events.push(Event::Step(describe(block["name"].as_str().unwrap_or("tool"), &block["input"]))),
                    _ => {}
                }
            }
        }
        // Cursor: {"type":"tool_call","subtype":"started","tool_call":{"readToolCall":{"args":{...}}}}
        "tool_call" if value["subtype"] == "started" => {
            if let Some((name, call)) = value["tool_call"].as_object().and_then(|calls| calls.iter().next()) {
                let name = name.trim_end_matches("ToolCall");
                events.push(Event::Step(describe(name, &call["args"])));
            }
        }
        "rate_limit_event" => {
            let info = &value["rate_limit_info"];
            let windows = info["unifiedWindows"].as_object();
            for (key, label) in [("five_hour", "5-hour"), ("seven_day", "Weekly")] {
                if let Some(window) = windows.and_then(|windows| windows.get(key)) {
                    if let Some(used) = window["utilization"].as_f64() {
                        events.push(Event::Limit { label: label.into(), used: used * 100.0, resets_at: window["resetsAt"].as_u64() });
                    }
                }
            }
        }
        "result" => {
            let usage = &value["usage"];
            events.push(Event::Usage {
                input: u64_at(usage, "input_tokens") + u64_at(usage, "cache_creation_input_tokens") + u64_at(usage, "cache_read_input_tokens"),
                output: u64_at(usage, "output_tokens"),
                cost: value["total_cost_usd"].as_f64().unwrap_or(0.0),
            });
            if value["is_error"].as_bool() == Some(true) {
                let message = value["result"].as_str().or(value["subtype"].as_str()).unwrap_or("The agent failed");
                events.push(Event::Error(message.into()));
            }
            events.push(Event::Done);
        }
        _ => {}
    }
}

fn parse_codex(value: &Value, events: &mut Vec<Event>) {
    let item = &value["item"];
    match (value["type"].as_str().unwrap_or_default(), item["type"].as_str().unwrap_or_default()) {
        ("thread.started", _) => {
            if let Some(id) = value["thread_id"].as_str() {
                events.push(Event::Session(id.into()));
            }
        }
        ("item.started", "command_execution") => {
            let command = text(&item["command"]);
            // Codex reads its imagegen skill right before it draws, the only sign of an image on the way.
            if command.contains("imagegen") {
                events.push(Event::ImageStart);
            }
            events.push(Event::Step(format!("Ran {}", shell_command(&command))));
        }
        ("item.completed", "agent_message") => events.push(Event::Text(text(&item["text"]))),
        ("item.completed", "file_change") => {
            for change in item["changes"].as_array().into_iter().flatten() {
                events.push(Event::Step(format!("Edited {}", text(&change["path"]))));
            }
        }
        ("item.completed", "mcp_tool_call") => events.push(Event::Step(format!("{} {}", text(&item["server"]), text(&item["tool"])))),
        ("item.completed", "web_search") => events.push(Event::Step(format!("Searched {}", short(&text(&item["query"]), 90)))),
        // Item errors are Codex's own warnings (a skills budget, a hook); the turn goes on.
        ("turn.completed", _) => {
            let usage = &value["usage"];
            events.push(Event::Usage { input: u64_at(usage, "input_tokens"), output: u64_at(usage, "output_tokens"), cost: 0.0 });
            events.push(Event::Done);
        }
        ("turn.failed", _) => events.push(Event::Error(text(&value["error"]["message"]))),
        ("error", _) => events.push(Event::Error(text(&value["message"]))),
        _ => {}
    }
}

fn parse_opencode(value: &Value, events: &mut Vec<Event>) {
    let part = &value["part"];
    if let Some(id) = value["sessionID"].as_str() {
        events.push(Event::Session(id.into()));
    }
    match value["type"].as_str().unwrap_or_default() {
        "text" if !text(&part["text"]).trim().is_empty() => events.push(Event::Text(text(&part["text"]))),
        "tool_use" => events.push(Event::Step(describe(part["tool"].as_str().unwrap_or("tool"), &part["state"]["input"]))),
        "step_finish" => {
            let tokens = &part["tokens"];
            events.push(Event::Usage {
                input: u64_at(tokens, "input") + u64_at(&tokens["cache"], "read"),
                output: u64_at(tokens, "output") + u64_at(tokens, "reasoning"),
                cost: part["cost"].as_f64().unwrap_or(0.0),
            });
        }
        "error" => {
            let error = &value["error"];
            let message = error["data"]["message"].as_str().or(error["message"].as_str()).or(error["name"].as_str());
            events.push(Event::Error(message.unwrap_or("OpenCode failed").into()));
        }
        _ => {}
    }
}

fn parse_gemini(value: &Value, events: &mut Vec<Event>) {
    match value["type"].as_str().unwrap_or_default() {
        "init" => {
            if let Some(id) = value["session_id"].as_str() {
                events.push(Event::Session(id.into()));
            }
        }
        "message" if value["role"] == "assistant" => {
            let content = text(&value["content"]);
            events.push(if value["delta"].as_bool() == Some(true) { Event::Delta(content) } else { Event::Text(content) });
        }
        "tool_use" => events.push(Event::Step(describe(value["tool_name"].as_str().unwrap_or("tool"), &value["parameters"]))),
        "error" => events.push(Event::Error(text(&value["message"]))),
        "result" => {
            let stats = &value["stats"];
            events.push(Event::Usage { input: u64_at(stats, "input_tokens"), output: u64_at(stats, "output_tokens"), cost: 0.0 });
            if value["status"] == "error" {
                events.push(Event::Error(text(&value["error"]["message"])));
            }
            events.push(Event::Done);
        }
        _ => {}
    }
}

/// The events in one line a CLI printed. Lines that are not JSON (banners, logs) mean nothing.
pub fn parse_line(kind: Kind, line: &str) -> Vec<Event> {
    let Ok(value) = serde_json::from_str::<Value>(line.trim()) else {
        return Vec::new();
    };
    let mut events = Vec::new();
    match kind {
        Kind::Claude | Kind::Cursor | Kind::Amp => parse_claude(&value, &mut events, kind == Kind::Claude),
        Kind::Codex => parse_codex(&value, &mut events),
        Kind::OpenCode => parse_opencode(&value, &mut events),
        Kind::Gemini | Kind::Qwen => parse_gemini(&value, &mut events),
        _ => {}
    }
    events
}

fn codex_home() -> PathBuf {
    std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".codex"))
}

/// The model a Codex member set to "Agent default" runs with. Left alone, Codex reads the model from
/// its config.toml, which the Codex desktop app may have set to a model this CLI does not know or a
/// ChatGPT sign-in cannot use ("gpt-6.1-sol … not supported when using Codex with a ChatGPT account").
/// So: that model when the CLI's own list (models_cache.json, the signed-in account's models) has it,
/// else the list's first pick. None when there is no list, which keeps Codex's own choice.
pub fn codex_default_model() -> Option<String> {
    let codex = codex_home();
    let cache: Value = std::fs::read(codex.join("models_cache.json")).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok())?;
    let mut listed: Vec<&Value> = cache["models"].as_array()?.iter().filter(|model| model["visibility"] != "hide" && model["slug"].is_string()).collect();
    listed.sort_by_key(|model| model["priority"].as_i64().unwrap_or(i64::MAX));
    let configured = std::fs::read_to_string(codex.join("config.toml")).ok().and_then(|config| {
        // The top-level `model = "…"`, before any [table].
        config.lines().take_while(|line| !line.trim_start().starts_with('[')).find_map(|line| {
            let (key, value) = line.split_once('=')?;
            (key.trim() == "model").then(|| value.trim().trim_matches('"').to_string())
        })
    });
    if let Some(model) = configured.filter(|model| listed.iter().any(|known| known["slug"] == model.as_str())) {
        return Some(model);
    }
    listed.first().and_then(|model| model["slug"].as_str()).map(String::from)
}

/// Images Codex's image tool saved during session `id` since `since` (ms): it writes them to
/// generated_images/<session>/ and its JSON stream does not mention them.
pub fn codex_images(id: &str, since: u64) -> Vec<PathBuf> {
    let mut images: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(codex_home().join("generated_images").join(id))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let modified = entry.metadata().ok()?.modified().ok()?;
            let ms = modified.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as u64;
            let path = entry.path();
            let image = matches!(path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(), Some("png" | "jpg" | "jpeg" | "webp"));
            (image && ms + 2_000 >= since).then_some((modified, path))
        })
        .collect();
    images.sort();
    images.into_iter().map(|(_, path)| path).collect()
}

/// Codex's subscription windows ("5-hour", "Weekly") from the newest `rate_limits` in the log of
/// session `id`: (label, percent used, reset time in seconds).
pub fn codex_limits(id: &str) -> Vec<(String, f64, Option<u64>)> {
    let codex = std::env::var_os("CODEX_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".codex"));
    let suffix = format!("{id}.jsonl");
    // Logs live in sessions/YYYY/MM/DD; a session that is being resumed is recent, so look at the
    // newest days first.
    let mut days: Vec<PathBuf> = Vec::new();
    for year in std::fs::read_dir(codex.join("sessions")).into_iter().flatten().flatten() {
        for month in std::fs::read_dir(year.path()).into_iter().flatten().flatten() {
            days.extend(std::fs::read_dir(month.path()).into_iter().flatten().flatten().map(|day| day.path()));
        }
    }
    days.sort();
    let Some(file) = days.iter().rev().take(60).find_map(|day| {
        std::fs::read_dir(day).ok()?.flatten().map(|entry| entry.path()).find(|path| path.to_string_lossy().ends_with(&suffix))
    }) else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(file) else { return Vec::new() };
    let Some(limits) = text.lines().rev().filter(|line| line.contains("\"rate_limits\"")).find_map(|line| {
        let value: Value = serde_json::from_str(line).ok()?;
        let limits = value["payload"]["rate_limits"].clone();
        limits.is_object().then_some(limits)
    }) else {
        return Vec::new();
    };
    ["primary", "secondary"]
        .iter()
        .filter_map(|key| {
            let window = &limits[*key];
            let used = window["used_percent"].as_f64()?;
            let label = match window["window_minutes"].as_u64() {
                Some(300) => "5-hour".to_string(),
                Some(10080) => "Weekly".to_string(),
                Some(minutes) if minutes % 1440 == 0 => format!("{}-day", minutes / 1440),
                Some(minutes) => format!("{}-hour", (minutes as f64 / 60.0).round()),
                None => key.to_string(),
            };
            Some((label, used, window["resets_at"].as_u64()))
        })
        .collect()
}

/// A plain sentence and a fix for failures whose raw text helps nobody.
pub fn explain(kind: Kind, raw: &str) -> Option<String> {
    let text = raw.to_lowercase();
    if kind == Kind::Gemini && (text.contains("no longer supported for gemini code assist") || text.contains("google_cloud_project")) {
        return Some("Google no longer lets Gemini CLI sign in with a free personal Google account. Add a free Gemini API key in Settings → Agents → Gemini CLI (get one at aistudio.google.com/apikey).".into());
    }
    if kind == Kind::Gemini && text.contains("api key not valid") {
        return Some("Google rejected the Gemini API key. Paste a new one in Settings → Agents → Gemini CLI.".into());
    }
    None
}

/// Runs one turn, calling `on` for each event, until the CLI exits or `cancel` fires.
pub async fn run(turn: Turn<'_>, cancel: Arc<tokio::sync::Notify>, mut on: impl FnMut(Event)) -> Result<(), String> {
    let kind = turn.kind;
    let program = program(kind).ok_or_else(|| format!("{} is not installed. Install it with: {}", kind.name(), kind.install()))?;
    if let (Kind::Claude, Some((url, token))) = (kind, &turn.mcp) {
        let file = mcp_file(&turn);
        std::fs::create_dir_all(file.parent().unwrap_or(turn.task_dir)).map_err(|e| e.to_string())?;
        let config = serde_json::json!({ "mcpServers": { "neru": { "type": "http", "url": url, "headers": { "Authorization": format!("Bearer {token}") } } } });
        std::fs::write(&file, config.to_string()).map_err(|e| e.to_string())?;
    }
    if kind.input() == Input::Arg {
        let file = prompt_file(&turn);
        std::fs::create_dir_all(file.parent().unwrap_or(turn.task_dir)).map_err(|e| e.to_string())?;
        std::fs::write(&file, &turn.prompt).map_err(|e| e.to_string())?;
    }
    let mut command = tokio::process::Command::new(&program);
    command
        .args(args(&turn))
        .envs(auto_env(&turn))
        .current_dir(turn.root)
        // Claude Code refuses to start inside another Claude Code session.
        .env_remove("CLAUDECODE")
        .env_remove("CLAUDE_CODE_ENTRYPOINT")
        .env("NO_COLOR", "1")
        // Gemini CLI refuses to run headless in a folder it has not been told to trust.
        .env("GEMINI_CLI_TRUST_WORKSPACE", "true")
        .envs(turn.env.iter().map(|(key, value)| (key.as_str(), value.as_str())))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = command.spawn().map_err(|e| format!("Could not start {}: {e}", kind.name()))?;
    if let Some(mut stdin) = child.stdin.take().filter(|_| kind.input() == Input::Stdin) {
        let prompt = turn.prompt.clone();
        tokio::spawn(async move {
            let _ = stdin.write_all(prompt.as_bytes()).await;
            let _ = stdin.shutdown().await;
        });
    }
    let stderr = child.stderr.take().map(|mut stream| {
        tokio::spawn(async move {
            let mut bytes = Vec::new();
            let _ = stream.read_to_end(&mut bytes).await;
            String::from_utf8_lossy(&bytes).into_owned()
        })
    });
    let mut lines = BufReader::new(child.stdout.take().ok_or("No output from the agent")?).lines();
    let (mut done, mut failed, mut said) = (false, None::<String>, false);
    // Plain-text CLIs: everything they print is the reply, streamed as it comes.
    loop {
        tokio::select! {
            line = lines.next_line() => match line {
                Ok(Some(line)) if !kind.streams_json() => {
                    said = said || !line.trim().is_empty();
                    on(Event::Delta(format!("{line}\n")));
                }
                Ok(Some(line)) => {
                    for event in parse_line(kind, &line) {
                        match &event {
                            Event::Done => done = true,
                            Event::Error(message) => failed = Some(message.clone()),
                            Event::Text(text) | Event::Delta(text) => said = said || !text.trim().is_empty(),
                            _ => {}
                        }
                        on(event);
                    }
                }
                _ => break,
            },
            _ = cancel.notified() => {
                crate::shells::kill_tree(&mut child);
                return Err("Stopped".into());
            }
        }
    }
    let status = child.wait().await.map_err(|e| e.to_string())?;
    let stderr = match stderr {
        Some(task) => task.await.unwrap_or_default(),
        None => String::new(),
    };
    if let Some(message) = failed {
        return Err(explain(kind, &message).unwrap_or(message));
    }
    if !status.success() && !said {
        if let Some(message) = explain(kind, &stderr) {
            return Err(message);
        }
        let tail: Vec<&str> = stderr.lines().filter(|line| !line.trim().is_empty()).collect();
        let tail = tail[tail.len().saturating_sub(6)..].join("\n");
        return Err(if tail.is_empty() { format!("{} exited with {status}", kind.name()) } else { tail });
    }
    if !done {
        on(Event::Done);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_stream_gives_session_text_steps_usage_and_limits() {
        let lines = [
            r#"{"type":"system","subtype":"init","cwd":"/w","session_id":"df94","tools":["Read"]}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Glob","input":{"pattern":"*"}}]}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"DO"}}}"#,
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"NE"}}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"DONE"}]}}"#,
            r#"{"type":"rate_limit_event","rate_limit_info":{"unifiedWindows":{"five_hour":{"utilization":0.14,"resetsAt":1790971200},"seven_day":{"utilization":0.85,"resetsAt":1791284400}}}}"#,
            r#"{"type":"result","subtype":"success","is_error":false,"result":"DONE","session_id":"df94","total_cost_usd":0.36,"usage":{"input_tokens":4,"cache_creation_input_tokens":10,"cache_read_input_tokens":6,"output_tokens":54}}"#,
            "not json",
        ];
        let events: Vec<Event> = lines.iter().flat_map(|line| parse_line(Kind::Claude, line)).collect();
        assert_eq!(events[0], Event::Session("df94".into()));
        assert_eq!(events[1], Event::Step("Glob *".into()));
        // Streamed: a block start, then the deltas; the finished block is not sent again.
        assert_eq!(events[2..5], [Event::Text(String::new()), Event::Delta("DO".into()), Event::Delta("NE".into())]);
        assert!(matches!(&events[5], Event::Limit { label, used, .. } if label == "5-hour" && (*used - 14.0).abs() < 0.01));
        assert!(matches!(&events[6], Event::Limit { label, .. } if label == "Weekly"));
        assert_eq!(events[7], Event::Usage { input: 20, output: 54, cost: 0.36 });
        assert_eq!(events[8], Event::Done);
        // Cursor shares the format without deltas, so its finished blocks still count.
        assert_eq!(parse_line(Kind::Cursor, lines[5]), vec![Event::Text("DONE".into())]);
        assert_eq!(events.len(), 9);
    }

    #[test]
    fn claude_errors_are_reported() {
        let line = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"result":"Claude AI usage limit reached","usage":{}}"#;
        assert!(parse_line(Kind::Claude, line).contains(&Event::Error("Claude AI usage limit reached".into())));
    }

    #[test]
    fn codex_stream_gives_thread_commands_and_messages() {
        let lines = [
            r#"{"type":"thread.started","thread_id":"01a0"}"#,
            r#"{"type":"item.started","item":{"id":"item_1","type":"command_execution","command":"\"C:\\pwsh.exe\" -Command \"Get-ChildItem -Force\"","status":"in_progress"}}"#,
            r#"{"type":"item.completed","item":{"id":"item_2","type":"agent_message","text":"DONE"}}"#,
            r#"{"type":"item.completed","item":{"id":"item_3","type":"file_change","changes":[{"path":"src/a.rs","kind":"update"}]}}"#,
            r#"{"type":"turn.completed","usage":{"input_tokens":112,"cached_input_tokens":94,"output_tokens":7}}"#,
        ];
        let events: Vec<Event> = lines.iter().flat_map(|line| parse_line(Kind::Codex, line)).collect();
        assert_eq!(
            events,
            vec![
                Event::Session("01a0".into()),
                Event::Step("Ran Get-ChildItem -Force".into()),
                Event::Text("DONE".into()),
                Event::Step("Edited src/a.rs".into()),
                Event::Usage { input: 112, output: 7, cost: 0.0 },
                Event::Done,
            ]
        );
        assert_eq!(parse_line(Kind::Codex, r#"{"type":"turn.failed","error":{"message":"You've hit your usage limit"}}"#), vec![Event::Error("You've hit your usage limit".into())]);
    }

    #[test]
    fn opencode_stream_gives_session_tools_text_and_cost() {
        let tool = r#"{"type":"tool_use","sessionID":"ses_1","part":{"type":"tool","tool":"glob","state":{"status":"completed","input":{"path":"D:\\w","pattern":"*"}}}}"#;
        assert_eq!(parse_line(Kind::OpenCode, tool), vec![Event::Session("ses_1".into()), Event::Step("glob D:\\w".into())]);
        let text = r#"{"type":"text","sessionID":"ses_1","part":{"type":"text","text":"OK"}}"#;
        assert_eq!(parse_line(Kind::OpenCode, text)[1], Event::Text("OK".into()));
        let finish = r#"{"type":"step_finish","sessionID":"ses_1","part":{"cost":0.01,"tokens":{"input":17,"output":2,"reasoning":1,"cache":{"read":3,"write":0}}}}"#;
        assert_eq!(parse_line(Kind::OpenCode, finish)[1], Event::Usage { input: 20, output: 3, cost: 0.01 });
    }

    #[test]
    fn gemini_and_cursor_streams_are_read() {
        let delta = r#"{"type":"message","role":"assistant","content":"Hel","delta":true}"#;
        assert_eq!(parse_line(Kind::Gemini, delta), vec![Event::Delta("Hel".into())]);
        let tool = r#"{"type":"tool_call","subtype":"started","tool_call":{"readToolCall":{"args":{"path":"README.md"}}}}"#;
        assert_eq!(parse_line(Kind::Cursor, tool), vec![Event::Step("read README.md".into())]);
    }

    #[tokio::test]
    async fn a_stub_cli_runs_and_its_stream_is_read() {
        let dir = std::env::temp_dir().join(format!("neru-team-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let fixture = dir.join("out.jsonl");
        std::fs::write(
            &fixture,
            "{\"type\":\"thread.started\",\"thread_id\":\"t9\"}\n{\"type\":\"item.completed\",\"item\":{\"type\":\"agent_message\",\"text\":\"hi from the stub\"}}\n{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":3,\"output_tokens\":2}}\n",
        )
        .unwrap();
        let stub = if cfg!(windows) {
            let stub = dir.join("codex.cmd");
            std::fs::write(&stub, format!("@type \"{}\"\r\n", fixture.display())).unwrap();
            stub
        } else {
            let stub = dir.join("codex");
            std::fs::write(&stub, format!("#!/bin/sh\ncat >/dev/null\ncat '{}'\n", fixture.display())).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            stub
        };
        // SAFETY: no other test reads NERU_TEAM_CODEX.
        unsafe { std::env::set_var("NERU_TEAM_CODEX", &stub) };
        let turn = Turn { kind: Kind::Codex, model: "", effort: "", access: Access::ReadOnly, resume: None, fork: false, root: &dir, task_dir: &dir, prompt: "hello".into(), env: Vec::new(), mcp: None };
        let mut events = Vec::new();
        run(turn, Arc::new(tokio::sync::Notify::new()), |event| events.push(event)).await.unwrap();
        assert_eq!(events[0], Event::Session("t9".into()));
        assert_eq!(events[1], Event::Text("hi from the stub".into()));
        assert_eq!(events.last(), Some(&Event::Done));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn arguments_follow_access_and_resume() {
        let root = Path::new("/w");
        let task = Path::new("/t");
        let turn = |kind, access, resume| Turn { kind, model: "", effort: "", access, resume, fork: false, root, task_dir: task, prompt: String::new(), env: Vec::new(), mcp: None };
        let claude = args(&turn(Kind::Claude, Access::ReadOnly, Some("s1"))).join(" ");
        assert!(claude.contains("--permission-mode default") && claude.contains("Edit(//t/**)") && claude.contains("--resume s1") && claude.contains("--add-dir /t"));
        assert!(args(&turn(Kind::Codex, Access::ReadOnly, None)).join(" ").contains("-s workspace-write -C /t"));
        assert_eq!(claude_rule_path(Path::new(r"D:\Neru\task")), "//d/Neru/task");
        let codex = args(&turn(Kind::Codex, Access::Edits, Some("t1"))).join(" ");
        assert!(codex.contains("-s workspace-write") && codex.ends_with("resume t1 -"));
        assert!(args(&turn(Kind::Codex, Access::Full, None)).contains(&"--dangerously-bypass-approvals-and-sandbox".to_string()));
        // A member's effort goes to the CLI the way it takes it.
        let effort = |kind| args(&Turn { model: "m", effort: "high", ..turn(kind, Access::Edits, None) }).join(" ");
        assert!(effort(Kind::Claude).contains("--model m --effort high"));
        assert!(effort(Kind::Codex).contains("-m m -c model_reasoning_effort=\"high\""));
        let fork = |kind| args(&Turn { fork: true, ..turn(kind, Access::Edits, Some("s9")) }).join(" ");
        assert!(fork(Kind::Claude).contains("--resume s9 --fork-session"));
        assert!(fork(Kind::Codex).ends_with("fork s9 -"));
        assert!(fork(Kind::OpenCode).contains("--session s9 --fork"));
        assert!(!fork(Kind::Cursor).contains("s9"));
        assert!(args(&turn(Kind::OpenCode, Access::Full, Some("ses"))).join(" ").contains("--auto --session ses"));
        assert!(!args(&turn(Kind::OpenCode, Access::Edits, None)).contains(&"--auto".to_string()));
        assert!(args(&turn(Kind::Gemini, Access::Full, None)).join(" ").contains("--approval-mode yolo"));
        let gemini = args(&turn(Kind::Gemini, Access::Auto, Some("g1"))).join(" ");
        assert!(gemini.contains("--approval-mode auto_edit") && gemini.contains("--resume g1") && gemini.contains("run_shell_command(git status)"));
        assert!(!fork(Kind::Gemini).contains("s9"));
        assert!(args(&turn(Kind::Claude, Access::Auto, None)).join(" ").contains("--permission-mode auto"));
        assert!(args(&turn(Kind::Codex, Access::Auto, None)).join(" ").contains("--approve-for-me"));
        // Prompt-as-argument CLIs get a pointer to the prompt file, never the prompt itself.
        let copilot = args(&Turn { prompt: "x".repeat(20_000), ..turn(Kind::Copilot, Access::Full, None) });
        assert!(copilot.iter().all(|arg| arg.len() < 400) && copilot.join(" ").contains("prompts") && copilot.contains(&"--allow-all-tools".to_string()));
        assert_eq!(Access::from_mode("auto"), Access::Auto);
    }

    #[test]
    fn codex_windows_come_from_its_session_log() {
        let home = std::env::temp_dir().join(format!("neru-codex-{}", uuid::Uuid::new_v4()));
        let day = home.join("sessions").join("2026").join("10").join("02");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(
            day.join("rollout-2026-10-02T18-01-16-abc-123.jsonl"),
            "{\"type\":\"event_msg\",\"payload\":{\"type\":\"token_count\",\"rate_limits\":{\"primary\":{\"used_percent\":7.0,\"window_minutes\":300,\"resets_at\":100},\"secondary\":{\"used_percent\":40.0,\"window_minutes\":10080,\"resets_at\":200}}}}\n",
        )
        .unwrap();
        // SAFETY: no other test reads CODEX_HOME.
        unsafe { std::env::set_var("CODEX_HOME", &home) };
        let limits = codex_limits("abc-123");
        unsafe { std::env::remove_var("CODEX_HOME") };
        assert_eq!(limits, vec![("5-hour".to_string(), 7.0, Some(100)), ("Weekly".to_string(), 40.0, Some(200))]);
        let _ = std::fs::remove_dir_all(home);
    }

    #[tokio::test]
    async fn a_plain_text_cli_replies_with_what_it_prints() {
        let dir = std::env::temp_dir().join(format!("neru-team-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        // The stub prints its prompt file, which proves the file was written and pointed at.
        let file = dir.join("prompts").join("aider.md");
        let stub = if cfg!(windows) {
            let stub = dir.join("aider.cmd");
            std::fs::write(&stub, format!("@echo off\r\necho plain reply\r\ntype \"{}\"\r\n", file.display())).unwrap();
            stub
        } else {
            let stub = dir.join("aider");
            std::fs::write(&stub, format!("#!/bin/sh\necho plain reply\ncat '{}'\n", file.display())).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            stub
        };
        // SAFETY: no other test reads NERU_TEAM_AIDER.
        unsafe { std::env::set_var("NERU_TEAM_AIDER", &stub) };
        let turn = Turn { kind: Kind::Aider, model: "", effort: "", access: Access::Edits, resume: None, fork: false, root: &dir, task_dir: &dir, prompt: "the whole prompt".into(), env: Vec::new(), mcp: None };
        let mut text = String::new();
        run(turn, Arc::new(tokio::sync::Notify::new()), |event| if let Event::Delta(delta) = event { text.push_str(&delta) }).await.unwrap();
        assert!(text.contains("plain reply") && text.contains("the whole prompt"), "{text}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
