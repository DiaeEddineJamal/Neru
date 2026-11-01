//! Semantic code search. The project is cut into chunks along its top-level definitions; the
//! window embeds them with a local model in a web worker (lib/semantic) and hands the vectors
//! back; they live in SQLite under the data folder, keyed by path, modification time and chunk.
//! A query is embedded the same way (an event out, a command back) and answered by cosine
//! similarity. Nothing leaves the machine except the one-time model download.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        LazyLock, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::Duration,
};

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tauri::Emitter;

/// Lines per chunk at most, and the overlap when a long definition is cut into windows.
const CHUNK_LINES: usize = 60;
const WINDOW_OVERLAP: usize = 8;
/// Shorter pieces join the next one.
const MIN_CHUNK_LINES: usize = 6;
const CHUNK_CHARS: usize = 2_000;

/// Projects whose index is kept up to date after edits (the ones semantic search was used in).
static ACTIVE: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(Default::default);
/// Query embeddings the window still owes, by request id.
static WAITING: LazyLock<Mutex<HashMap<u64, mpsc::Sender<Result<(String, Vec<f32>), String>>>>> = LazyLock::new(Default::default);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
/// Whether a query was answered yet this run; the first one may wait for the model download.
static WARM: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|p| p.into_inner())
}

fn root_key(root: &Path) -> String {
    root.to_string_lossy().to_string()
}

// ---------------------------------------------------------------------------------------------
// Chunks
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    /// 1-based, inclusive.
    pub start: usize,
    pub end: usize,
    pub text: String,
}

/// Cuts a file at its top-level definitions (`starts`, 1-based lines), joins tiny pieces and
/// windows long ones. Each chunk's text starts with the path, which helps the embedding.
pub fn chunk(path: &str, text: &str, starts: &[u32]) -> Vec<Chunk> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let mut bounds: Vec<usize> = starts.iter().map(|&line| line as usize).filter(|&line| line >= 1 && line <= lines.len()).collect();
    bounds.push(1);
    bounds.sort_unstable();
    bounds.dedup();
    let mut pieces: Vec<(usize, usize)> = bounds.windows(2).map(|pair| (pair[0], pair[1] - 1)).collect();
    pieces.push((*bounds.last().unwrap_or(&1), lines.len()));
    // Join pieces shorter than MIN_CHUNK_LINES to the next.
    let mut joined: Vec<(usize, usize)> = Vec::new();
    let mut carry: Option<usize> = None;
    for (start, end) in pieces {
        let start = carry.take().unwrap_or(start);
        if end + 1 - start < MIN_CHUNK_LINES {
            carry = Some(start);
            continue;
        }
        joined.push((start, end));
    }
    if let Some(start) = carry {
        match joined.last_mut() {
            Some(last) => last.1 = lines.len(),
            None => joined.push((start, lines.len())),
        }
    }
    let mut out = Vec::new();
    for (start, end) in joined {
        let mut at = start;
        loop {
            let stop = (at + CHUNK_LINES - 1).min(end);
            let body = lines[at - 1..stop].join("\n");
            if !body.trim().is_empty() {
                let text: String = format!("{path}\n{body}").chars().take(CHUNK_CHARS).collect();
                out.push(Chunk { start: at, end: stop, text });
            }
            if stop >= end {
                break;
            }
            at = stop + 1 - WINDOW_OVERLAP;
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Vector store
// ---------------------------------------------------------------------------------------------

pub fn open(path: &Path) -> Result<Connection, String> {
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    conn.busy_timeout(Duration::from_secs(5)).map_err(|e| e.to_string())?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS chunks (
            root TEXT NOT NULL, path TEXT NOT NULL, mtime INTEGER NOT NULL, chunk INTEGER NOT NULL,
            start_line INTEGER NOT NULL, end_line INTEGER NOT NULL, model TEXT NOT NULL, vector BLOB NOT NULL,
            PRIMARY KEY (root, path, chunk));
         CREATE INDEX IF NOT EXISTS chunks_model ON chunks(root, model);",
    )
    .map_err(|e| e.to_string())?;
    Ok(conn)
}

fn database() -> Result<Connection, String> {
    let dir = crate::workspace::data_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    open(&dir.join("semantic.sqlite"))
}

fn to_blob(vector: &[f32]) -> Vec<u8> {
    vector.iter().flat_map(|value| value.to_le_bytes()).collect()
}

fn from_blob(blob: &[u8]) -> Vec<f32> {
    blob.chunks_exact(4).map(|bytes| f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])).collect()
}

