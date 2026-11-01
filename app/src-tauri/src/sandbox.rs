//! The operating-system sandbox around the agent's shell commands, under the approval rules in
//! policy.rs.
//!
//! - Linux: bubblewrap (`bwrap`). The whole system is read-only except the project, temporary
//!   folders and package caches; cloud and GPG credentials are hidden; the command gets its own
//!   process namespace and dies with Neru.
//! - macOS: `sandbox-exec` with a profile that refuses writes outside the same folders and reads
//!   of the same credentials.
//! - Windows: a job object. The command and everything it starts form one unit that is ended
//!   together, capped in process count and memory, and kept from other apps' windows, the
//!   clipboard's contents, display and system settings, and shutting Windows down. The command
//!   also runs at Low integrity: the project, a temporary folder and `--add-dir` folders get a Low
//!   mandatory label, and Windows refuses its writes to everything else the user owns (no write
//!   up). If the token or the labels cannot be set, or the shell cannot start at Low integrity
//!   (Windows PowerShell 5.1 fails fast there on some systems; checked once), the command runs in
//!   the job object alone. `NERU_SANDBOX_WRITES=off` keeps only the job object.
//!
//! The sandbox is on by default. `"sandbox": {"enabled": false}` in `~/.claude/settings.json` (or
//! a trusted project's settings) or `NERU_SANDBOX=off` turns it off. A command the model asks to
//! run outside it always needs the user's approval, except in Bypass mode.

#[allow(unused_imports)] // .hidden() is called here only on some systems
use crate::Hidden;

use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;

/// What confines the agent's commands on this computer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)] // each platform builds only its own kind
pub enum Kind {
    JobObject,
    Bubblewrap,
    Seatbelt,
    /// Nothing is available (Linux without bubblewrap) or the sandbox is turned off.
    None,
}

/// Whether the user turned the sandbox off, by environment or settings.
pub fn turned_off(root: &Path) -> bool {
    if std::env::var("NERU_SANDBOX").is_ok_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "off" | "0" | "false" | "no")) {
        return true;
    }
    let mut trusted = None;
    crate::policy::settings_files(root).into_iter().any(|(label, path)| {
        let Some(settings) = crate::policy::read_settings(&path) else { return false };
        // An untrusted project may not lower the guard on its own commands.
        settings["sandbox"]["enabled"] == serde_json::json!(false) && (label.starts_with('~') || *trusted.get_or_insert_with(|| crate::trust::is_trusted(root)))
    })
}

/// The sandbox the agent's commands in `root` run in.
pub fn kind(root: &Path) -> Kind {
    if turned_off(root) {
        return Kind::None;
    }
    available()
}

/// The sandbox this computer offers, whether or not it is turned on.
pub fn available() -> Kind {
    #[cfg(windows)]
    let kind = Kind::JobObject;
    #[cfg(target_os = "macos")]
    let kind = if Path::new(SANDBOX_EXEC).is_file() { Kind::Seatbelt } else { Kind::None };
    #[cfg(all(unix, not(target_os = "macos")))]
    let kind = if bubblewrap().is_some() { Kind::Bubblewrap } else { Kind::None };
    kind
}

/// One line for doctor, /status and the shell tool's description.
pub fn describe(root: &Path) -> String {
    match kind(root) {
        Kind::Bubblewrap => "bubblewrap: writes only inside the project, temporary folders and package caches".into(),
        Kind::Seatbelt => "sandbox-exec: writes only inside the project, temporary folders and package caches".into(),
        Kind::JobObject if write_limit() && job::shell_runs_low() => "a Windows job object at Low integrity: writes only inside the project and a temporary folder; the process tree is capped and ended together".into(),
        Kind::JobObject => "a Windows job object: the whole process tree is contained, capped and ended together".into(),
        Kind::None if turned_off(root) => "off (turned off in settings or NERU_SANDBOX)".into(),
        Kind::None if cfg!(target_os = "linux") => "not available: install bubblewrap (sudo apt install bubblewrap) to confine commands".into(),
        Kind::None => "not available on this system".into(),
    }
}

