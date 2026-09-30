mod agent;
mod cli;
mod cli_ui;
mod commands;
mod documents;
mod extras;
mod fallback;
mod git;
mod hooks;
mod limits;
mod mcp;
mod oauth;
mod policy;
mod preview;
mod providers;
mod sessions;
mod settings;
mod skills;
mod stream;
mod tasks;
mod terminal;
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
    },
    /// A tool on a connected MCP server.
    Mcp {
        server: String,
        tool: String,
        arguments: Value,
        tool_call_id: Option<String>,
    },
}

impl PendingAction {
    pub fn tool_call_id(&self) -> Option<&str> {
        match self {
            Self::Edit { tool_call_id, .. }
            | Self::Task { tool_call_id, .. }
            | Self::Command { tool_call_id, .. }
            | Self::Mcp { tool_call_id, .. } => tool_call_id.as_deref(),
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
        tauri::async_runtime::spawn(mcp::start_enabled(app));
    })
    .run(context())
    .expect("error while building Neru");
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

/// Everything but the window: plugins, state and commands. `ready` runs once the app is set up.
pub(crate) fn builder(ready: impl FnOnce(tauri::AppHandle) + Send + 'static) -> tauri::Builder<tauri::Wry> {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(AppState::default())
        .setup(move |app| {
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
            workspace::propose_file,
            workspace::apply_pending,
            workspace::reject_pending,
            workspace::restore_checkpoint,
            sessions::list_sessions,
            sessions::list_all_sessions,
            sessions::current_session,
            sessions::create_session,
            sessions::create_chat_session,
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
            preview::preview_stop,
            workspace::save_file,
            workspace::open_in_editor,
            commands::list_commands,
            terminal::terminal_start,
            terminal::terminal_write,
            terminal::terminal_resize,
            terminal::terminal_stop,
            agent::configure_provider,
            agent::provider_status,
            agent::new_chat,
            agent::list_models,
            agent::ai_chat,
            documents::inspect_documents,
            agent::stop_chat,
            agent::run_pending_task,
            agent::steer_session,
            agent::memory_files,
            agent::permission_rules,
            agent::revoke_permission,
            extras::doctor,
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
            voice::voice_status,
            voice::configure_voice,
            settings::forget_keys,
            voice::transcribe_audio,
            web::open_url,
        ])
}
