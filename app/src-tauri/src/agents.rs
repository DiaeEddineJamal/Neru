//! Custom sub-agents, the way Claude Code defines them: one Markdown file each in the project's
//! `.neru/agents/` or `.claude/agents/`, or in Neru's personal `agents` folder. The front matter
//! names the agent (`name`), says when to use it (`description`), lists its tools (`tools`, Neru
//! or Claude names) and may pick a `model`; the body is its system prompt. The `task` tool lists
//! them next to the built-in explore, plan and general agents. Edit and command tools are given only
//! when the `tools` line lists them; the user approves their calls as for the main agent.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::Serialize;
use serde_json::{Value, json};
use tauri::State;

use crate::{AppState, skills::front_value, workspace::data_dir};

/// Read-only tools a sub-agent can be given; an agent without a `tools` line gets all of them.
/// Edit and command tools (`subagent::WRITE_TOOLS`) must be listed by name.
pub const SUBAGENT_TOOLS: &[&str] = &[
    "list_directory", "read_file", "read_files", "search_text", "find_symbol", "find_files", "project_map", "git_status", "git_diff", "web_search", "fetch_url",
];

/// Largest agent file read, like skills.
const MAX_FILE: usize = 40_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentDef {
    pub name: String,
    pub description: String,
    /// The system prompt: the file's body.
    pub prompt: String,
    /// Neru tool names it gets.
    pub tools: Vec<String>,
    /// Tools it listed that a sub-agent cannot have (edits, commands, unknown names).
    pub ignored: Vec<String>,
    /// A model id to use instead of the session's; Claude's aliases (sonnet, opus, …) are ignored.
    pub model: Option<String>,
    /// "project" (.neru/agents), "claude" (.claude/agents) or "personal".
    pub source: String,
    pub path: PathBuf,
}

/// A custom agent as Settings and the CLI's `/agents` list it.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AgentView {
    pub name: String,
    pub description: String,
    pub source: String,
    pub tools: Vec<String>,
    pub ignored_tools: Vec<String>,
    pub model: Option<String>,
    pub path: String,
}

/// The Neru tools a tool name in an agent file stands for: Claude Code's names map to Neru's
/// equivalents, and Neru's own names stand for themselves. Unknown names map to nothing.
pub fn map_tool(name: &str) -> Vec<&'static str> {
    let known = |name: &str| -> Vec<&'static str> {
        const NERU: &[&str] = &[
            "list_directory", "read_file", "read_files", "search_text", "find_symbol", "find_files", "project_map", "git_status", "git_diff", "web_search", "fetch_url",
            "read_skill", "add_review_comment", "open_preview", "propose_edit", "propose_write_file", "propose_delete", "propose_move", "propose_create_folder",
            "run_project_task", "run_shell_command", "shell_output", "kill_shell", "check_preview", "update_todos", "task", "save_memory",
        ];
        NERU.iter().copied().filter(|tool| *tool == name).collect()
    };
    match name.trim().to_lowercase().as_str() {
        "read" | "notebookread" => vec!["read_file", "read_files"],
        "grep" => vec!["search_text", "find_symbol"],
        "glob" => vec!["find_files", "list_directory"],
        "ls" => vec!["list_directory"],
        "webfetch" => vec!["fetch_url"],
        "websearch" => vec!["web_search"],
        "edit" | "multiedit" | "notebookedit" => vec!["propose_edit"],
        "write" => vec!["propose_write_file"],
        "bash" => vec!["run_shell_command"],
        "bashoutput" => vec!["shell_output"],
        "killshell" | "killbash" => vec!["kill_shell"],
        "todowrite" => vec!["update_todos"],
        "agent" => vec!["task"],
        other => known(other),
    }
}