/// One embedded chunk on its way into the store.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Stored {
    pub path: String,
    pub mtime: i64,
    pub chunk: u32,
    pub start: u32,
    pub end: u32,
    pub vector: Vec<f32>,
}

/// Replaces what the store has for each file in `items` (older versions and other models go).
pub fn store(conn: &mut Connection, root: &str, model: &str, items: &[Stored]) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;
    let mut cleared: HashSet<&str> = HashSet::new();
    for item in items {
        if cleared.insert(&item.path) {
            tx.execute("DELETE FROM chunks WHERE root = ?1 AND path = ?2 AND (mtime != ?3 OR model != ?4)", params![root, item.path, item.mtime, model]).map_err(|e| e.to_string())?;
        }
        tx.execute(
            "INSERT OR REPLACE INTO chunks (root, path, mtime, chunk, start_line, end_line, model, vector) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![root, item.path, item.mtime, item.chunk, item.start, item.end, model, to_blob(&item.vector)],
        )
        .map_err(|e| e.to_string())?;
    }
    tx.commit().map_err(|e| e.to_string())
}

/// The modification time each file was embedded at with `model`.
pub fn embedded(conn: &Connection, root: &str, model: &str) -> Result<HashMap<String, i64>, String> {
    let mut statement = conn.prepare("SELECT DISTINCT path, mtime FROM chunks WHERE root = ?1 AND model = ?2").map_err(|e| e.to_string())?;
    let rows = statement.query_map(params![root, model], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))).map_err(|e| e.to_string())?;
    Ok(rows.flatten().collect())
}

/// Drops files that are gone from the project, and vectors of other models.
pub fn prune(conn: &Connection, root: &str, model: &str, live: &HashSet<String>) -> Result<usize, String> {
    let mut removed = conn.execute("DELETE FROM chunks WHERE root = ?1 AND model != ?2", params![root, model]).map_err(|e| e.to_string())?;
    for path in embedded(conn, root, model)?.into_keys().filter(|path| !live.contains(path)) {
        removed += conn.execute("DELETE FROM chunks WHERE root = ?1 AND path = ?2", params![root, path]).map_err(|e| e.to_string())?;
    }
    Ok(removed)
}

/// The `k` chunks most similar to `query` by cosine: (path, start, end, score), best first.
pub fn nearest(conn: &Connection, root: &str, model: &str, query: &[f32], k: usize) -> Result<Vec<(String, u32, u32, f32)>, String> {
    let query_norm = query.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-12);
    let mut statement = conn.prepare("SELECT path, start_line, end_line, vector FROM chunks WHERE root = ?1 AND model = ?2").map_err(|e| e.to_string())?;
    let rows = statement
        .query_map(params![root, model], |row| Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?, row.get::<_, u32>(2)?, row.get::<_, Vec<u8>>(3)?)))
        .map_err(|e| e.to_string())?;
    let mut scored: Vec<(String, u32, u32, f32)> = Vec::new();
    for (path, start, end, blob) in rows.flatten() {
        let vector = from_blob(&blob);
        if vector.len() != query.len() {
            continue;
        }
        let dot: f32 = vector.iter().zip(query).map(|(a, b)| a * b).sum();
        let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt().max(1e-12);
        scored.push((path, start, end, dot / (norm * query_norm)));
    }
    scored.sort_by(|a, b| b.3.total_cmp(&a.3));
    // Overlapping windows of one definition say the same thing; keep the best of each region.
    let mut kept: Vec<(String, u32, u32, f32)> = Vec::new();
    for hit in scored {
        if kept.iter().any(|known| known.0 == hit.0 && known.1 <= hit.2 && hit.1 <= known.2) {
            continue;
        }
        kept.push(hit);
        if kept.len() >= k {
            break;
        }
    }
    Ok(kept)
}

// ---------------------------------------------------------------------------------------------
// The window's side: chunks out, vectors in
// ---------------------------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChunkJob {
    pub path: String,
    pub mtime: i64,
    pub chunk: u32,
    pub start: u32,
    pub end: u32,
    pub text: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Batch {
    pub files: usize,
    pub embedded: usize,
    pub chunks: Vec<ChunkJob>,
}