/// A hint added to a failed sandboxed command's output when the sandbox likely caused the failure.
pub fn failure_hint(output: &str) -> Option<&'static str> {
    let lower = output.to_lowercase();
    ["read-only file system", "operation not permitted", "sandbox", "deny file-write"].iter().any(|marker| lower.contains(marker)).then_some(
        "[This command ran in Neru's sandbox, which allows writes only inside the project, temporary folders and package caches. If it must write elsewhere, run it again with outside_sandbox true; the user is asked first.]",
    )
}

/// Whether Windows commands also get the Low-integrity write limit.
pub fn write_limit() -> bool {
    !std::env::var("NERU_SANDBOX_WRITES").is_ok_and(|value| matches!(value.trim().to_ascii_lowercase().as_str(), "off" | "0" | "false" | "no"))
}

/// An agent command, ready to configure (folder, pipes) and start with [`Prepared::start`].
pub struct Prepared {
    pub process: tokio::process::Command,
    #[cfg_attr(not(windows), allow(dead_code))]
    job: bool,
    /// Folders a write-limited command may write to (Windows); empty without the limit.
    #[cfg_attr(not(windows), allow(dead_code))]
    writable: Vec<std::path::PathBuf>,
}

/// Holds what keeps a started command confined. On Windows, dropping it ends every process the
/// command left behind.
pub struct Guard {
    #[cfg(windows)]
    _job: Option<job::Job>,
}

/// The shell process for `command` in `root`, inside the sandbox unless `sandboxed` is false or
/// none is available.
pub fn prepare(command: &str, root: &Path, sandboxed: bool) -> Prepared {
    let kind = if sandboxed { kind(root) } else { Kind::None };
    #[cfg(windows)]
    {
        let mut process = crate::shells::command(command);
        // Suspended until it is in the job, so not even its first instruction runs outside it.
        process.creation_flags(CREATE_NO_WINDOW | if kind == Kind::JobObject { CREATE_SUSPENDED } else { 0 });
        let mut writable = Vec::new();
        if kind == Kind::JobObject && write_limit() && job::shell_runs_low() {
            // Temporary files go to a folder the Low-integrity command may write.
            let temp = std::env::temp_dir().join("neru-low");
            let _ = std::fs::create_dir_all(&temp);
            process.env("TEMP", &temp).env("TMP", &temp);
            writable.push(root.to_path_buf());
            writable.push(temp);
            writable.extend(crate::run_options::get().add_dirs);
        }
        Prepared { process, job: kind == Kind::JobObject, writable }
    }
    #[cfg(target_os = "macos")]
    {
        let process = if kind == Kind::Seatbelt {
            let mut process = tokio::process::Command::new(SANDBOX_EXEC);
            let profile = seatbelt_profile(root);
            process.args(["-p", profile.as_str(), "sh", "-lc", command]);
            process
        } else {
            crate::shells::command(command)
        };
        Prepared { process, job: false, writable: Vec::new() }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let process = match (kind, bubblewrap()) {
            (Kind::Bubblewrap, Some(bwrap)) => {
                let mut process = tokio::process::Command::new(bwrap);
                process.args(bubblewrap_args(root, command));
                process
            }
            _ => crate::shells::command(command),
        };
        Prepared { process, job: false, writable: Vec::new() }
    }
}

