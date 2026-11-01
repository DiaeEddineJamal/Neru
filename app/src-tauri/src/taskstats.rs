//! What each task cost: requests sent (on every model, helpers such as the editor and argument repair
//! included), tool rounds, failed edits and tool calls, time spent waiting on rate limits, model switches
//! and tokens. A task runs from a message the user sends to Neru's final reply, approvals included. Free
//! plans run out of requests before anything else, so requests per task is the number every free-model
//! improvement has to bring down. Kept on this computer only.

use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    future::Future,
    sync::{Arc, LazyLock, Mutex},
};

use serde::{Deserialize, Serialize};

use crate::ProviderConfig;

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct TaskStats {
    pub session_id: String,
    /// Unix milliseconds.
    pub started: u64,
    pub finished: u64,
    /// Every request sent to a model, failed ones included.
    pub requests: u32,
    /// Requests by "provider/model".
    pub models: BTreeMap<String, u32>,
    /// Main-model replies in the agent loop.
    pub rounds: u32,
    pub tool_calls: u32,
    /// Edits whose old text did not match the file.
    pub failed_edits: u32,
    /// Tool calls whose arguments could not be read.
    pub parse_failures: u32,
    /// The same request sent again after a dropped connection or a server error.
    pub retries: u32,
    pub switches: u32,
    /// Time spent pacing to rate limits and waiting before retries.
    pub waited_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_tokens: u64,
    /// "done", "stopped" or "error"; empty while the task runs or waits for an approval.
    pub outcome: String,
}

/// Typical cost of recent finished tasks (medians), to compare the current one against.
#[derive(Serialize, Clone, Default, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Typical {
    pub tasks: usize,
    pub requests: u32,
    pub rounds: u32,
    pub failed_edits: u32,
}

#[derive(Serialize, Clone, Default, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskReport {
    /// The task running in this session, or waiting for an approval.
    pub current: Option<TaskStats>,
    /// This session's last finished task.
    pub last: Option<TaskStats>,
    /// Across all sessions.
    pub typical: Option<Typical>,
}

type Task = Arc<Mutex<TaskStats>>;

tokio::task_local! {
    static CURRENT: Task;
}

/// Open tasks by session: a task outlives the call that stops for an approval.
static OPEN: LazyLock<Mutex<HashMap<String, Task>>> = LazyLock::new(|| Mutex::new(HashMap::new()));
static HISTORY: LazyLock<Mutex<VecDeque<TaskStats>>> = LazyLock::new(|| Mutex::new(load()));

/// Finished tasks kept on disk.
const KEEP: usize = 500;
/// Finished tasks the typical cost is taken from.
const RECENT: usize = 20;

fn path() -> Option<std::path::PathBuf> {
    // Tests must not touch the user's data folder.
    if cfg!(test) {
        return None;
    }
    crate::workspace::data_dir().ok().map(|dir| dir.join("task-stats.jsonl"))
}

fn load() -> VecDeque<TaskStats> {
    let Some(text) = path().and_then(|path| std::fs::read_to_string(path).ok()) else { return VecDeque::new() };
    let mut tasks: VecDeque<TaskStats> = text.lines().filter_map(|line| serde_json::from_str(line).ok()).collect();
    while tasks.len() > KEEP {
        tasks.pop_front();
    }
    tasks
}

fn save(tasks: &VecDeque<TaskStats>, added: &TaskStats) {
    let Some(path) = path() else { return };
    // Append, and rewrite the whole file only once it has grown well past what is kept.
    let lines = std::fs::read_to_string(&path).map(|text| text.lines().count()).unwrap_or(0);
    if lines + 1 > KEEP + KEEP / 5 {
        let text: String = tasks.iter().filter_map(|task| serde_json::to_string(task).ok()).map(|line| line + "\n").collect();
        let _ = std::fs::write(&path, text);
    } else if let Ok(line) = serde_json::to_string(added) {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            let _ = writeln!(file, "{line}");
        }
    }
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// The session's open task. `fresh` (a new message from the user) starts a new one; a task left open
/// by an earlier message that was never approved is recorded as stopped first.
pub fn open(session: &str, fresh: bool) -> Task {
    let mut open = lock(&OPEN);
    if !fresh {
        if let Some(task) = open.get(session) {
            return task.clone();
        }
    }
    if let Some(left) = open.remove(session) {
        drop(open);
        finish(left, "stopped");
        open = lock(&OPEN);
    }
    let task = Arc::new(Mutex::new(TaskStats { session_id: session.to_string(), started: now(), ..TaskStats::default() }));
    open.insert(session.to_string(), task.clone());
    task
}

/// Runs `future` with `task` as the task its requests count towards.
pub async fn scope<F: Future>(task: Task, future: F) -> F::Output {
    CURRENT.scope(task, future).await
}

/// `future` counting towards the current task, for work spawned onto its own tokio task. The task is
/// taken now, while still inside it, not when the spawned future first runs.
pub fn carry<F: Future>(future: F) -> impl Future<Output = F::Output> {
    let task = CURRENT.try_with(Task::clone).ok();
    async move {
        match task {
            Some(task) => CURRENT.scope(task, future).await,
            None => future.await,
        }
    }
}

/// Updates the current task; does nothing outside one (probes, settings checks, background chores).
pub fn note(update: impl FnOnce(&mut TaskStats)) {
    let _ = CURRENT.try_with(|task| update(&mut lock(task)));
}