/// The next whole files to embed (about `limit` chunks), with progress, for `model`. Empty
/// chunks mean the project is up to date.
#[tauri::command]
pub async fn semantic_chunks(root: String, model: String, limit: Option<usize>) -> Result<Batch, String> {
    tauri::async_runtime::spawn_blocking(move || next_batch(Path::new(&root), &model, limit.unwrap_or(32).clamp(1, 256))).await.map_err(|e| e.to_string())?
}

fn next_batch(root: &Path, model: &str, limit: usize) -> Result<Batch, String> {
    let handle = crate::index::ready(root, Duration::from_secs(30)).ok_or("The project index is still building")?;
    let sources = handle.sources();
    let conn = database()?;
    let key = root_key(root);
    let live: HashSet<String> = sources.iter().map(|(path, ..)| path.clone()).collect();
    prune(&conn, &key, model, &live)?;
    let done = embedded(&conn, &key, model)?;
    let mut embedded_files = 0;
    let mut chunks: Vec<ChunkJob> = Vec::new();
    // The files the agent works with first, so the index is useful before it is complete.
    let focus: HashSet<String> = crate::repomap::focus(root).into_iter().collect();
    let mut order: Vec<&(String, i64, &'static str, Vec<u32>)> = sources.iter().collect();
    order.sort_by_key(|(path, ..)| !focus.contains(path));
    for (path, mtime, _, starts) in order {
        if done.get(path) == Some(mtime) {
            embedded_files += 1;
            continue;
        }
        if chunks.len() >= limit {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(root.join(path)) else { continue };
        for (index, piece) in chunk(path, &text, starts).into_iter().enumerate() {
            chunks.push(ChunkJob { path: path.clone(), mtime: *mtime, chunk: index as u32, start: piece.start as u32, end: piece.end as u32, text: piece.text });
        }
    }
    Ok(Batch { files: sources.len(), embedded: embedded_files, chunks })
}

#[tauri::command]
pub async fn semantic_store(root: String, model: String, items: Vec<Stored>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || store(&mut database()?, &root, &model, &items)).await.map_err(|e| e.to_string())?
}

/// The window's answer to a query embedding request.
#[tauri::command]
pub fn semantic_reply(id: u64, model: Option<String>, vector: Option<Vec<f32>>, error: Option<String>) {
    if let Some(sender) = lock(&WAITING).remove(&id) {
        let _ = sender.send(match (model, vector) {
            (Some(model), Some(vector)) if !vector.is_empty() => Ok((model, vector)),
            _ => Err(error.unwrap_or_else(|| "no embedding".into())),
        });
    }
}

/// After the app changed files: the window re-embeds them, in projects that use semantic search.
pub fn note_changed(root: &Path) {
    if lock(&ACTIVE).contains(root) {
        if let Some(app) = crate::index::app() {
            let _ = app.emit("semantic://index", json!({"root": root_key(root)}));
        }
    }
}

/// The `semantic_search` tool. Blocks (on a worker thread) until the window embedded the query.
pub fn search(root: &Path, query: &str, k: usize) -> Result<String, String> {
    let query = query.trim();
    if query.chars().count() < 3 {
        return Err("Query is too short".into());
    }
    let app = crate::index::app().ok_or("Semantic search needs the Neru window; use search_text")?;
    lock(&ACTIVE).insert(root.to_path_buf());
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let (sender, receiver) = mpsc::channel();
    lock(&WAITING).insert(id, sender);
    app.emit("semantic://embed", json!({"id": id, "root": root_key(root), "text": query})).map_err(|e| e.to_string())?;
    // The first query of a run may wait for the model to download and load.
    let wait = if WARM.load(Ordering::Relaxed) { Duration::from_secs(30) } else { Duration::from_secs(180) };
    let reply = receiver.recv_timeout(wait);
    lock(&WAITING).remove(&id);
    let (model, vector) = reply.map_err(|_| "The embedding model did not answer in time (it may still be downloading). Use search_text for now.".to_string())??;
    WARM.store(true, Ordering::Relaxed);
    let conn = database()?;
    let key = root_key(root);
    let hits = nearest(&conn, &key, &model, &vector, k)?;
    let total = crate::index::ready(root, Duration::from_secs(5)).map_or(0, |handle| handle.sources().len());
    let done = embedded(&conn, &key, &model)?.len();
    let coverage = if total > 0 && done < total {
        format!("\n[The semantic index covers {done} of {total} files so far and is still building; results may miss code. Combine with search_text.]")
    } else {
        String::new()
    };
    if hits.is_empty() {
        return Ok(format!("No indexed code yet for this query.{coverage}"));
    }
    let mut out = String::new();
    for (path, start, end, score) in hits {
        let preview: Vec<String> = std::fs::read_to_string(root.join(&path))
            .map(|text| text.lines().skip(start.saturating_sub(1) as usize).take((end + 1 - start) as usize).map(str::trim).filter(|line| !line.is_empty()).take(3).map(|line| line.chars().take(120).collect()).collect())
            .unwrap_or_default();
        out.push_str(&format!("{path}:{start}-{end} ({score:.2})\n"));
        for line in preview {
            out.push_str(&format!("    {line}\n"));
        }
    }
    Ok(format!("{}{coverage}", out.trim_end()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_follow_definitions() {
        let text: String = (1..=150).map(|n| format!("line {n}\n")).collect();
        let chunks = chunk("src/a.rs", &text, &[3, 5, 40]);
        // 1-2 and 3-4 are too short and join 5-39; 40-150 is windowed.
        assert_eq!((chunks[0].start, chunks[0].end), (1, 39));
        assert_eq!((chunks[1].start, chunks[1].end), (40, 99));
        assert_eq!(chunks[2].start, 92);
        assert_eq!(chunks.last().unwrap().end, 150);
        assert!(chunks[0].text.starts_with("src/a.rs\nline 1"));
        assert!(chunk("a.rs", "", &[]).is_empty());
        assert_eq!(chunk("a.rs", "x\ny\n", &[2]).len(), 1);
    }

    #[test]
    fn vector_store_round_trip_and_ranking() {
        let path = std::env::temp_dir().join(format!("neru-semantic-{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut conn = open(&path).unwrap();
        let item = |file: &str, mtime: i64, chunk: u32, vector: Vec<f32>| Stored { path: file.into(), mtime, chunk, start: chunk * 10 + 1, end: chunk * 10 + 10, vector };
        store(&mut conn, "R", "m", &[item("a.rs", 1, 0, vec![1.0, 0.0, 0.0]), item("a.rs", 1, 1, vec![0.0, 1.0, 0.0]), item("b.rs", 5, 0, vec![0.7, 0.7, 0.0])]).unwrap();
        store(&mut conn, "other", "m", &[item("a.rs", 1, 0, vec![1.0, 0.0, 0.0])]).unwrap();
        let hits = nearest(&conn, "R", "m", &[1.0, 0.1, 0.0], 2).unwrap();
        assert_eq!((hits[0].0.as_str(), hits[0].1), ("a.rs", 1));
        assert_eq!(hits[1].0, "b.rs");
        assert!(hits[0].3 > hits[1].3);
        assert_eq!(embedded(&conn, "R", "m").unwrap(), HashMap::from([("a.rs".to_string(), 1), ("b.rs".to_string(), 5)]));
        // A new version of a.rs replaces both of its old chunks.
        store(&mut conn, "R", "m", &[item("a.rs", 2, 0, vec![0.0, 0.0, 1.0])]).unwrap();
        assert_eq!(nearest(&conn, "R", "m", &[0.0, 1.0, 0.0], 10).unwrap().iter().filter(|hit| hit.0 == "a.rs").count(), 1);
        // Deleted files and other models are pruned; other projects are untouched.
        assert_eq!(prune(&conn, "R", "m", &HashSet::from(["a.rs".to_string()])).unwrap(), 1);
        assert_eq!(embedded(&conn, "R", "m").unwrap().len(), 1);
        assert_eq!(embedded(&conn, "other", "m").unwrap().len(), 1);
        assert!(nearest(&conn, "R", "other-model", &[1.0, 0.0, 0.0], 3).unwrap().is_empty());
        drop(conn);
        let _ = std::fs::remove_file(&path);
    }
}