impl Prepared {
    /// Starts the command; on Windows it joins its job before it runs.
    pub fn start(mut self) -> Result<(tokio::process::Child, Guard), String> {
        let child = self.process.spawn().map_err(|e| format!("Could not start the command: {e}"))?;
        #[cfg(windows)]
        {
            let job = if self.job {
                let job = job::Job::new();
                let joined = match (&job, child.raw_handle()) {
                    (Some(job), Some(handle)) => job.assign(handle),
                    _ => false,
                };
                // Still suspended: drop to Low integrity before its first instruction, once every
                // folder it may write carries the Low label.
                if joined && !self.writable.is_empty() {
                    let limited = self.writable.iter().all(|folder| job::allow_low_writes(folder)) && child.raw_handle().is_some_and(job::lower_integrity);
                    if !limited {
                        log::warn!("could not limit a command's writes; it runs in the job object alone");
                    }
                }
                // Resume whatever happens: a process left suspended would hang the session.
                if let Some(pid) = child.id() {
                    job::resume(pid);
                }
                if !joined {
                    log::warn!("could not put a command in its job object; it runs without one");
                }
                job.filter(|_| joined)
            } else {
                None
            };
            Ok((child, Guard { _job: job }))
        }
        #[cfg(not(windows))]
        Ok((child, Guard {}))
    }
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const CREATE_SUSPENDED: u32 = 0x0000_0004;

// ---------------------------------------------------------------------------------------------
// Folders a confined command may write to, and credentials it may not read (Linux and macOS)

/// Package-manager and build caches under the home folder; installs and builds fail without them.
#[cfg(unix)]
const HOME_CACHES: &[&str] = &[
    ".cache", ".npm", ".yarn", ".pnpm-store", ".local/share/pnpm", ".bun", ".deno", ".cargo", ".rustup", ".gradle", ".m2", "go",
    ".nuget", ".pub-cache", ".gem", ".local/share/virtualenvs", "Library/Caches",
];

/// Credentials of cloud and signing tools; a command in the project has no business reading them.
#[cfg(unix)]
const HOME_SECRETS: &[&str] = &[".aws", ".azure", ".config/gcloud", ".kube", ".gnupg", ".password-store", ".docker"];

#[cfg(unix)]
fn writable(root: &Path) -> Vec<PathBuf> {
    let mut folders = vec![root.to_path_buf(), PathBuf::from("/tmp")];
    if let Some(temp) = std::env::var_os("TMPDIR").map(PathBuf::from) {
        folders.push(temp);
    }
    if cfg!(target_os = "macos") {
        folders.push(PathBuf::from("/private/tmp"));
        folders.push(PathBuf::from("/private/var/folders"));
    }
    if let Some(home) = dirs::home_dir() {
        folders.extend(HOME_CACHES.iter().map(|dir| home.join(dir)));
    }
    // Folders the user added with --add-dir are part of the work.
    folders.extend(crate::run_options::get().add_dirs);
    folders
}

#[cfg(unix)]
fn secrets() -> Vec<PathBuf> {
    dirs::home_dir().map(|home| HOME_SECRETS.iter().map(|dir| home.join(dir)).filter(|dir| dir.is_dir()).collect()).unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// Linux: bubblewrap

/// `bwrap`, when it is installed and allowed to create namespaces here. Checked once.
#[cfg(all(unix, not(target_os = "macos")))]
fn bubblewrap() -> Option<&'static Path> {
    static FOUND: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    FOUND
        .get_or_init(|| {
            let path = std::env::var_os("PATH").and_then(|paths| std::env::split_paths(&paths).map(|dir| dir.join("bwrap")).find(|path| path.is_file()))?;
            // Some systems ship bwrap but forbid unprivileged user namespaces; then it cannot help.
            let works = std::process::Command::new(&path).hidden()
                .args(["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc", "--unshare-pid", "--die-with-parent", "true"])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|status| status.success());
            works.then_some(path)
        })
        .as_deref()
}

#[cfg(all(unix, not(target_os = "macos")))]
fn bubblewrap_args(root: &Path, command: &str) -> Vec<std::ffi::OsString> {
    let mut args: Vec<std::ffi::OsString> = ["--die-with-parent", "--new-session", "--unshare-pid", "--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc", "--tmpfs", "/dev/shm"]
        .iter()
        .map(Into::into)
        .collect();
    for dir in writable(root) {
        args.extend(["--bind-try".into(), dir.clone().into_os_string(), dir.into_os_string()]);
    }
    for dir in secrets() {
        args.extend(["--tmpfs".into(), dir.into_os_string()]);
    }
    args.extend(["--chdir".into(), root.as_os_str().to_owned(), "--".into(), "sh".into(), "-lc".into(), command.into()]);
    args
}

