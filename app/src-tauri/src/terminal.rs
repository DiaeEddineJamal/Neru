use std::{
    collections::HashMap,
    io::{Read, Write},
    sync::{Arc, Mutex},
    thread,
};

use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::{AppState, workspace::project_root};

pub struct TerminalSession {
    pub master: Box<dyn portable_pty::MasterPty + Send>,
    pub writer: Box<dyn Write + Send>,
    pub child: Box<dyn portable_pty::Child + Send>,
}

pub type TerminalMap = Arc<Mutex<HashMap<String, TerminalSession>>>;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TerminalEvent {
    id: String,
    data: String,
}

#[tauri::command]
pub fn terminal_start(app: AppHandle, state: State<'_, AppState>) -> Result<String, String> {
    let root = project_root(&state)?;
    let pty = NativePtySystem::default();
    let pair = pty
        .openpty(PtySize {
            rows: 30,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())?;
    let shell = if cfg!(windows) {
        "powershell.exe".to_string()
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into())
    };
    let mut command = CommandBuilder::new(&shell);
    // Neru draws its own welcome; skip the PowerShell copyright banner.
    if cfg!(windows) {
        command.arg("-NoLogo");
    }
    command.cwd(root);
    // `neru` in this terminal starts the CLI that ships next to the app.
    if let Some(bin) = cli_shim_dir() {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![bin];
        paths.extend(std::env::split_paths(&path));
        if let Ok(joined) = std::env::join_paths(paths) {
            command.env("PATH", joined);
        }
    }
    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|e| e.to_string())?;
    drop(pair.slave);
    let mut reader = pair.master.try_clone_reader().map_err(|e| e.to_string())?;
    let writer = pair.master.take_writer().map_err(|e| e.to_string())?;
    let id = uuid::Uuid::new_v4().to_string();
    state.terminals.lock().map_err(|e| e.to_string())?.insert(
        id.clone(),
        TerminalSession {
            master: pair.master,
            writer,
            child,
        },
    );
    let event_id = id.clone();
    thread::spawn(move || {
        let mut buffer = [0_u8; 8192];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let _ = app.emit(
                        "terminal-output",
                        TerminalEvent {
                            id: event_id.clone(),
                            data: String::from_utf8_lossy(&buffer[..n]).to_string(),
                        },
                    );
                }
            }
        }
        let _ = app.emit("terminal-exit", event_id);
    });
    Ok(id)
}

#[tauri::command]
pub fn terminal_write(id: String, data: String, state: State<'_, AppState>) -> Result<(), String> {
    if data.len() > 8192 {
        return Err("Terminal input too large".into());
    }
    let mut sessions = state.terminals.lock().map_err(|e| e.to_string())?;
    let session = sessions.get_mut(&id).ok_or("Terminal session ended")?;
    session
        .writer
        .write_all(data.as_bytes())
        .map_err(|e| e.to_string())?;
    session.writer.flush().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn terminal_resize(
    id: String,
    cols: u16,
    rows: u16,
    state: State<'_, AppState>,
) -> Result<(), String> {
    if cols == 0 || rows == 0 {
        return Err("Invalid terminal size".into());
    }
    let sessions = state.terminals.lock().map_err(|e| e.to_string())?;
    let session = sessions.get(&id).ok_or("Terminal session ended")?;
    session
        .master
        .resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn terminal_stop(id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut sessions = state.terminals.lock().map_err(|e| e.to_string())?;
    if let Some(mut session) = sessions.remove(&id) {
        session.child.kill().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// A folder holding a `neru` launcher for the CLI binary beside this executable, created on
/// demand in Neru's data folder. None when the CLI is not installed next to the app.
pub fn cli_shim_dir() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let name = if cfg!(windows) { "neru-cli.exe" } else { "neru-cli" };
    let cli = exe.parent()?.join(name);
    if !cli.is_file() {
        return None;
    }
    let dir = crate::workspace::data_dir().ok()?.join("bin");
    std::fs::create_dir_all(&dir).ok()?;
    if cfg!(windows) {
        let body = format!("@echo off\r\n\"{}\" %*\r\n", cli.display());
        let shim = dir.join("neru.cmd");
        if std::fs::read_to_string(&shim).ok().as_deref() != Some(body.as_str()) {
            std::fs::write(&shim, body).ok()?;
        }
    } else {
        let shim = dir.join("neru");
        let body = format!("#!/bin/sh\nexec \"{}\" \"$@\"\n", cli.display());
        if std::fs::read_to_string(&shim).ok().as_deref() != Some(body.as_str()) {
            std::fs::write(&shim, body).ok()?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755));
            }
        }
    }
    Some(dir)
}