/// One request sent to `config`'s model.
pub fn request(config: &ProviderConfig) {
    note(|task| {
        task.requests += 1;
        *task.models.entry(format!("{}/{}", config.provider_id, config.model)).or_default() += 1;
    });
}

pub fn tokens(input: u64, output: u64, cached: u64) {
    note(|task| {
        task.input_tokens += input;
        task.output_tokens += output;
        task.cached_tokens += cached;
    });
}

pub fn waited(wait: std::time::Duration) {
    note(|task| task.waited_ms += wait.as_millis() as u64);
}

fn finish(task: Task, outcome: &str) -> TaskStats {
    let stats = {
        let mut stats = lock(&task);
        stats.finished = now();
        stats.outcome = outcome.to_string();
        stats.clone()
    };
    let mut history = lock(&HISTORY);
    history.push_back(stats.clone());
    while history.len() > KEEP {
        history.pop_front();
    }
    save(&history, &stats);
    stats
}

/// Ends the session's open task with `outcome` and records it.
pub fn close(session: &str, outcome: &str) -> Option<TaskStats> {
    let task = lock(&OPEN).remove(session)?;
    Some(finish(task, outcome))
}

fn median(mut values: Vec<u32>) -> u32 {
    values.sort_unstable();
    values.get(values.len() / 2).copied().unwrap_or(0)
}

fn typical(history: &VecDeque<TaskStats>) -> Option<Typical> {
    // Tasks that sent nothing (stopped at once, a slash command) say nothing about cost.
    let recent: Vec<&TaskStats> = history.iter().rev().filter(|task| task.requests > 0).take(RECENT).collect();
    if recent.is_empty() {
        return None;
    }
    Some(Typical {
        tasks: recent.len(),
        requests: median(recent.iter().map(|task| task.requests).collect()),
        rounds: median(recent.iter().map(|task| task.rounds).collect()),
        failed_edits: median(recent.iter().map(|task| task.failed_edits).collect()),
    })
}

pub fn report(session: &str) -> TaskReport {
    let current = lock(&OPEN).get(session).map(|task| lock(task).clone());
    let history = lock(&HISTORY);
    TaskReport { current, last: history.iter().rev().find(|task| task.session_id == session).cloned(), typical: typical(&history) }
}

#[tauri::command]
pub fn task_stats(session_id: String) -> TaskReport {
    report(&session_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(model: &str) -> ProviderConfig {
        ProviderConfig { provider_id: "test".into(), api_format: String::new(), base_url: String::new(), model: model.into(), api_key: String::new() }
    }

    #[test]
    fn counts_only_inside_a_task() {
        request(&config("outside"));
        let task = open("test-counts", true);
        tauri::async_runtime::block_on(scope(task, async {
            request(&config("main"));
            request(&config("main"));
            request(&config("helper"));
            tokens(100, 20, 60);
            waited(std::time::Duration::from_millis(1_500));
            note(|task| task.failed_edits += 1);
        }));
        let stats = close("test-counts", "done").unwrap();
        assert_eq!(stats.requests, 3);
        assert_eq!(stats.models.get("test/main"), Some(&2));
        assert_eq!(stats.models.get("test/helper"), Some(&1));
        assert!(!stats.models.contains_key("test/outside"));
        assert_eq!((stats.input_tokens, stats.output_tokens, stats.cached_tokens), (100, 20, 60));
        assert_eq!((stats.waited_ms, stats.failed_edits), (1_500, 1));
        assert_eq!(stats.outcome, "done");
    }

    #[test]
    fn a_task_carries_on_after_an_approval() {
        let first = open("test-approval", true);
        tauri::async_runtime::block_on(scope(first, async { request(&config("m")) }));
        assert_eq!(report("test-approval").current.map(|task| task.requests), Some(1), "open while it waits");
        let resumed = open("test-approval", false);
        tauri::async_runtime::block_on(scope(resumed, async { request(&config("m")) }));
        let stats = close("test-approval", "done").unwrap();
        assert_eq!(stats.requests, 2);
        let report = report("test-approval");
        assert!(report.current.is_none());
        assert_eq!(report.last.map(|task| task.requests), Some(2));
    }

    #[test]
    fn a_new_message_stops_the_task_left_waiting() {
        let first = open("test-replaced", true);
        tauri::async_runtime::block_on(scope(first, async { request(&config("m")) }));
        let second = open("test-replaced", true);
        assert_eq!(lock(&second).requests, 0);
        let last = report("test-replaced").last.unwrap();
        assert_eq!((last.requests, last.outcome.as_str()), (1, "stopped"));
        close("test-replaced", "done");
    }

    #[test]
    fn spawned_work_counts_towards_the_task_that_started_it() {
        let task = open("test-carry", true);
        tauri::async_runtime::block_on(scope(task, async {
            let spawned = tokio::spawn(carry(async { request(&config("sub-agent")) }));
            spawned.await.unwrap();
        }));
        assert_eq!(close("test-carry", "done").unwrap().models.get("test/sub-agent"), Some(&1));
    }

    #[test]
    fn typical_cost_is_the_median_of_tasks_that_sent_something() {
        let task = |requests| TaskStats { requests, rounds: requests, ..TaskStats::default() };
        let history: VecDeque<TaskStats> = [task(0), task(3), task(9), task(4), task(0)].into_iter().collect();
        let typical = typical(&history).unwrap();
        assert_eq!((typical.tasks, typical.requests, typical.rounds), (3, 4, 4));
        assert!(super::typical(&VecDeque::new()).is_none());
    }
}