// ---------------------------------------------------------------------------------------------
// macOS: sandbox-exec (Seatbelt)

#[cfg(target_os = "macos")]
const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// A path as Seatbelt sees it: symlinks resolved (/tmp is /private/tmp), quoted for the profile.
#[cfg(unix)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn seatbelt_path(path: &Path) -> String {
    let real = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    format!("\"{}\"", real.display().to_string().replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(unix)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn seatbelt_profile(root: &Path) -> String {
    let writes: Vec<String> = writable(root).iter().map(|dir| format!("(subpath {})", seatbelt_path(dir))).collect();
    let hidden: Vec<String> = secrets().iter().map(|dir| format!("(subpath {})", seatbelt_path(dir))).collect();
    let mut profile = format!(
        "(version 1)\n(allow default)\n(deny file-write*)\n(allow file-write* {} (literal \"/dev/null\") (literal \"/dev/zero\") (literal \"/dev/dtracehelper\") (regex #\"^/dev/tty\") (regex #\"^/dev/fd/\") (regex #\"^/dev/ptmx\") (regex #\"^/dev/ttys\"))\n",
        writes.join(" ")
    );
    if !hidden.is_empty() {
        profile.push_str(&format!("(deny file-read* {})\n", hidden.join(" ")));
    }
    profile
}

// ---------------------------------------------------------------------------------------------
// Windows: a job object

#[cfg(windows)]
mod job {
    use std::ffi::c_void;

    type Handle = *mut c_void;

    const KILL_ON_JOB_CLOSE: u32 = 0x2000;
    const ACTIVE_PROCESS: u32 = 0x8;
    const JOB_MEMORY: u32 = 0x200;
    const DIE_ON_UNHANDLED_EXCEPTION: u32 = 0x400;
    const EXTENDED_LIMIT_INFORMATION: i32 = 9;
    const BASIC_UI_RESTRICTIONS: i32 = 4;
    // Other apps' windows, reading the clipboard, display and system settings, desktops, global
    // atoms and shutting down. Writing to the clipboard stays allowed (`clip`).
    const UI_LIMITS: u32 = 0x1 | 0x2 | 0x8 | 0x10 | 0x20 | 0x40 | 0x80;
    const THREAD_SUSPEND_RESUME: u32 = 0x2;
    const SNAPTHREAD: u32 = 0x4;
    const INVALID_HANDLE: Handle = -1isize as Handle;
    /// Plenty for parallel builds (cargo, webpack, test runners); stops a fork bomb.
    const MAX_PROCESSES: u32 = 512;

    #[repr(C)]
    #[derive(Default)]
    struct BasicLimits {
        per_process_user_time: i64,
        per_job_user_time: i64,
        flags: u32,
        min_working_set: usize,
        max_working_set: usize,
        active_processes: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct ExtendedLimits {
        basic: BasicLimits,
        io: [u64; 6],
        process_memory: usize,
        job_memory: usize,
        peak_process_memory: usize,
        peak_job_memory: usize,
    }

    #[repr(C)]
    #[derive(Default)]
    struct ThreadEntry {
        size: u32,
        usage: u32,
        thread_id: u32,
        owner_process_id: u32,
        base_priority: i32,
        delta_priority: i32,
        flags: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct MemoryStatus {
        length: u32,
        load: u32,
        total_physical: u64,
        available_physical: u64,
        total_page_file: u64,
        available_page_file: u64,
        total_virtual: u64,
        available_virtual: u64,
        available_extended_virtual: u64,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> Handle;
        fn SetInformationJobObject(job: Handle, class: i32, info: *const c_void, length: u32) -> i32;
        fn AssignProcessToJobObject(job: Handle, process: Handle) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
        fn CreateToolhelp32Snapshot(flags: u32, process_id: u32) -> Handle;
        fn Thread32First(snapshot: Handle, entry: *mut ThreadEntry) -> i32;
        fn Thread32Next(snapshot: Handle, entry: *mut ThreadEntry) -> i32;
        fn OpenThread(access: u32, inherit: i32, thread_id: u32) -> Handle;
        fn ResumeThread(thread: Handle) -> u32;
        fn GlobalMemoryStatusEx(status: *mut MemoryStatus) -> i32;
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
        fn GetVolumePathNameW(path: *const u16, volume: *mut u16, length: u32) -> i32;
        fn GetVolumeInformationW(root: *const u16, name: *mut u16, name_len: u32, serial: *mut u32, max_component: *mut u32, flags: *mut u32, fs_name: *mut u16, fs_name_len: u32) -> i32;
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn OpenProcessToken(process: Handle, access: u32, token: *mut Handle) -> i32;
        fn SetTokenInformation(token: Handle, class: i32, info: *const c_void, length: u32) -> i32;
        fn ConvertStringSidToSidW(sid: *const u16, out: *mut *mut c_void) -> i32;
        fn GetLengthSid(sid: *mut c_void) -> u32;
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl: *const u16, revision: u32, descriptor: *mut *mut c_void, size: *mut u32) -> i32;
        fn GetSecurityDescriptorSacl(descriptor: *mut c_void, present: *mut i32, sacl: *mut *mut c_void, defaulted: *mut i32) -> i32;
        fn SetNamedSecurityInfoW(name: *const u16, kind: u32, info: u32, owner: *mut c_void, group: *mut c_void, dacl: *mut c_void, sacl: *mut c_void) -> u32;
    }

    const TOKEN_QUERY: u32 = 0x8;
    const TOKEN_ADJUST_DEFAULT: u32 = 0x80;
    const TOKEN_INTEGRITY_LEVEL: i32 = 25;
    const SE_GROUP_INTEGRITY: u32 = 0x20;
    const SE_FILE_OBJECT: u32 = 1;
    const LABEL_SECURITY_INFORMATION: u32 = 0x10;
    const FILE_PERSISTENT_ACLS: u32 = 0x8;

    fn wide(text: &std::ffi::OsStr) -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        text.encode_wide().chain(std::iter::once(0)).collect()
    }

    #[repr(C)]
    struct MandatoryLabel {
        sid: *mut c_void,
        attributes: u32,
    }

    /// Lowers a (suspended) process's token to Low integrity, so it cannot write to anything with
    /// the default Medium label. Lowering needs no privilege; raising back is impossible.
    pub fn lower_integrity(process: std::os::windows::io::RawHandle) -> bool {
        // SAFETY: handles and the SID are checked and released; the label struct is sized.
        unsafe {
            let mut token: Handle = std::ptr::null_mut();
            if OpenProcessToken(process as Handle, TOKEN_ADJUST_DEFAULT | TOKEN_QUERY, &mut token) == 0 {
                return false;
            }
            let mut sid: *mut c_void = std::ptr::null_mut();
            let low = wide(std::ffi::OsStr::new("S-1-16-4096"));
            let done = if ConvertStringSidToSidW(low.as_ptr(), &mut sid) != 0 {
                let label = MandatoryLabel { sid, attributes: SE_GROUP_INTEGRITY };
                let ok = SetTokenInformation(token, TOKEN_INTEGRITY_LEVEL, (&raw const label).cast(), (size_of::<MandatoryLabel>() as u32) + GetLengthSid(sid)) != 0;
                LocalFree(sid);
                ok
            } else {
                false
            };
            CloseHandle(token);
            done
        }
    }

    /// Whether the folder's volume keeps security information (NTFS, ReFS). FAT and exFAT keep
    /// none, so any process may write there and no label is needed (or possible).
    pub fn keeps_acls(folder: &std::path::Path) -> bool {
        let path = wide(folder.as_os_str());
        let mut volume = [0u16; 300];
        let mut flags = 0u32;
        // SAFETY: buffers are sized as passed.
        unsafe {
            if GetVolumePathNameW(path.as_ptr(), volume.as_mut_ptr(), volume.len() as u32) == 0 {
                return true;
            }
            if GetVolumeInformationW(volume.as_ptr(), std::ptr::null_mut(), 0, std::ptr::null_mut(), std::ptr::null_mut(), &mut flags, std::ptr::null_mut(), 0) == 0 {
                return true;
            }
        }
        flags & FILE_PERSISTENT_ACLS != 0
    }

    /// Whether the agent's shell (Windows PowerShell) starts at Low integrity here. Checked once.
    pub fn shell_runs_low() -> bool {
        static WORKS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *WORKS.get_or_init(|| {
            use std::os::windows::{io::AsRawHandle, process::CommandExt};
            let Ok(mut child) = std::process::Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-Command", "exit 7"])
                .creation_flags(super::CREATE_NO_WINDOW | super::CREATE_SUSPENDED)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            else {
                return false;
            };
            let lowered = lower_integrity(child.as_raw_handle());
            resume(child.id());
            let started = std::time::Instant::now();
            let code = loop {
                match child.try_wait() {
                    Ok(Some(status)) => break status.code(),
                    Ok(None) if started.elapsed() < std::time::Duration::from_secs(20) => std::thread::sleep(std::time::Duration::from_millis(50)),
                    _ => {
                        let _ = child.kill();
                        break None;
                    }
                }
            };
            let works = lowered && code == Some(7);
            if !works {
                log::warn!("PowerShell does not start at Low integrity here; agent commands run in the job object without the write limit");
            }
            works
        })
    }

    /// Folders already labelled in this run.
    static LABELLED: std::sync::LazyLock<std::sync::Mutex<std::collections::HashSet<std::path::PathBuf>>> = std::sync::LazyLock::new(Default::default);

    /// Gives a folder (and, by inheritance, everything in it) the Low mandatory label, so Low
    /// integrity commands may write there. Medium processes are unaffected.
    pub fn allow_low_writes(folder: &std::path::Path) -> bool {
        if LABELLED.lock().is_ok_and(|done| done.contains(folder)) {
            return true;
        }
        if !folder.is_dir() {
            return false;
        }
        let labelled = !keeps_acls(folder) || {
            // SAFETY: the descriptor is freed; the SACL points into it while it lives.
            unsafe {
                let sddl = wide(std::ffi::OsStr::new("S:(ML;OICI;NW;;;LW)"));
                let mut descriptor: *mut c_void = std::ptr::null_mut();
                if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut descriptor, std::ptr::null_mut()) == 0 {
                    return false;
                }
                let (mut present, mut defaulted, mut sacl) = (0, 0, std::ptr::null_mut());
                let ok = GetSecurityDescriptorSacl(descriptor, &mut present, &mut sacl, &mut defaulted) != 0
                    && SetNamedSecurityInfoW(wide(folder.as_os_str()).as_ptr(), SE_FILE_OBJECT, LABEL_SECURITY_INFORMATION, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), sacl) == 0;
                LocalFree(descriptor);
                ok
            }
        };
        if labelled {
            if let Ok(mut done) = LABELLED.lock() {
                done.insert(folder.to_path_buf());
            }
        }
        labelled
    }

    /// A job whose processes all end when it is dropped.
    pub struct Job(Handle);

    // SAFETY: a job handle is a kernel object handle, usable from any thread.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        pub fn new() -> Option<Job> {
            // SAFETY: plain Win32 calls with valid, correctly sized arguments.
            unsafe {
                let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if handle.is_null() {
                    return None;
                }
                let job = Job(handle);
                let mut limits = ExtendedLimits::default();
                limits.basic.flags = KILL_ON_JOB_CLOSE | ACTIVE_PROCESS | JOB_MEMORY | DIE_ON_UNHANDLED_EXCEPTION;
                limits.basic.active_processes = MAX_PROCESSES;
                limits.job_memory = memory_cap();
                if SetInformationJobObject(handle, EXTENDED_LIMIT_INFORMATION, (&raw const limits).cast(), size_of::<ExtendedLimits>() as u32) == 0 {
                    return None;
                }
                let ui: u32 = UI_LIMITS;
                // The UI limits are extra protection; the job still contains the tree without them.
                SetInformationJobObject(handle, BASIC_UI_RESTRICTIONS, (&raw const ui).cast(), size_of::<u32>() as u32);
                Some(job)
            }
        }

        pub fn assign(&self, process: std::os::windows::io::RawHandle) -> bool {
            // SAFETY: both handles are valid for the duration of the call.
            unsafe { AssignProcessToJobObject(self.0, process as Handle) != 0 }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: the handle came from CreateJobObjectW and is closed once.
            unsafe { CloseHandle(self.0) };
        }
    }

    /// Three quarters of the computer's memory, and at least 4 GB, for everything the command starts.
    fn memory_cap() -> usize {
        let mut status = MemoryStatus { length: size_of::<MemoryStatus>() as u32, ..Default::default() };
        // SAFETY: `status` is a correctly sized MEMORYSTATUSEX with its length set.
        let total = if unsafe { GlobalMemoryStatusEx(&mut status) } != 0 { status.total_physical } else { 0 };
        usize::try_from((total / 4 * 3).max(4 << 30)).unwrap_or(usize::MAX)
    }

    /// Lets every thread of a process started suspended run.
    pub fn resume(process_id: u32) {
        // SAFETY: the snapshot and thread handles are checked and closed; the entry is sized.
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(SNAPTHREAD, 0);
            if snapshot == INVALID_HANDLE || snapshot.is_null() {
                return;
            }
            let mut entry = ThreadEntry { size: size_of::<ThreadEntry>() as u32, ..Default::default() };
            let mut more = Thread32First(snapshot, &mut entry) != 0;
            while more {
                if entry.owner_process_id == process_id {
                    let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.thread_id);
                    if !thread.is_null() {
                        ResumeThread(thread);
                        CloseHandle(thread);
                    }
                }
                more = Thread32Next(snapshot, &mut entry) != 0;
            }
            CloseHandle(snapshot);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_failures_get_a_hint() {
        assert!(failure_hint("touch: cannot touch '/etc/x': Read-only file system").is_some());
        assert!(failure_hint("error: tests failed").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn the_seatbelt_profile_limits_writes_to_the_project() {
        let root = std::env::temp_dir();
        let profile = seatbelt_profile(&root);
        assert!(profile.contains("(deny file-write*)"));
        assert!(profile.contains(&seatbelt_path(&root)));
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn bubblewrap_binds_the_project_writable_and_runs_the_command() {
        let args: Vec<String> = bubblewrap_args(Path::new("/work/app"), "npm test").iter().map(|arg| arg.to_string_lossy().into_owned()).collect();
        let joined = args.join(" ");
        assert!(joined.starts_with("--die-with-parent --new-session --unshare-pid --ro-bind / /"));
        assert!(joined.contains("--bind-try /work/app /work/app"));
        assert!(joined.ends_with("--chdir /work/app -- sh -lc npm test"));
    }

    /// Runs real commands in a job object: output comes back, the process count is capped, and
    /// dropping the guard ends what the command left running.
    #[cfg(windows)]
    #[tokio::test]
    async fn windows_commands_run_inside_a_job() {
        let root = std::env::temp_dir();
        let mut prepared = prepare("Write-Output 'inside'", &root, true);
        prepared.process.current_dir(&root).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        let (child, _guard) = prepared.start().expect("starts");
        let output = tokio::time::timeout(std::time::Duration::from_secs(60), child.wait_with_output()).await.expect("finishes").expect("output");
        assert!(String::from_utf8_lossy(&output.stdout).contains("inside"), "{output:?}");

        // A background process the command starts is ended with the guard.
        let marker = format!("neru-sandbox-{}", std::process::id());
        let mut prepared = prepare(&format!("Start-Process -WindowStyle Hidden powershell -ArgumentList '-NoProfile','-Command','Start-Sleep 120 # {marker}'; Start-Sleep 2"), &root, true);
        prepared.process.current_dir(&root);
        let (mut child, guard) = prepared.start().expect("starts");
        let _ = tokio::time::timeout(std::time::Duration::from_secs(60), child.wait()).await;
        let count = || {
            let out = std::process::Command::new("powershell").hidden()
                .args(["-NoProfile", "-Command", &format!("@(Get-CimInstance Win32_Process | Where-Object {{ $_.CommandLine -like '*{marker}*' -and $_.Name -eq 'powershell.exe' -and $_.CommandLine -notlike '*Get-CimInstance*' }}).Count")])
                .output()
                .expect("powershell");
            String::from_utf8_lossy(&out.stdout).trim().parse::<u32>().unwrap_or(0)
        };
        assert!(count() >= 1, "the detached process should still run while the guard is held");
        drop(guard);
        std::thread::sleep(std::time::Duration::from_millis(800));
        assert_eq!(count(), 0, "dropping the guard ends the detached process");
    }

    /// A folder for the write-limit test on a volume that keeps security information: the temp
    /// folder, or the user's own temp folder when that one is FAT/exFAT (nothing to refuse there).
    /// The test removes it again.
    #[cfg(windows)]
    fn acl_scratch(name: &str) -> std::path::PathBuf {
        let candidates = [std::env::temp_dir(), dirs::data_local_dir().unwrap_or_default().join("Temp")];
        let base = candidates.iter().find(|dir| dir.is_dir() && job::keeps_acls(dir)).cloned().unwrap_or_else(std::env::temp_dir);
        let dir = base.join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Writes inside the project work; writes to another folder the user owns are refused. Driven
    /// with cmd.exe, which starts at Low integrity everywhere, through the same start path.
    #[cfg(windows)]
    #[tokio::test]
    async fn windows_commands_cannot_write_outside_the_project() {
        let project = acl_scratch("neru-sbx-project");
        let outside = acl_scratch("neru-sbx-outside");
        let inside_file = project.join("inside.txt");
        let outside_file = outside.join("outside.txt");
        let mut process = tokio::process::Command::new("cmd.exe");
        process.raw_arg(format!("/d /c \"echo ok> \"{}\" & echo leaked> \"{}\" & whoami /groups\"", inside_file.display(), outside_file.display()));
        process.creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED).current_dir(&project).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        let prepared = Prepared { process, job: true, writable: vec![project.clone()] };
        let (child, _guard) = prepared.start().expect("starts");
        let output = tokio::time::timeout(std::time::Duration::from_secs(60), child.wait_with_output()).await.expect("finishes").expect("output");
        let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        let result = (inside_file.is_file(), outside_file.exists(), text.contains("Low Mandatory Level"));
        let _ = std::fs::remove_dir_all(&project);
        let _ = std::fs::remove_dir_all(&outside);
        assert!(result.2, "the command runs at Low integrity: {text}");
        assert!(result.0, "writes inside the project work: {text}");
        assert!(!result.1, "writes outside the project are refused: {text}");
    }

    /// The agent's own commands keep working: with the write limit when PowerShell starts at Low
    /// integrity, in the job object alone when it cannot.
    #[cfg(windows)]
    #[tokio::test]
    async fn windows_shell_falls_back_when_it_cannot_run_low() {
        let root = std::env::temp_dir();
        let prepared = prepare("Write-Output 'fine'", &root, true);
        assert_eq!(prepared.writable.is_empty(), !(write_limit() && job::shell_runs_low()));
    }
}
