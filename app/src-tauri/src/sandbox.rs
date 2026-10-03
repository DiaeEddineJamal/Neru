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
//!   clipboard's contents, display and system settings, and shutting Windows down. Windows has no
//!   per-folder write limit for ordinary processes, so there the approval rules guard files.
//!
//! The sandbox is on by default. `"sandbox": {"enabled": false}` in `~/.claude/settings.json` (or
//! a trusted project's settings) or `NERU_SANDBOX=off` turns it off. A command the model asks to
//! run outside it always needs the user's approval, except in Bypass mode.

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

/// An agent command, ready to configure (folder, pipes) and start with [`Prepared::start`].
pub struct Prepared {
    pub process: tokio::process::Command,
    #[cfg_attr(not(windows), allow(dead_code))]
    job: bool,
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
        Prepared { process, job: kind == Kind::JobObject }
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
        Prepared { process, job: false }
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
        Prepared { process, job: false }
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
            let works = std::process::Command::new(&path)
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
            let out = std::process::Command::new("powershell")
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
}