/// The names in a `tools` value: `Read, Grep`, `[Read, Grep]`, a YAML list, or with Claude's
/// permission patterns such as `Bash(git diff:*)`, which Neru does not use.
fn tool_names(value: &str) -> Vec<String> {
    let mut plain = String::new();
    let mut depth = 0usize;
    for c in value.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 => plain.push(c),
            _ => {}
        }
    }
    plain
        .split(|c: char| c == ',' || c.is_whitespace() || matches!(c, '[' | ']' | '"' | '\''))
        .map(|name| name.trim_start_matches('-'))
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

/// The tools an agent gets and the ones it asked for but cannot have. Without a `tools` line it
/// gets every read-only tool.
pub fn resolve_tools(value: Option<&str>) -> (Vec<String>, Vec<String>) {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty() && *value != "*") else {
        return (SUBAGENT_TOOLS.iter().map(|tool| tool.to_string()).collect(), Vec::new());
    };
    let (mut tools, mut ignored) = (Vec::<String>::new(), Vec::new());
    for name in tool_names(value) {
        let granted: Vec<&str> = map_tool(&name).into_iter().filter(|tool| SUBAGENT_TOOLS.contains(tool) || crate::subagent::WRITE_TOOLS.contains(tool)).collect();
        if granted.is_empty() {
            if !ignored.contains(&name) {
                ignored.push(name);
            }
            continue;
        }
        for tool in granted {
            if !tools.iter().any(|known| known == tool) {
                tools.push(tool.to_string());
            }
        }
    }
    (tools, ignored)
}

fn valid(name: &str) -> bool {
    !name.is_empty() && name.len() <= 64 && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// Reads one agent file's text. `stem` names it when the front matter does not.
pub fn parse(text: &str, stem: &str, source: &str, path: &Path) -> Option<AgentDef> {
    let text = text.trim_start_matches('\u{feff}');
    let (front, body) = match text.strip_prefix("---").and_then(|rest| rest.find("\n---").map(|end| (rest, end))) {
        Some((rest, end)) => {
            let body = rest[end + 4..].trim_start_matches(['-']).trim_start_matches(['\r', '\n']);
            (rest[..end].to_string(), body.trim().to_string())
        }
        None => (String::new(), text.trim().to_string()),
    };
    let name = front_value(&front, "name").unwrap_or_else(|| stem.to_string()).trim().to_lowercase().replace(' ', "-");
    if !valid(&name) || body.is_empty() {
        return None;
    }
    let (tools, ignored) = resolve_tools(front_value(&front, "tools").as_deref());
    let model = front_value(&front, "model")
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty() && !matches!(model.to_lowercase().as_str(), "inherit" | "default" | "sonnet" | "opus" | "haiku" | "fable"));
    Some(AgentDef {
        description: front_value(&front, "description").unwrap_or_else(|| format!("Custom agent {name}")),
        name,
        prompt: body,
        tools,
        ignored,
        model,
        source: source.into(),
        path: path.to_path_buf(),
    })
}

fn read_folder(folder: &Path, source: &str, into: &mut Vec<AgentDef>) {
    let Ok(entries) = fs::read_dir(folder) else { return };
    let mut files: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("md"))).collect();
    files.sort();
    for file in files {
        let Ok(text) = fs::read_to_string(&file) else { continue };
        if text.len() > MAX_FILE {
            continue;
        }
        let stem = file.file_stem().map(|stem| stem.to_string_lossy().to_lowercase()).unwrap_or_default();
        if let Some(agent) = parse(&text, &stem, source, &file) {
            // The first one found keeps the name: the project's before Claude's before personal.
            if !into.iter().any(|known| known.name == agent.name) {
                into.push(agent);
            }
        }
    }
}

/// Every custom agent for a project: `.neru/agents`, then `.claude/agents`, then personal ones.
pub fn definitions(root: &Path) -> Vec<AgentDef> {
    let mut agents = Vec::new();
    if !root.as_os_str().is_empty() {
        read_folder(&root.join(".neru").join("agents"), "project", &mut agents);
        read_folder(&root.join(".claude").join("agents"), "claude", &mut agents);
    }
    if let Ok(dir) = data_dir() {
        read_folder(&dir.join("agents"), "personal", &mut agents);
    }
    agents.sort_by(|a, b| a.name.cmp(&b.name));
    agents
}

