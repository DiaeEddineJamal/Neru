mod agent;
mod agents;
mod cli;
mod cli_ui;
mod cli_setup;
mod commands;
mod documents;
mod extras;
mod fallback;
mod git;
mod hooks;
mod imports;
mod index;
mod limits;
mod mcp;
mod models;
mod oauth;
mod policy;
mod preview;
mod preview_proxy;
mod providers;
mod run_options;
mod sandbox;
mod sessions;
mod settings;
mod shells;
mod skills;
mod styles;
mod stream;
mod subagent;
mod tasks;
mod team;
mod team_agents;
mod team_mcp;
mod team_more;
mod team_tools;
mod terminal;
mod tools;
mod trust;
mod voice;
mod web;
mod workspace;

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use serde_json::Value;

#[derive(Clone)]
pub struct ProviderConfig {
    pub provider_id: String,
    pub api_format: String,
    pub base_url: String,
    pub model: String,
    pub api_key: String,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum PendingAction {
    Edit {
        proposal: workspace::EditProposal,
        tool_call_id: Option<String>,
    },
    Task {
        task: String,
        tool_call_id: Option<String>,
    },
    Command {
        command: String,
        tool_call_id: Option<String>,
        /// Runs detached (a dev server, a watcher); the agent reads its output with shell_output.
        #[serde(default)]
        background: bool,
        /// Seconds before the command is stopped; None keeps the default limit.
        #[serde(default)]
        timeout_seconds: Option<u64>,
        /// The model asked to run it outside the OS sandbox (sandbox.rs); the user always approves.
        #[serde(default)]
        outside_sandbox: bool,
    },
    /// A tool on a connected MCP server.
    Mcp {
        server: String,
        tool: String,
        arguments: Value,
        tool_call_id: Option<String>,
    },
    /// A plan from exit_plan_mode, waiting for the user to approve it or ask for changes.
    Plan {
        plan: String,
        tool_call_id: Option<String>,
    },
    /// Questions from ask_user_question, waiting for the user's answers.
    Question {
        questions: Vec<extras::Question>,
        tool_call_id: Option<String>,
    },
}

impl PendingAction {
    pub fn tool_call_id(&self) -> Option<&str> {
        match self {
            Self::Edit { tool_call_id, .. }
            | Self::Task { tool_call_id, .. }
            | Self::Command { tool_call_id, .. }
            | Self::Mcp { tool_call_id, .. }
            | Self::Plan { tool_call_id, .. }
            | Self::Question { tool_call_id, .. } => tool_call_id.as_deref(),
        }
    }
}

pub struct AppState {
    pub root: Mutex<Option<PathBuf>>,
    pub provider: Mutex<ProviderConfig>,
    pub provider_keys: Mutex<HashMap<String, String>>,
    pub voice: Mutex<voice::VoiceConfig>,
    /// Loaded session runtimes by id; several may be running at once.
    pub sessions: sessions::SessionMap,
    /// The session shown in the window.
    pub active_session: Mutex<Option<String>>,
    pub terminals: terminal::TerminalMap,
    pub mcp: mcp::McpManager,
    pub preview: preview::PreviewManager,
}

impl Default for AppState {
    fn default() -> Self {
        let last_project = workspace::data_dir()
            .ok()
            .and_then(|dir| std::fs::read_to_string(dir.join("recent-projects.json")).ok())
            .and_then(|text| serde_json::from_str::<Vec<String>>(&text).ok())
            .and_then(|paths| {
                paths.into_iter().find_map(|path| {
                    std::path::PathBuf::from(path)
                        .canonicalize()
                        .ok()
                        .filter(|path| path.is_dir())
                })
            });
        let mut provider = ProviderConfig {
            provider_id: "local".into(),
            api_format: "openai-chat".into(),
            base_url: "http://localhost:3001/v1".into(),
            model: "auto".into(),
            api_key: String::new(),
        };
        let mut keys = HashMap::new();
        let mut voice = voice::VoiceConfig::default();
        settings::load(&mut provider, &mut keys, &mut voice);
        let state = Self {
            root: Mutex::new(last_project.clone()),
            provider: Mutex::new(provider),
            provider_keys: Mutex::new(keys),
            voice: Mutex::new(voice),
            sessions: Mutex::new(HashMap::new()),
            active_session: Mutex::new(None),
            terminals: Arc::new(Mutex::new(Default::default())),
            mcp: mcp::McpManager::default(),
            preview: preview::PreviewManager::default(),
        };
        if let Some(root) = last_project {
            let _ = sessions::restore_for_root(&state, &root);
        }
        state
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    builder(|app| {
        fit_main_window(&app);
        tauri::async_runtime::spawn(mcp::start_enabled(app));
    })
    .build(context())
    .expect("error while building Neru")
    .run(|_, event| {
        // Background shells would outlive the window otherwise; statics are never dropped.
        if let tauri::RunEvent::Exit = event {
            shells::kill_all();
        }
    });
}

/// Shrinks the main window to the screen it opens on when the configured size would not fit, so small
/// laptop screens still show the whole window, then centers it.
fn fit_main_window(app: &tauri::AppHandle) {
    use tauri::Manager;
    let Some(window) = app.get_webview_window("main") else { return };
    let (Ok(Some(monitor)), Ok(size)) = (window.current_monitor(), window.outer_size()) else { return };
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let (area_w, area_h) = (area.size.width as f64 / scale, area.size.height as f64 / scale);
    let (width, height) = (size.width as f64 / scale, size.height as f64 / scale);
    let (max_w, max_h) = (area_w * 0.92, area_h * 0.92);
    if width <= max_w && height <= max_h {
        return;
    }
    let fitted = tauri::LogicalSize::new(width.min(max_w).max(640.0_f64.min(area_w)), height.min(max_h).max(480.0_f64.min(area_h)));
    let _ = window.set_size(fitted);
    let x = area.position.x + ((area_w - fitted.width).max(0.0) * scale / 2.0) as i32;
    let y = area.position.y + ((area_h - fitted.height).max(0.0) * scale / 2.0) as i32;
    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
}

/// The terminal version (`neru` on the command line): the same core with no window.
pub fn run_cli() {
    cli::run();
}

/// App configuration shared by the window and the CLI.
pub(crate) fn context() -> tauri::Context<tauri::Wry> {
    let mut context = tauri::generate_context!();
    #[cfg(windows)]
    {
        use tauri::utils::config::AppDirectoriesOverride;

        let project_data = PathBuf::from(r"D:\Neru\.local\data");
        let data_dir = std::env::var_os("NERU_DATA_DIR")
            .map(PathBuf::from)
            .or_else(|| {
                project_data
                    .parent()
                    .is_some_and(|p| p.is_dir())
                    .then_some(project_data)
            });
        if let Some(data_dir) = data_dir {
            context.config_mut().app.app_directories_override =
                Some(AppDirectoriesOverride::Root(data_dir.join("tauri")));
        }
    }

    context
}

/// Windows gives every console program a GUI app starts its own console window, which flashes open
/// and shut (git, gh, PowerShell, cmd). `.hidden()` starts it without one; elsewhere it does nothing.
pub(crate) trait Hidden {
    fn hidden(self) -> Self;
}

impl Hidden for std::process::Command {
    #[allow(unused_mut)]
    fn hidden(mut self) -> Self {
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(&mut self, 0x0800_0000); // CREATE_NO_WINDOW
        self
    }
}

impl Hidden for tokio::process::Command {
    #[allow(unused_mut)]
    fn hidden(mut self) -> Self {
        #[cfg(windows)]
        self.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        self
    }
}

/// Everything but the window: plugins, state and commands. `ready` runs once the app is set up.
pub(crate) fn builder(ready: impl FnOnce(tauri::AppHandle) + Send + 'static) -> tauri::Builder<tauri::Wry> {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(AppState::default())
        .setup(move |app| {
            use tauri::Manager;
            skills::set_bundled_dir(app.path().resource_dir().ok());
            index::set_app(app.handle().clone());
            if let Some(root) = app.state::<AppState>().root.lock().ok().and_then(|root| root.clone()) {
                // Start indexing the last project now, so the first search or chat finds it ready.
                index::start(&root);
            }
            ready(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            workspace::open_project,
            workspace::current_project,
            workspace::recent_projects,
            workspace::list_directory,
            workspace::read_file,
            workspace::search_text,
            workspace::list_project_files,
            index::index_status,
            index::index_refresh,
            index::search_files,
            workspace::propose_file,
            workspace::apply_pending,
            workspace::reject_pending,
            workspace::restore_checkpoint,
            sessions::list_sessions,
            sessions::list_all_sessions,
            sessions::current_session,
            sessions::create_session,
            sessions::create_chat_session,
            sessions::fork_session,
            sessions::select_session,
            sessions::rename_session,
            sessions::delete_session,
            sessions::session_snapshot,
            sessions::rewind_session,
            git::git_status,
            git::clone_project,
            git::git_diff,
            git::git_stage,
            git::git_unstage,
            git::git_commit,
            git::git_branches,
            git::git_create_branch,
            git::git_remote_info,
            git::git_push,
            git::create_pull_request,
            git::merge_pull_request,
            git::git_fetch,
            git::git_pull,
            git::git_checkout,
            git::git_merge,
            git::git_rebase,
            git::git_stash,
            git::pull_request_status,
            git::failed_check_log,
            preview::preview_hint,
            preview::preview_start,
            preview::preview_current,
            preview::preview_route,
            preview::preview_stop,
            workspace::save_file,
            workspace::open_in_editor,
            workspace::forget_project,
            workspace::reveal_path,
            workspace::file_action,
            commands::list_commands,
            commands::expand_command,
            terminal::terminal_start,
            terminal::terminal_write,
            terminal::terminal_resize,
            terminal::terminal_stop,
            agent::configure_provider,
            agent::provider_status,
            agent::new_chat,
            models::list_models,
            models::probe_model,
            subagent::answer_subagent_approval,
            models::check_models,
            models::cancel_check_models,
            agent::ai_chat,
            documents::inspect_documents,
            agent::stop_chat,
            agent::run_pending_task,
            agent::steer_session,
            agent::memory_files,
            agent::permission_rules,
            agent::revoke_permission,
            extras::doctor,
            extras::resolve_plan,
            extras::answer_question,
            extras::open_memory_file,
            agent::run_task_command,
            agent::allow_pending_always,
            agent::session_context,
            agent::compact_session,
            agent::refresh_context,
            agent::auto_title_session,
            skills::list_skills,
            skills::import_skills,
            skills::remove_skill,
            skills::open_skills_folder,
            skills::set_skill_enabled,
            agents::list_agents,
            shells::list_shells,
            shells::stop_shell,
            workspace::session_changes,
            agent::effort_supported,
            documents::document_bytes,
            documents::read_image,
            mcp::mcp_servers,
            mcp::mcp_save_server,
            mcp::mcp_remove_server,
            mcp::mcp_restart,
            mcp::mcp_sign_in,
            mcp::mcp_sign_out,
            mcp::mcp_sync_project,
            trust::commands::project_trust_status,
            trust::commands::trust_project,
            trust::commands::untrust_project,
            team::commands::list_team_agents,
            team::commands::list_team_tasks,
            team::commands::create_team_task,
            team::commands::team_attach,
            team::commands::team_snapshot,
            team::commands::add_team_member,
            team::commands::update_team_member,
            team::commands::remove_team_member,
            team::commands::set_team_worktree,
            team::commands::rename_team_task,
            team::commands::set_team_routing,
            team::commands::delete_team_task,
            team::commands::send_team_message,
            team::commands::stop_team,
            team::commands::cancel_team_routing,
            team::commands::list_team_artifacts,
            team::commands::read_team_artifact,
            team::commands::write_team_artifact,
            team::commands::team_agent_login,
            team::commands::sweep_team_worktrees,
            team::commands::undo_team_turn,
            team::commands::fork_team_member,
            team::commands::ask_team_side,
            team::commands::team_open_terminal,
            team::commands::set_team_pinned,
            team::commands::set_team_labels,
            team::commands::search_team,
            team::commands::list_custom_agents,
            team::commands::set_gemini_key,
            team::commands::team_cleanup_info,
            team::commands::set_team_appearance,
            team::commands::update_team_queue,
            team::commands::rerun_team_setup,
            team_more::commands::team_task_changes,
            team_more::commands::team_pr_status,
            team_more::commands::team_failed_log,
            team_more::commands::team_task_cwd,
            team_more::commands::team_terminal_launch,
            team_more::commands::list_agent_models,
            team_more::commands::list_agent_commands,
            team_more::commands::set_agent_path,
            team_more::commands::agent_latest_version,
            team_more::commands::install_agent,
            team_more::commands::set_team_execution,
            team_more::commands::run_team_execution,
            team_more::commands::stop_team_execution,
            team_more::commands::export_team_usage,
            team::commands::set_team_artifact_status,
            team::commands::export_team_artifacts,
            imports::scan_imports,
            imports::import_chats,
            imports::import_rules,
            imports::import_servers,
            voice::voice_status,
            voice::configure_voice,
            settings::forget_keys,
            settings::saved_key_providers,
            voice::transcribe_audio,
            web::open_url,
            cli_setup::cli_status,
            cli_setup::cli_install_path,
        ])
}