/// The custom agent named `name` (any case), if there is one.
pub fn find(root: &Path, name: &str) -> Option<AgentDef> {
    let name = name.trim().to_lowercase();
    definitions(root).into_iter().find(|agent| agent.name == name)
}

fn view(agent: AgentDef) -> AgentView {
    AgentView {
        name: agent.name,
        description: agent.description,
        source: agent.source,
        tools: agent.tools,
        ignored_tools: agent.ignored,
        model: agent.model,
        path: agent.path.to_string_lossy().to_string(),
    }
}

/// The custom agents of a project, for the window and the CLI's `/agents`.
pub fn list_agents_for(root: &Path) -> Vec<AgentView> {
    definitions(root).into_iter().map(view).collect()
}

/// The custom agents of the open project, plus personal ones.
#[tauri::command]
pub fn list_agents(state: State<'_, AppState>) -> Vec<AgentView> {
    let root = state.root.lock().ok().and_then(|root| root.clone()).unwrap_or_default();
    list_agents_for(&root)
}

/// The tool list with the `task` tool told about this project's custom agents.
pub fn with_custom_agents(tools: Value, root: &Path) -> Value {
    let agents = definitions(root);
    add_agents(tools, &agents)
}

fn add_agents(mut tools: Value, agents: &[AgentDef]) -> Value {
    if agents.is_empty() {
        return tools;
    }
    let Some(task) = tools.as_array_mut().and_then(|tools| tools.iter_mut().find(|tool| tool["function"]["name"] == "task")) else { return tools };
    let lines = agents
        .iter()
        .take(40)
        .map(|agent| format!("- {}: {}", agent.name, agent.description.chars().take(300).collect::<String>()))
        .collect::<Vec<_>>()
        .join("\n");
    let description = format!(
        "{} Custom agents, also read-only; pick one with agent_type when its description fits the work:\n{lines}",
        task["function"]["description"].as_str().unwrap_or("")
    );
    task["function"]["description"] = json!(description);
    let kinds = &mut task["function"]["parameters"]["properties"]["agent_type"];
    let mut names: Vec<Value> = kinds["enum"].as_array().cloned().unwrap_or_default();
    for agent in agents.iter().take(40) {
        if !names.iter().any(|name| name == agent.name.as_str()) {
            names.push(json!(agent.name));
        }
    }
    kinds["enum"] = Value::Array(names);
    tools
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(text: &str) -> Option<AgentDef> {
        parse(text, "from-file", "project", Path::new("/p/.claude/agents/from-file.md"))
    }

    #[test]
    fn reads_claude_style_front_matter() {
        let text = "---\nname: code-reviewer\ndescription: Reviews diffs for bugs. Use after edits.\ntools: Read, Grep, Glob, Bash\nmodel: sonnet\n---\n\nYou are a senior reviewer.\nBe strict.\n";
        let agent = agent(text).unwrap();
        assert_eq!(agent.name, "code-reviewer");
        assert_eq!(agent.description, "Reviews diffs for bugs. Use after edits.");
        assert_eq!(agent.prompt, "You are a senior reviewer.\nBe strict.");
        assert_eq!(agent.tools, vec!["read_file", "read_files", "search_text", "find_symbol", "find_files", "list_directory", "run_shell_command"]);
        assert!(agent.ignored.is_empty());
        assert_eq!(agent.model, None, "Claude aliases are not model ids");
    }

    #[test]
    fn falls_back_to_the_file_name_and_read_only_tools() {
        let agent = agent("---\ndescription: \"Finds dead code\"\nmodel: gpt-5-mini\n---\nLook for unused exports.").unwrap();
        assert_eq!(agent.name, "from-file");
        assert_eq!(agent.model.as_deref(), Some("gpt-5-mini"));
        assert_eq!(agent.tools.len(), SUBAGENT_TOOLS.len());
        assert!(agent.ignored.is_empty());
        let plain = self::agent("Just a prompt with no front matter.").unwrap();
        assert_eq!((plain.name.as_str(), plain.prompt.as_str()), ("from-file", "Just a prompt with no front matter."));
        assert!(self::agent("---\nname: Bad Name!\n---\nbody").is_none());
        assert!(self::agent("---\nname: empty\n---\n").is_none());
    }

    #[test]
    fn tool_lists_in_every_shape() {
        assert_eq!(tool_names("[Read, \"Grep\"]"), vec!["Read", "Grep"]);
        assert_eq!(tool_names("- Read - WebSearch"), vec!["Read", "WebSearch"]);
        assert_eq!(tool_names("Bash(git diff:*), Read"), vec!["Bash", "Read"]);
        let text = "---\nname: web\ntools:\n  - WebSearch\n  - WebFetch\n  - read_file\n  - mcp__x__y\n---\nResearch.";
        let agent = agent(text).unwrap();
        assert_eq!(agent.tools, vec!["web_search", "fetch_url", "read_file"]);
        assert_eq!(agent.ignored, vec!["mcp__x__y"]);
    }

    #[test]
    fn maps_claude_names_to_neru_tools() {
        assert_eq!(map_tool("Read"), vec!["read_file", "read_files"]);
        assert_eq!(map_tool("glob"), vec!["find_files", "list_directory"]);
        assert_eq!(map_tool("Edit"), vec!["propose_edit"]);
        assert_eq!(map_tool("Bash"), vec!["run_shell_command"]);
        assert_eq!(map_tool("project_map"), vec!["project_map"]);
        assert!(map_tool("Teleport").is_empty());
        let (tools, ignored) = resolve_tools(Some("Edit, Write, Grep, TodoWrite"));
        assert_eq!(tools, vec!["propose_edit", "propose_write_file", "search_text", "find_symbol"]);
        assert_eq!(ignored, vec!["TodoWrite"]);
        assert_eq!(resolve_tools(Some("*")).0.len(), SUBAGENT_TOOLS.len());
    }

    #[test]
    fn task_tool_lists_custom_agents() {
        let tools: Value = serde_json::from_str(crate::extras::EXTRA_TOOLS).unwrap();
        let reviewer = agent("---\nname: reviewer\ndescription: Reviews code\n---\nReview.").unwrap();
        let tools = add_agents(tools, &[reviewer]);
        let task = tools.as_array().unwrap().iter().find(|tool| tool["function"]["name"] == "task").unwrap();
        assert!(task["function"]["description"].as_str().unwrap().contains("- reviewer: Reviews code"));
        let kinds = task["function"]["parameters"]["properties"]["agent_type"]["enum"].as_array().unwrap();
        assert!(kinds.iter().any(|kind| kind == "explore") && kinds.iter().any(|kind| kind == "reviewer"));
    }

    #[test]
    fn project_agents_win_over_personal_ones() {
        let dir = std::env::temp_dir().join(format!("neru-agents-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".neru").join("agents")).unwrap();
        fs::create_dir_all(dir.join(".claude").join("agents")).unwrap();
        fs::write(dir.join(".neru").join("agents").join("helper.md"), "---\nname: helper\ndescription: Neru one\n---\nA").unwrap();
        fs::write(dir.join(".claude").join("agents").join("helper.md"), "---\nname: helper\ndescription: Claude one\n---\nB").unwrap();
        fs::write(dir.join(".claude").join("agents").join("notes.txt"), "ignored").unwrap();
        let found: Vec<AgentDef> = definitions(&dir).into_iter().filter(|agent| agent.name == "helper").collect();
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].description.as_str(), found[0].source.as_str()), ("Neru one", "project"));
        assert_eq!(find(&dir, "HELPER").map(|agent| agent.prompt), Some("A".to_string()));
        let _ = fs::remove_dir_all(&dir);
    }
}
