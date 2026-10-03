//! A fast in-memory index of the open project, built on background threads.
//!
//! * A parallel, `.gitignore`-aware walk lists every text file with its size, modification time,
//!   language and line count.
//! * A trigram inverted index narrows content search to the few files that can match, so a search
//!   over a large repository reads a handful of files instead of all of them.
//! * A cheap regex outline (functions, classes, types, headings) feeds the project map the model
//!   sees, so it does not burn rounds listing folders.
//! * There is no file-watcher crate in the offline dependency set, so freshness comes from
//!   three places: the agent's own writes update entries directly, a stat-only rescan runs before
//!   searches and on an adaptive timer, and the window asks for one when it regains focus.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, LazyLock, Mutex, MutexGuard, OnceLock, RwLock, Weak,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use ignore::{WalkBuilder, WalkState};
use regex::Regex;
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, State};

use crate::{
    AppState,
    workspace::{SearchHit, line_hits, project_root, relative_path},
};

/// Folders that never hold source a person or agent should search: dependencies, build output, caches.
const SKIP_DIRS: &[&str] = &[
    ".git", "node_modules", "target", "dist", ".next", ".nuxt", ".svelte-kit", ".turbo", ".cache", ".gradle", ".venv", "venv",
    "__pycache__", ".pytest_cache", ".mypy_cache", "coverage", ".local", ".parcel-cache", ".angular", ".idea", ".vs",
    // The oh-my-claudecode plugin's state for Claude Code.
    ".omc",
];

const BINARY_EXTS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "ico", "icns", "bmp", "tiff", "avif", "woff", "woff2", "ttf", "otf", "eot", "mp3", "mp4", "mov",
    "avi", "webm", "wav", "ogg", "flac", "zip", "gz", "tgz", "7z", "rar", "tar", "pdf", "exe", "dll", "so", "dylib", "bin", "o", "a",
    "lib", "class", "jar", "wasm", "pyc", "db", "sqlite", "psd", "ai", "docx", "xlsx", "pptx", "map", "onnx", "pt", "safetensors", "gguf",
];

const LOCK_FILES: &[&str] = &["Cargo.lock", "package-lock.json", "yarn.lock", "pnpm-lock.yaml", "bun.lockb", "poetry.lock", "composer.lock", "Gemfile.lock"];

/// Files above this are listed but their content is not indexed or searched.
const MAX_INDEXED_BYTES: u64 = 1_000_000;
const MAX_SYMBOLS_PER_FILE: usize = 300;
/// Text past this many bytes is still listed and searched, but read from disk on every search
/// instead of being trigram-indexed, which keeps the index's memory in check on huge repositories
/// (the trigram data costs a few times the size of the text it covers).
const INDEX_BUDGET_BYTES: u64 = 160_000_000;
/// A rescan more recent than this is trusted by the tools; older ones run a stat-only diff first.
const STALE_AFTER: Duration = Duration::from_secs(3);

pub fn is_skipped_dir(name: &str) -> bool {
    SKIP_DIRS.contains(&name)
}

fn is_binary_path(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, ext)| BINARY_EXTS.contains(&ext.to_ascii_lowercase().as_str()))
}

fn is_lock_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    LOCK_FILES.contains(&name) || name.ends_with(".min.js") || name.ends_with(".min.css")
}

/// A temp file `write_atomic` leaves for a moment; never worth indexing.
fn is_temp_name(name: &str) -> bool {
    name.starts_with('.') && name.contains(".neru-") && name.ends_with(".tmp")
}

fn skipped_path(rel: &str) -> bool {
    rel.split('/').any(is_skipped_dir)
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

fn mtime_ms(meta: &fs::Metadata) -> i64 {
    meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_millis() as i64)
}

fn workers() -> usize {
    thread::available_parallelism().map_or(4, |n| n.get()).clamp(2, 12)
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ---------------------------------------------------------------------------------------------
// Records
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Symbol {
    pub name: String,
    pub kind: &'static str,
    /// 1-based line.
    pub line: u32,
    /// 0 at the top level of the file, 1 when indented (methods, nested items).
    pub depth: u8,
}

#[derive(Clone, Debug)]
pub struct FileRec {
    /// Project-relative, `/` separated.
    pub path: String,
    pub size: u64,
    pub mtime: i64,
    pub lang: &'static str,
    pub lines: u32,
    /// False for binary files and files too large to index.
    pub text: bool,
    /// Whether the trigram data is complete; a text file that is not is scanned by every search.
    pub indexed: bool,
    pub symbols: Vec<Symbol>,
    /// Sorted, deduplicated lowercase trigrams of the content.
    tri: Vec<u32>,
}

#[derive(Clone, Debug)]
struct Stat {
    path: String,
    size: u64,
    mtime: i64,
}

pub fn language(path: &str) -> &'static str {
    let name = path.rsplit('/').next().unwrap_or(path);
    let ext = name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "ts" | "tsx" | "mts" | "cts" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "py" | "pyw" => "python",
        "rs" => "rust",
        "go" => "go",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "cs" => "csharp",
        "swift" => "swift",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => "cpp",
        "rb" => "ruby",
        "php" => "php",
        "css" | "scss" | "less" => "css",
        "html" | "htm" | "vue" | "svelte" | "astro" => "html",
        "json" | "jsonc" => "json",
        "md" | "mdx" | "markdown" => "markdown",
        "toml" => "toml",
        "yml" | "yaml" => "yaml",
        "sh" | "bash" | "zsh" => "shell",
        "ps1" | "psm1" => "powershell",
        "sql" => "sql",
        _ => "",
    }
}

// ---------------------------------------------------------------------------------------------
// Symbol extraction (regex outline; not a parser, but right for navigation)
// ---------------------------------------------------------------------------------------------

macro_rules! re {
    ($name:ident, $pattern:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pattern).expect("valid symbol pattern"));
    };
}

re!(TS_DECL, r"^(\s*)(?:export\s+)?(?:default\s+)?(?:declare\s+)?(?:abstract\s+)?(?:async\s+)?(function\*?|class|interface|type|enum|namespace)\s+([A-Za-z_$][\w$]*)");
re!(TS_EXPORT_VAR, r"^(\s*)export\s+(?:declare\s+)?(const|let|var)\s+([A-Za-z_$][\w$]*)");
re!(TS_ARROW, r"^(\s*)(?:export\s+)?(?:const|let|var)\s+([A-Za-z_$][\w$]*)\s*(?::[^=]+)?=\s*(?:async\s*)?(?:\([^)]*\)|[A-Za-z_$][\w$]*)\s*(?::[^=]+)?=>");
re!(PY_DECL, r"^(\s*)(?:async\s+)?(def|class)\s+([A-Za-z_]\w*)");
re!(RS_DECL, r"^(\s*)(?:pub(?:\([^)]*\))?\s+)?(?:default\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?(?:extern\s+\S+\s+)?(fn|struct|enum|trait|type|mod|union|static|const)\s+(?:mut\s+)?([A-Za-z_]\w*)");
re!(RS_IMPL, r"^(\s*)(?:unsafe\s+)?impl(?:<[^>]*>)?\s+(?:[\w:]+(?:<[^>]*>)?\s+for\s+)?([A-Za-z_]\w*)");
re!(RS_MACRO, r"^(\s*)macro_rules!\s+([A-Za-z_]\w*)");
re!(GO_FUNC, r"^func\s+(?:\([^)]*\)\s*)?([A-Za-z_]\w*)");
re!(GO_TYPE, r"^type\s+([A-Za-z_]\w*)\s+(struct|interface)");
re!(JVM_DECL, r"^(\s*)(?:@\w+(?:\([^)]*\))?\s+)*(?:(?:public|private|protected|internal|static|final|abstract|sealed|data|open|override|suspend|partial|export|default)\s+)*(class|interface|enum|record|object|struct|fun)\s+([A-Za-z_]\w*)");
re!(CSS_RULE, r"^\s{0,2}(\.[A-Za-z_-][\w-]*|#[A-Za-z_-][\w-]*|@keyframes\s+[\w-]+|:root)\b[^;{]*\{");
re!(HTML_ID, r#"\bid\s*=\s*["']([A-Za-z][\w-]*)["']"#);
re!(HTML_TITLE, r"<title>([^<]{1,80})");
re!(MD_HEADING, r"^(#{1,3})\s+(.{1,80}?)\s*#*\s*$");

fn depth_of(indent: &str) -> u8 {
    u8::from(!indent.is_empty())
}

pub fn extract_symbols(lang: &str, text: &str) -> Vec<Symbol> {
    let mut out: Vec<Symbol> = Vec::new();
    let mut in_fence = false;
    let mut css_seen: HashSet<String> = HashSet::new();
    let push = |out: &mut Vec<Symbol>, name: &str, kind: &'static str, line: usize, depth: u8| {
        if out.len() < MAX_SYMBOLS_PER_FILE {
            out.push(Symbol { name: name.to_string(), kind, line: line as u32 + 1, depth });
        }
    };
    for (i, line) in text.lines().enumerate() {
        if line.len() > 400 || out.len() >= MAX_SYMBOLS_PER_FILE {
            continue;
        }
        match lang {
            "typescript" | "javascript" => {
                if !(line.contains("function") || line.contains("class ") || line.contains("interface ") || line.contains("type ") || line.contains("enum ") || line.contains("const ") || line.contains("let ") || line.contains("var ") || line.contains("namespace ")) {
                    continue;
                }
                if let Some(c) = TS_DECL.captures(line) {
                    let kind = match &c[2] {
                        "class" => "class",
                        "interface" => "interface",
                        "type" => "type",
                        "enum" => "enum",
                        "namespace" => "namespace",
                        _ => "function",
                    };
                    push(&mut out, &c[3], kind, i, depth_of(&c[1]));
                } else if let Some(c) = TS_EXPORT_VAR.captures(line) {
                    let kind = if TS_ARROW.is_match(line) { "function" } else { "const" };
                    push(&mut out, &c[3], kind, i, depth_of(&c[1]));
                } else if let Some(c) = TS_ARROW.captures(line) {
                    if c[1].is_empty() {
                        push(&mut out, &c[2], "function", i, 0);
                    }
                }
            }
            "python" => {
                if !(line.contains("def ") || line.contains("class ")) {
                    continue;
                }
                if let Some(c) = PY_DECL.captures(line) {
                    let depth = depth_of(&c[1]);
                    if c[1].len() <= 8 {
                        push(&mut out, &c[3], if &c[2] == "class" { "class" } else if depth == 0 { "function" } else { "method" }, i, depth);
                    }
                }
            }
            "rust" => {
                if let Some(c) = RS_DECL.captures(line) {
                    let kind = match &c[2] {
                        "fn" => "function",
                        "struct" => "struct",
                        "enum" => "enum",
                        "trait" => "trait",
                        "type" => "type",
                        "mod" => "module",
                        "union" => "union",
                        _ => "const",
                    };
                    push(&mut out, &c[3], kind, i, depth_of(&c[1]));
                } else if let Some(c) = RS_IMPL.captures(line) {
                    push(&mut out, &c[2], "impl", i, depth_of(&c[1]));
                } else if let Some(c) = RS_MACRO.captures(line) {
                    push(&mut out, &c[2], "macro", i, depth_of(&c[1]));
                }
            }
            "go" => {
                if let Some(c) = GO_FUNC.captures(line) {
                    push(&mut out, &c[1], "function", i, 0);
                } else if let Some(c) = GO_TYPE.captures(line) {
                    push(&mut out, &c[1], if &c[2] == "struct" { "struct" } else { "interface" }, i, 0);
                }
            }
            "java" | "kotlin" | "csharp" | "swift" => {
                if let Some(c) = JVM_DECL.captures(line) {
                    push(&mut out, &c[3], if &c[2] == "fun" { "function" } else { "class" }, i, depth_of(&c[1]));
                }
            }
            "css" => {
                if let Some(c) = CSS_RULE.captures(line) {
                    let name = c[1].to_string();
                    if css_seen.insert(name.clone()) && css_seen.len() <= 40 {
                        push(&mut out, &name, "rule", i, 0);
                    }
                }
            }
            "html" => {
                if let Some(c) = HTML_TITLE.captures(line) {
                    push(&mut out, c[1].trim(), "title", i, 0);
                }
                if out.len() < 25 {
                    for c in HTML_ID.captures_iter(line).take(3) {
                        push(&mut out, &format!("#{}", &c[1]), "id", i, 0);
                    }
                }
            }
            "markdown" => {
                if line.trim_start().starts_with("```") || line.trim_start().starts_with("~~~") {
                    in_fence = !in_fence;
                } else if !in_fence {
                    if let Some(c) = MD_HEADING.captures(line) {
                        push(&mut out, &c[2], "heading", i, (c[1].len() - 1) as u8);
                    }
                }
            }
            _ => break,
        }
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Trigrams
// ---------------------------------------------------------------------------------------------

fn tri_key(a: u8, b: u8, c: u8) -> u32 {
    (u32::from(a.to_ascii_lowercase()) << 16) | (u32::from(b.to_ascii_lowercase()) << 8) | u32::from(c.to_ascii_lowercase())
}

fn trigrams(bytes: &[u8]) -> Vec<u32> {
    let mut out: Vec<u32> = Vec::with_capacity(bytes.len().min(1 << 16));
    for w in bytes.windows(3) {
        // Matches never span lines, so trigrams with a line break are useless.
        if w[0] == b'\n' || w[1] == b'\n' || w[2] == b'\n' || w[0] == b'\r' || w[1] == b'\r' || w[2] == b'\r' {
            continue;
        }
        out.push(tri_key(w[0], w[1], w[2]));
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Literal strings (lowercase, at least three bytes) that every match must contain at least one
/// of, or None when no safe filter exists and every text file has to be scanned.
pub fn needles_for(pattern: &str, regex_mode: bool, case_sensitive: bool) -> Option<Vec<Vec<u8>>> {
    let lower = |bytes: &[u8]| bytes.to_ascii_lowercase();
    if !regex_mode {
        if pattern.len() < 3 || (!case_sensitive && !pattern.is_ascii()) {
            return None;
        }
        return Some(vec![lower(pattern.as_bytes())]);
    }
    // Parsed case-sensitively on purpose: the index is lowercase, so the lowercased literal of a
    // pattern is what any case variant of a match contains, and case-folding would only explode
    // the literal set past what the extractor keeps.
    let hir = regex_syntax::ParserBuilder::new().build().parse(pattern).ok()?;
    let seq = regex_syntax::hir::literal::Extractor::new().extract(&hir);
    let literals = seq.literals()?;
    if literals.is_empty() || literals.len() > 64 {
        return None;
    }
    let mut needles = Vec::new();
    for literal in literals {
        let bytes = literal.as_bytes();
        if bytes.len() < 3 || (!case_sensitive && !bytes.is_ascii()) {
            return None;
        }
        needles.push(lower(bytes));
    }
    needles.sort();
    needles.dedup();
    Some(needles)
}

fn intersect(a: &[u32], b: &[u32]) -> Vec<u32> {
    // `a` is the shorter list.
    if a.len().saturating_mul(24) < b.len() {
        return a.iter().copied().filter(|id| b.binary_search(id).is_ok()).collect();
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::with_capacity(a.len());
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                out.push(a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    out
}

fn union(mut lists: Vec<Vec<u32>>) -> Vec<u32> {
    if lists.len() == 1 {
        return lists.pop().unwrap_or_default();
    }
    let mut all: Vec<u32> = lists.into_iter().flatten().collect();
    all.sort_unstable();
    all.dedup();
    all
}

// ---------------------------------------------------------------------------------------------
// The index
// ---------------------------------------------------------------------------------------------

pub struct Index {
    files: Vec<Option<FileRec>>,
    by_path: BTreeMap<String, u32>,
    postings: HashMap<u32, Vec<u32>>,
    /// Text files without trigram data (over the budget).
    loose: HashSet<u32>,
    generation: u64,
}

impl Index {
    fn new() -> Self {
        Self { files: Vec::new(), by_path: BTreeMap::new(), postings: HashMap::new(), loose: HashSet::new(), generation: 0 }
    }

    pub fn from_records(records: Vec<FileRec>) -> Self {
        let mut index = Self::new();
        for record in records {
            index.upsert(record);
        }
        index
    }

    pub fn len(&self) -> usize {
        self.by_path.len()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn file(&self, path: &str) -> Option<&FileRec> {
        self.by_path.get(path).and_then(|&id| self.files[id as usize].as_ref())
    }

    pub fn records(&self) -> impl Iterator<Item = &FileRec> {
        self.by_path.values().filter_map(|&id| self.files[id as usize].as_ref())
    }

    pub fn symbol_count(&self) -> usize {
        self.records().map(|f| f.symbols.len()).sum()
    }

    pub fn total_bytes(&self) -> u64 {
        self.records().map(|f| f.size).sum()
    }

    /// Inserts a file or replaces its previous entry.
    pub fn upsert(&mut self, mut record: FileRec) {
        self.generation += 1;
        let tri = std::mem::take(&mut record.tri);
        let id = match self.by_path.get(&record.path) {
            Some(&id) => {
                self.unpost(id);
                id
            }
            None => {
                let id = self.files.len() as u32;
                self.files.push(None);
                self.by_path.insert(record.path.clone(), id);
                id
            }
        };
        for key in &tri {
            let list = self.postings.entry(*key).or_default();
            match list.last() {
                Some(&last) if last < id => list.push(id),
                None => list.push(id),
                _ => {
                    if let Err(at) = list.binary_search(&id) {
                        list.insert(at, id);
                    }
                }
            }
        }
        record.tri = tri;
        if record.text && !record.indexed {
            self.loose.insert(id);
        } else {
            self.loose.remove(&id);
        }
        self.files[id as usize] = Some(record);
    }

    fn unpost(&mut self, id: u32) {
        let Some(old) = self.files[id as usize].as_mut() else { return };
        for key in std::mem::take(&mut old.tri) {
            if let Some(list) = self.postings.get_mut(&key) {
                if let Ok(at) = list.binary_search(&id) {
                    list.remove(at);
                }
                if list.is_empty() {
                    self.postings.remove(&key);
                }
            }
        }
    }

    pub fn remove(&mut self, path: &str) -> bool {
        let Some(id) = self.by_path.remove(path) else { return false };
        self.generation += 1;
        self.unpost(id);
        self.loose.remove(&id);
        self.files[id as usize] = None;
        true
    }

    /// Paths equal to `prefix` or inside the folder `prefix`.
    fn under(&self, prefix: &str) -> Vec<String> {
        if prefix.is_empty() {
            return self.by_path.keys().cloned().collect();
        }
        let folder = format!("{prefix}/");
        let mut out: Vec<String> = self.by_path.range(folder.clone()..).take_while(|(path, _)| path.starts_with(&folder)).map(|(path, _)| path.clone()).collect();
        if self.by_path.contains_key(prefix) {
            out.push(prefix.to_string());
        }
        out
    }

    pub fn count_under(&self, prefix: &str) -> usize {
        if prefix.is_empty() {
            return self.by_path.len();
        }
        let folder = format!("{prefix}/");
        self.by_path.range(folder.clone()..).take_while(|(path, _)| path.starts_with(&folder)).count()
    }

    /// Ids of files that contain at least one needle, or every text file when `needles` is None.
    fn candidates(&self, needles: Option<&[Vec<u8>]>) -> Vec<u32> {
        let Some(needles) = needles else {
            return self.by_path.values().copied().filter(|&id| self.files[id as usize].as_ref().is_some_and(|f| f.text)).collect();
        };
        let mut per_needle: Vec<Vec<u32>> = Vec::new();
        for needle in needles {
            let mut keys: Vec<u32> = needle.windows(3).map(|w| tri_key(w[0], w[1], w[2])).collect();
            keys.sort_unstable();
            keys.dedup();
            let mut lists: Vec<&Vec<u32>> = Vec::with_capacity(keys.len());
            let mut missing = false;
            for key in &keys {
                match self.postings.get(key) {
                    Some(list) => lists.push(list),
                    None => {
                        missing = true;
                        break;
                    }
                }
            }
            if missing || lists.is_empty() {
                continue;
            }
            lists.sort_by_key(|list| list.len());
            let mut current = lists[0].clone();
            for list in &lists[1..] {
                if current.is_empty() {
                    break;
                }
                current = intersect(&current, list);
            }
            per_needle.push(current);
        }
        let mut found = if per_needle.is_empty() { Vec::new() } else { union(per_needle) };
        if !self.loose.is_empty() {
            found.extend(self.loose.iter().copied());
            found.sort_unstable();
            found.dedup();
        }
        found
    }

    fn path_of(&self, id: u32) -> Option<&str> {
        self.files[id as usize].as_ref().map(|f| f.path.as_str())
    }

    /// Files whose stat differs from the index, and indexed paths that are gone.
    fn diff(&self, stats: &[Stat]) -> (Vec<Stat>, Vec<String>) {
        let mut changed = Vec::new();
        let mut seen: HashSet<&str> = HashSet::with_capacity(stats.len());
        for stat in stats {
            seen.insert(stat.path.as_str());
            match self.file(&stat.path) {
                Some(rec) if rec.size == stat.size && rec.mtime == stat.mtime => {}
                _ => changed.push(stat.clone()),
            }
        }
        let removed = self.by_path.keys().filter(|path| !seen.contains(path.as_str())).cloned().collect();
        (changed, removed)
    }

    // ----- queries -----

    /// Fuzzy file finder for the picker: best matches first.
    pub fn fuzzy_files(&self, query: &str, limit: usize) -> Vec<String> {
        let q: Vec<char> = query.trim().to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
        if q.is_empty() {
            return self.by_path.keys().take(limit).cloned().collect();
        }
        let mut scored: Vec<(i32, &String)> = Vec::new();
        for path in self.by_path.keys() {
            if let Some(score) = fuzzy_score(&q, path) {
                scored.push((score, path));
            }
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.len().cmp(&b.1.len())).then_with(|| a.1.cmp(b.1)));
        scored.into_iter().take(limit).map(|(_, path)| path.clone()).collect()
    }

    /// Symbols whose name matches, exact names first.
    pub fn find_symbols(&self, query: &str, limit: usize) -> Vec<(String, Symbol)> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Vec::new();
        }
        let mut scored: Vec<(u8, &str, &Symbol)> = Vec::new();
        for rec in self.records() {
            for symbol in &rec.symbols {
                let name = symbol.name.to_lowercase();
                let score = if name == needle { 3 } else if name.starts_with(&needle) { 2 } else if name.contains(&needle) { 1 } else { 0 };
                if score > 0 {
                    scored.push((score, rec.path.as_str(), symbol));
                }
            }
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)).then(a.2.line.cmp(&b.2.line)));
        scored.into_iter().take(limit).map(|(_, path, symbol)| (path.to_string(), symbol.clone())).collect()
    }

    /// Symbols of one file, one per line with line numbers, indented when nested.
    pub fn outline(&self, path: &str) -> Option<String> {
        let rec = self.file(path)?;
        let mut out = format!("{path} ({} lines, {})\n", rec.lines, if rec.lang.is_empty() { "text" } else { rec.lang });
        if rec.symbols.is_empty() {
            out.push_str("(no symbols found; read the file for its structure)\n");
        }
        for symbol in &rec.symbols {
            out.push_str(&format!("{}L{} {} {}\n", if symbol.depth > 0 { "  " } else { "" }, symbol.line, symbol.kind, symbol.name));
        }
        Some(out)
    }

    /// A compact tree of the project (or the folder `prefix`) that fits `budget` characters:
    /// every folder first, then file names, then top-level symbols, richest detail to the folders
    /// nearest the root first.
    pub fn render_map(&self, prefix: &str, budget: usize) -> String {
        let prefix = prefix.trim_matches('/');
        let mut dirs: BTreeMap<String, Vec<&FileRec>> = BTreeMap::new();
        let mut total = 0usize;
        for path in self.under(prefix) {
            let Some(rec) = self.file(&path) else { continue };
            total += 1;
            if is_lock_path(&rec.path) {
                continue;
            }
            let dir = rec.path.rsplit_once('/').map_or("", |(dir, _)| dir).to_string();
            dirs.entry(dir).or_default().push(rec);
        }
        if dirs.is_empty() {
            return String::new();
        }
        struct Section<'a> {
            dir: String,
            depth: usize,
            files: Vec<&'a FileRec>,
            renders: [String; 3],
        }
        let shown = |dir: &str| if dir.is_empty() { "./".to_string() } else { format!("{dir}/") };
        let mut sections: Vec<Section> = dirs
            .into_iter()
            .map(|(dir, mut files)| {
                files.sort_by(|a, b| a.path.cmp(&b.path));
                let depth = if dir.is_empty() { 0 } else { dir.matches('/').count() + 1 };
                let sources: Vec<&&FileRec> = files.iter().filter(|f| f.text).collect();
                let assets = files.len() - sources.len();
                let mut r0 = format!("{} ({} file{})\n", shown(&dir), files.len(), if files.len() == 1 { "" } else { "s" });
                let r0_len = r0.len();
                r0.truncate(r0_len);
                let mut names: Vec<String> = sources.iter().take(40).map(|f| f.path.rsplit('/').next().unwrap_or(&f.path).to_string()).collect();
                if sources.len() > 40 {
                    names.push(format!("+{} more", sources.len() - 40));
                }
                if assets > 0 {
                    names.push(format!("{assets} asset{}", if assets == 1 { "" } else { "s" }));
                }
                let r1 = format!("{}\n  {}\n", shown(&dir), names.join(", "));
                let mut r2 = format!("{}\n", shown(&dir));
                let mut plain: Vec<String> = Vec::new();
                for rec in sources.iter().take(60) {
                    let name = rec.path.rsplit('/').next().unwrap_or(&rec.path);
                    let top: Vec<&Symbol> = rec.symbols.iter().filter(|s| s.depth == 0).collect();
                    if top.is_empty() {
                        plain.push(name.to_string());
                    } else {
                        let mut list: Vec<String> = top.iter().take(8).map(|s| s.name.clone()).collect();
                        if top.len() > 8 {
                            list.push(format!("+{}", top.len() - 8));
                        }
                        r2.push_str(&format!("  {name} ({}) {}\n", rec.lines, list.join(", ")));
                    }
                }
                if !plain.is_empty() || assets > 0 {
                    let mut rest = plain;
                    if assets > 0 {
                        rest.push(format!("{assets} asset{}", if assets == 1 { "" } else { "s" }));
                    }
                    r2.push_str(&format!("  {}\n", rest.join(", ")));
                }
                Section { dir, depth, files, renders: [r0, r1, r2] }
            })
            .collect();
        let header = format!(
            "{} files in {} folders under {}. Names in a line are top-level symbols; use project_map with path for a folder's detail, outline for line numbers.\n",
            total,
            sections.len(),
            if prefix.is_empty() { "the project" } else { prefix }
        );
        let mut level: Vec<i8> = vec![-1; sections.len()];
        let mut used = header.len();
        // Priority: nearest the root first, then smaller folders.
        let mut order: Vec<usize> = (0..sections.len()).collect();
        order.sort_by(|&a, &b| sections[a].depth.cmp(&sections[b].depth).then(sections[a].files.len().cmp(&sections[b].files.len())).then(sections[a].dir.cmp(&sections[b].dir)));
        let mut omitted = 0usize;
        for &i in &order {
            let cost = sections[i].renders[0].len();
            if used + cost <= budget {
                level[i] = 0;
                used += cost;
            } else {
                omitted += 1;
            }
        }
        for target in 1..=2i8 {
            for &i in &order {
                if level[i] != target - 1 {
                    continue;
                }
                let delta = sections[i].renders[target as usize].len().saturating_sub(sections[i].renders[(target - 1) as usize].len());
                if used + delta <= budget {
                    level[i] = target;
                    used += delta;
                }
            }
        }
        let mut out = header;
        for (i, section) in sections.iter_mut().enumerate() {
            if level[i] >= 0 {
                out.push_str(&section.renders[level[i] as usize]);
            }
        }
        if omitted > 0 {
            out.push_str(&format!("… {omitted} more folders not shown; call project_map with a path to open one.\n"));
        }
        out
    }
}

/// Subsequence score: consecutive letters, word starts and the file name count for more.
fn fuzzy_score(query: &[char], path: &str) -> Option<i32> {
    let lower = path.to_lowercase();
    let name_start = lower.rfind('/').map_or(0, |i| i + 1);
    let chars: Vec<char> = lower.chars().collect();
    let name_start_chars = lower[..name_start].chars().count();
    let mut score = 0i32;
    let mut qi = 0;
    let mut last: Option<usize> = None;
    for (i, &c) in chars.iter().enumerate() {
        if qi < query.len() && c == query[qi] {
            score += 10;
            if last == Some(i.wrapping_sub(1)) {
                score += 15;
            }
            if i == 0 || matches!(chars[i - 1], '/' | '-' | '_' | '.' | ' ') {
                score += 12;
            }
            if i >= name_start_chars {
                score += 6;
            }
            last = Some(i);
            qi += 1;
        }
    }
    if qi < query.len() {
        return None;
    }
    let name: String = chars[name_start_chars..].iter().collect();
    let q: String = query.iter().collect();
    if name == q {
        score += 200;
    } else if name.starts_with(&q) {
        score += 100;
    } else if name.contains(&q) {
        score += 60;
    } else if lower.contains(&q) {
        score += 25;
    }
    Some(score - (path.len() as i32 / 8))
}

// ---------------------------------------------------------------------------------------------
// Building
// ---------------------------------------------------------------------------------------------

fn walker(start: &Path) -> WalkBuilder {
    let mut builder = WalkBuilder::new(start);
    builder.follow_links(false).hidden(false).require_git(false).threads(workers());
    let base = start.to_path_buf();
    builder.filter_entry(move |entry| {
        let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
        !(is_dir && entry.path() != base && is_skipped_dir(&entry.file_name().to_string_lossy()))
    });
    builder
}

struct Sink<'a> {
    local: Vec<Stat>,
    out: &'a Mutex<Vec<Stat>>,
}

impl Drop for Sink<'_> {
    fn drop(&mut self) {
        lock(self.out).append(&mut self.local);
    }
}

/// Lists every indexable file under `root` with its stat, in path order.
fn scan(root: &Path, counter: Option<&AtomicUsize>) -> Vec<Stat> {
    let out = Mutex::new(Vec::new());
    walker(root).build_parallel().run(|| {
        let mut sink = Sink { local: Vec::with_capacity(256), out: &out };
        Box::new(move |result| {
            let Ok(entry) = result else { return WalkState::Continue };
            if !entry.file_type().is_some_and(|t| t.is_file()) || is_temp_name(&entry.file_name().to_string_lossy()) {
                return WalkState::Continue;
            }
            let Ok(meta) = entry.metadata() else { return WalkState::Continue };
            sink.local.push(Stat { path: relative_path(root, entry.path()), size: meta.len(), mtime: mtime_ms(&meta) });
            if let Some(counter) = counter {
                counter.fetch_add(1, Ordering::Relaxed);
            }
            WalkState::Continue
        })
    });
    let mut stats = out.into_inner().unwrap_or_else(|p| p.into_inner());
    stats.sort_by(|a, b| a.path.cmp(&b.path));
    stats
}

/// Files near `target` (a file or folder inside `root`), found by a walk from the root that only
/// enters the folders on the way, so every .gitignore rule applies exactly as in a full scan.
fn scan_near(root: &Path, target: &Path) -> Vec<Stat> {
    let dir = if target.is_dir() { target.to_path_buf() } else { target.parent().unwrap_or(root).to_path_buf() };
    let mut builder = WalkBuilder::new(root);
    builder.follow_links(false).hidden(false).require_git(false).threads(1);
    builder.filter_entry(move |entry| {
        if !entry.file_type().is_some_and(|t| t.is_dir()) || entry.depth() == 0 {
            return true;
        }
        !is_skipped_dir(&entry.file_name().to_string_lossy()) && (dir.starts_with(entry.path()) || entry.path().starts_with(&dir))
    });
    let mut stats = Vec::new();
    for entry in builder.build().flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) || is_temp_name(&entry.file_name().to_string_lossy()) {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            stats.push(Stat { path: relative_path(root, entry.path()), size: meta.len(), mtime: mtime_ms(&meta) });
        }
    }
    stats
}

fn analyze(root: &Path, stat: &Stat, budget: Option<&AtomicU64>) -> FileRec {
    let mut rec = FileRec { path: stat.path.clone(), size: stat.size, mtime: stat.mtime, lang: language(&stat.path), lines: 0, text: false, indexed: false, symbols: Vec::new(), tri: Vec::new() };
    if stat.size == 0 || stat.size > MAX_INDEXED_BYTES || is_binary_path(&stat.path) {
        return rec;
    }
    let Ok(bytes) = fs::read(root.join(&stat.path)) else { return rec };
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return rec;
    }
    rec.text = true;
    rec.lines = bytes.iter().filter(|&&b| b == b'\n').count() as u32 + u32::from(bytes.last().is_some_and(|&b| b != b'\n'));
    if budget.is_none_or(|used| used.fetch_add(stat.size, Ordering::Relaxed) + stat.size <= INDEX_BUDGET_BYTES) {
        rec.tri = trigrams(&bytes);
        rec.indexed = true;
    }
    if !rec.lang.is_empty() {
        let text = String::from_utf8_lossy(&bytes);
        rec.symbols = extract_symbols(rec.lang, &text);
    }
    rec
}

fn analyze_all(root: &Path, stats: &[Stat], progress: Option<&AtomicUsize>, budget: Option<&AtomicU64>) -> Vec<FileRec> {
    if stats.len() <= 4 {
        return stats.iter().map(|stat| analyze(root, stat, budget)).collect();
    }
    let cursor = AtomicUsize::new(0);
    let results: Mutex<Vec<FileRec>> = Mutex::new(Vec::with_capacity(stats.len()));
    const BATCH: usize = 24;
    thread::scope(|scope| {
        for _ in 0..workers().min(stats.len() / BATCH + 1) {
            scope.spawn(|| {
                let mut local = Vec::new();
                loop {
                    let start = cursor.fetch_add(BATCH, Ordering::Relaxed);
                    if start >= stats.len() {
                        break;
                    }
                    for stat in &stats[start..(start + BATCH).min(stats.len())] {
                        local.push(analyze(root, stat, budget));
                        if let Some(progress) = progress {
                            progress.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                }
                lock(&results).append(&mut local);
            });
        }
    });
    let mut records = results.into_inner().unwrap_or_else(|p| p.into_inner());
    records.sort_by(|a, b| a.path.cmp(&b.path));
    records
}

/// Builds an index synchronously (tests and benchmarks).
#[cfg(test)]
pub fn build_index(root: &Path) -> Index {
    let stats = scan(root, None);
    Index::from_records(analyze_all(root, &stats, None, None))
}

// ---------------------------------------------------------------------------------------------
// Handles, status and events
// ---------------------------------------------------------------------------------------------

#[derive(Serialize, Clone, Default, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub root: String,
    /// "indexing", "ready" or "error".
    pub state: String,
    pub files: usize,
    /// Files to analyze once the walk finishes (0 while walking).
    pub total: usize,
    pub symbols: usize,
    pub bytes: u64,
    pub ms: u64,
    pub scans: u64,
    pub updated_at: u64,
}

static APP: OnceLock<AppHandle> = OnceLock::new();

/// Lets the index tell the window about progress and external changes. Called once at startup.
pub fn set_app(app: AppHandle) {
    let _ = APP.set(app);
}

fn emit_status(status: &Status) {
    if let Some(app) = APP.get() {
        let _ = app.emit("index://progress", status);
    }
}

pub struct Handle {
    pub root: PathBuf,
    index: RwLock<Index>,
    status: Mutex<Status>,
    ready: Condvar,
    last_scan: Mutex<Instant>,
    last_scan_ms: Mutex<u64>,
    last_used: Mutex<Instant>,
    scanning: AtomicBool,
    map_cache: Mutex<HashMap<(String, usize), (u64, String)>>,
}

static REGISTRY: LazyLock<Mutex<HashMap<PathBuf, Arc<Handle>>>> = LazyLock::new(Default::default);
const MAX_HANDLES: usize = 6;

impl Handle {
    fn new(root: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            index: RwLock::new(Index::new()),
            status: Mutex::new(Status { root: root.to_string_lossy().to_string(), state: "indexing".into(), ..Default::default() }),
            root,
            ready: Condvar::new(),
            last_scan: Mutex::new(Instant::now()),
            last_scan_ms: Mutex::new(0),
            last_used: Mutex::new(Instant::now()),
            scanning: AtomicBool::new(false),
            map_cache: Mutex::new(HashMap::new()),
        })
    }

    fn read(&self) -> std::sync::RwLockReadGuard<'_, Index> {
        self.index.read().unwrap_or_else(|p| p.into_inner())
    }

    fn write(&self) -> std::sync::RwLockWriteGuard<'_, Index> {
        self.index.write().unwrap_or_else(|p| p.into_inner())
    }

    pub fn status(&self) -> Status {
        lock(&self.status).clone()
    }

    fn touch(&self) {
        *lock(&self.last_used) = Instant::now();
    }

    fn set_status(&self, update: impl FnOnce(&mut Status)) -> Status {
        let mut status = lock(&self.status);
        update(&mut status);
        status.updated_at = now_ms();
        status.clone()
    }

    /// The first full build: walk, analyze in parallel, publish, then keep fresh.
    fn build(self: &Arc<Self>) {
        let started = Instant::now();
        let counter = Arc::new(AtomicUsize::new(0));
        let total = Arc::new(AtomicUsize::new(0));
        let done = Arc::new(AtomicBool::new(false));
        let ticker = {
            let (handle, counter, total, done) = (self.clone(), counter.clone(), total.clone(), done.clone());
            thread::spawn(move || {
                while !done.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(120));
                    if done.load(Ordering::Relaxed) {
                        break;
                    }
                    let status = handle.set_status(|s| {
                        s.files = counter.load(Ordering::Relaxed);
                        s.total = total.load(Ordering::Relaxed);
                        s.ms = started.elapsed().as_millis() as u64;
                    });
                    emit_status(&status);
                }
            })
        };
        let stats = scan(&self.root, Some(&counter));
        // How long a stat-only scan of this project takes; it sets how often the timer rescans.
        let walk_ms = started.elapsed().as_millis() as u64;
        total.store(stats.len(), Ordering::Relaxed);
        counter.store(0, Ordering::Relaxed);
        let records = analyze_all(&self.root, &stats, Some(&counter), Some(&AtomicU64::new(0)));
        let index = Index::from_records(records);
        let (files, symbols, bytes) = (index.len(), index.symbol_count(), index.total_bytes());
        *self.write() = index;
        done.store(true, Ordering::Relaxed);
        let _ = ticker.join();
        *lock(&self.last_scan) = Instant::now();
        *lock(&self.last_scan_ms) = walk_ms;
        let status = self.set_status(|s| {
            s.state = "ready".into();
            s.files = files;
            s.total = files;
            s.symbols = symbols;
            s.bytes = bytes;
            s.ms = started.elapsed().as_millis() as u64;
        });
        self.ready.notify_all();
        emit_status(&status);
        spawn_poller(Arc::downgrade(self));
    }

    /// Blocks until the first build finishes (or the timeout passes); true when the index is usable.
    pub fn wait_ready(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut status = lock(&self.status);
        while status.state == "indexing" {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            status = self.ready.wait_timeout(status, left).map(|(guard, _)| guard).unwrap_or_else(|p| p.into_inner().0);
        }
        status.state == "ready"
    }

    /// Stat-only diff against disk: re-indexes what changed, drops what is gone. Returns the
    /// changed and removed paths (empty when nothing moved or another scan is running).
    pub fn rescan(&self, announce: bool) -> Vec<String> {
        if self.status().state != "ready" || self.scanning.swap(true, Ordering::AcqRel) {
            return Vec::new();
        }
        let started = Instant::now();
        let stats = scan(&self.root, None);
        let (changed, removed) = self.read().diff(&stats);
        let mut touched: Vec<String> = Vec::new();
        if !changed.is_empty() || !removed.is_empty() {
            let records = analyze_all(&self.root, &changed, None, None);
            let mut index = self.write();
            for record in records {
                touched.push(record.path.clone());
                index.upsert(record);
            }
            for path in &removed {
                index.remove(path);
                touched.push(path.clone());
            }
            drop(index);
            self.refresh_status(started);
            if announce {
                queue_change(touched.clone(), true);
            }
        }
        *lock(&self.last_scan) = Instant::now();
        *lock(&self.last_scan_ms) = started.elapsed().as_millis() as u64;
        self.scanning.store(false, Ordering::Release);
        touched
    }

    fn refresh_status(&self, started: Instant) {
        let (files, symbols, bytes) = {
            let index = self.read();
            (index.len(), index.symbol_count(), index.total_bytes())
        };
        let status = self.set_status(|s| {
            s.files = files;
            s.total = files;
            s.symbols = symbols;
            s.bytes = bytes;
            s.scans += 1;
            s.ms = started.elapsed().as_millis() as u64;
        });
        emit_status(&status);
    }

    /// How old a scan may be before a search re-checks the disk first: a few seconds on a small
    /// project, much longer where one scan takes long (the timer and the agent's own writes keep
    /// those fresh).
    fn stale_after(&self) -> Duration {
        let scan_ms = *lock(&self.last_scan_ms);
        STALE_AFTER.max(Duration::from_millis(scan_ms.saturating_mul(30)).min(Duration::from_secs(600)))
    }

    fn refresh_if_stale(&self) {
        let age = lock(&self.last_scan).elapsed();
        if age > self.stale_after() {
            self.rescan(true);
        }
    }

    /// Updates entries for paths the app itself changed (files, folders, deletions, moves).
    pub fn note_changed(&self, rels: &[String]) {
        if self.status().state != "ready" {
            return;
        }
        let mut touched = false;
        for rel in rels {
            let rel = rel.trim_matches('/').replace('\\', "/");
            if rel.is_empty() || skipped_path(&rel) {
                continue;
            }
            let found: Vec<Stat> = scan_near(&self.root, &self.root.join(&rel)).into_iter().filter(|s| s.path == rel || s.path.starts_with(&format!("{rel}/"))).collect();
            let records = analyze_all(&self.root, &found, None, None);
            let keep: HashSet<String> = found.iter().map(|s| s.path.clone()).collect();
            let mut index = self.write();
            for stale in index.under(&rel) {
                if !keep.contains(&stale) {
                    index.remove(&stale);
                }
            }
            for record in records {
                index.upsert(record);
            }
            touched = true;
        }
        if touched {
            self.refresh_status(Instant::now());
        }
    }

    fn poll_interval(&self) -> Duration {
        let scan_ms = *lock(&self.last_scan_ms);
        let idle = lock(&self.last_used).elapsed();
        // A scan should use a few percent of one core at most, however big the project is.
        let base = Duration::from_millis(scan_ms.saturating_mul(60).clamp(4_000, 300_000));
        if idle > Duration::from_secs(15 * 60) { base.max(Duration::from_secs(120)) } else { base }
    }

    // ----- queries used by tools and commands -----

    pub fn search(&self, query: &Query, matcher: &Regex) -> SearchOutput {
        self.touch();
        let started = Instant::now();
        self.refresh_if_stale();
        let (paths, total) = {
            let index = self.read();
            let needles = query.needles();
            let mut paths: Vec<String> = index
                .candidates(needles.as_deref())
                .into_iter()
                .filter_map(|id| index.path_of(id))
                .filter(|path| query.glob.as_deref().is_none_or(|glob| path_matches(glob, path)))
                .map(str::to_string)
                .collect();
            paths.sort();
            (paths, index.len())
        };
        let (hits, scanned, truncated) = scan_files(&self.root, &paths, matcher, query.limit);
        SearchOutput { hits, files_scanned: scanned, candidates: paths.len(), total_files: total, truncated, ms: started.elapsed().as_millis() as u64 }
    }

    pub fn files_matching(&self, pattern: &str, limit: usize) -> (Vec<String>, usize) {
        self.touch();
        self.refresh_if_stale();
        let index = self.read();
        let mut all: Vec<String> = index.by_path.keys().filter(|path| path_matches(pattern, path)).cloned().collect();
        let count = all.len();
        all.truncate(limit);
        (all, count)
    }

    pub fn fuzzy(&self, query: &str, limit: usize) -> Vec<String> {
        self.touch();
        self.read().fuzzy_files(query, limit)
    }

    pub fn all_paths(&self, limit: usize) -> Vec<String> {
        self.touch();
        self.read().by_path.keys().take(limit).cloned().collect()
    }

    pub fn symbols(&self, query: &str, limit: usize) -> Vec<(String, Symbol)> {
        self.touch();
        self.refresh_if_stale();
        self.read().find_symbols(query, limit)
    }

    pub fn outline(&self, path: &str) -> Option<String> {
        self.touch();
        self.refresh_if_stale();
        self.read().outline(path.trim_matches('/'))
    }

    pub fn map(&self, prefix: &str, budget: usize) -> String {
        self.touch();
        self.refresh_if_stale();
        let index = self.read();
        let key = (prefix.to_string(), budget);
        if let Some((generation, text)) = lock(&self.map_cache).get(&key) {
            if *generation == index.generation() {
                return text.clone();
            }
        }
        let text = index.render_map(prefix, budget);
        let mut cache = lock(&self.map_cache);
        if cache.len() > 16 {
            cache.clear();
        }
        cache.insert(key, (index.generation(), text.clone()));
        text
    }

    /// Every text file of the project, when there are at most `max_files` of them and they add up
    /// to at most `max_bytes`; None for anything larger. Lock files and generated bundles are left
    /// out and do not count.
    pub fn small_text_files(&self, max_files: usize, max_bytes: u64) -> Option<Vec<String>> {
        self.touch();
        self.refresh_if_stale();
        let index = self.read();
        let mut files = Vec::new();
        let mut bytes = 0;
        for record in index.records() {
            let name = record.path.rsplit('/').next().unwrap_or(&record.path);
            let generated = name.ends_with(".lock")
                || name.ends_with("-lock.json")
                || name.ends_with("-lock.yaml")
                || name.ends_with(".min.js")
                || name.ends_with(".min.css")
                || name.ends_with(".map");
            if !record.text || generated || record.size == 0 {
                continue;
            }
            bytes += record.size;
            files.push(record.path.clone());
            if files.len() > max_files || bytes > max_bytes {
                return None;
            }
        }
        Some(files)
    }

    /// (lines, files below) for a path, used to annotate directory listings.
    pub fn describe(&self, rel: &str) -> (Option<u32>, usize, bool) {
        let index = self.read();
        match index.file(rel) {
            Some(rec) => (Some(rec.lines), 0, true),
            None => (None, index.count_under(rel), false),
        }
    }
}

fn spawn_poller(handle: Weak<Handle>) {
    thread::spawn(move || {
        loop {
            let Some(strong) = handle.upgrade() else { break };
            let wait = strong.poll_interval();
            drop(strong);
            thread::sleep(wait);
            let Some(strong) = handle.upgrade() else { break };
            strong.rescan(true);
        }
    });
}

/// The handle for `root`, starting a background build the first time it is asked for.
pub fn handle_for(root: &Path) -> Arc<Handle> {
    let mut registry = lock(&REGISTRY);
    if let Some(handle) = registry.get(root) {
        return handle.clone();
    }
    if registry.len() >= MAX_HANDLES {
        if let Some(oldest) = registry.iter().min_by_key(|(_, h)| *lock(&h.last_used)).map(|(root, _)| root.clone()) {
            registry.remove(&oldest);
        }
    }
    let handle = Handle::new(root.to_path_buf());
    registry.insert(root.to_path_buf(), handle.clone());
    let building = handle.clone();
    thread::spawn(move || building.build());
    handle
}

/// The index for `root` when it is (or soon will be) usable.
pub fn ready(root: &Path, wait: Duration) -> Option<Arc<Handle>> {
    let handle = handle_for(root);
    handle.wait_ready(wait).then_some(handle)
}

fn existing(root: &Path) -> Option<Arc<Handle>> {
    lock(&REGISTRY).get(root).cloned()
}

/// Starts indexing a project in the background (cheap if already started).
pub fn start(root: &Path) {
    handle_for(root);
}

/// Tells the index (and the window) that the app changed these project-relative paths.
pub fn note_changed(root: &Path, rels: &[String]) {
    let rels: Vec<String> = rels.iter().filter(|p| !p.is_empty()).cloned().collect();
    if rels.is_empty() {
        return;
    }
    if let Some(handle) = existing(root) {
        handle.note_changed(&rels);
    }
    queue_change(rels, false);
}

/// Looks for changes after something else may have written files (a command the agent ran, a
/// connector tool) without blocking the caller.
pub fn refresh_async(root: &Path) {
    if let Some(handle) = existing(root) {
        thread::spawn(move || {
            handle.rescan(true);
        });
    }
}

// ----- one window event for a burst of changes -----

static PENDING_CHANGES: LazyLock<Mutex<(Vec<String>, Vec<String>)>> = LazyLock::new(Default::default);
static CHANGE_SCHEDULED: AtomicBool = AtomicBool::new(false);

/// Batches change notices: a 15-file scaffold sends one `workspace://changed` event, not fifteen.
/// `external` marks changes found by a rescan (made outside Neru), which the window does not
/// present as the agent's work.
fn queue_change(paths: Vec<String>, external: bool) {
    if paths.is_empty() {
        return;
    }
    {
        let mut pending = lock(&PENDING_CHANGES);
        for path in paths {
            if !pending.0.contains(&path) && pending.0.len() < 5_000 {
                pending.0.push(path.clone());
            }
            if external && !pending.1.contains(&path) && pending.1.len() < 5_000 {
                pending.1.push(path);
            }
        }
    }
    if CHANGE_SCHEDULED.swap(true, Ordering::AcqRel) {
        return;
    }
    thread::spawn(|| {
        thread::sleep(Duration::from_millis(70));
        CHANGE_SCHEDULED.store(false, Ordering::Release);
        let (paths, external) = std::mem::take(&mut *lock(&PENDING_CHANGES));
        if let Some(app) = APP.get() {
            let _ = app.emit("workspace://changed", json!({"paths": paths, "external": external}));
        }
    });
}

// ---------------------------------------------------------------------------------------------
// Search
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Query {
    pub text: String,
    pub regex: bool,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub glob: Option<String>,
    pub limit: usize,
}

impl Query {
    pub fn literal(text: &str, limit: usize) -> Self {
        Self { text: text.trim().to_string(), regex: false, case_sensitive: false, whole_word: false, glob: None, limit }
    }

    /// The regular expression the matcher is built from.
    pub fn pattern(&self) -> String {
        let base = if self.regex { self.text.clone() } else { regex::escape(&self.text) };
        if self.whole_word { format!(r"\b(?:{base})\b") } else { base }
    }

    /// Literals every match must contain, for the trigram filter.
    pub fn needles(&self) -> Option<Vec<Vec<u8>>> {
        if self.regex { needles_for(&self.pattern(), true, self.case_sensitive) } else { needles_for(&self.text, false, self.case_sensitive) }
    }

    pub fn matcher(&self) -> Result<Regex, String> {
        regex::RegexBuilder::new(&self.pattern())
            .case_insensitive(!self.case_sensitive)
            .size_limit(1 << 20)
            .build()
            .map_err(|e| format!("That is not a valid regular expression: {}", e.to_string().lines().last().unwrap_or("")))
    }
}

#[derive(Debug, Default)]
pub struct SearchOutput {
    pub hits: Vec<SearchHit>,
    pub files_scanned: usize,
    /// Files the trigram filter let through (the tests check the filter narrows the scan).
    #[allow(dead_code)]
    pub candidates: usize,
    pub total_files: usize,
    pub truncated: bool,
    pub ms: u64,
}

fn scan_one(root: &Path, path: &str, matcher: &Regex, limit: usize, out: &mut Vec<SearchHit>) {
    let Ok(bytes) = fs::read(root.join(path)) else { return };
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => std::borrow::Cow::Borrowed(text),
        Err(_) => String::from_utf8_lossy(&bytes),
    };
    line_hits(path, &text, matcher, out, limit);
}

/// Scans candidate files in path order, several at a time, stopping once `limit` hits are in.
fn scan_files(root: &Path, paths: &[String], matcher: &Regex, limit: usize) -> (Vec<SearchHit>, usize, bool) {
    let mut hits: Vec<SearchHit> = Vec::new();
    if paths.len() <= 6 {
        for (i, path) in paths.iter().enumerate() {
            scan_one(root, path, matcher, limit, &mut hits);
            if hits.len() >= limit {
                hits.truncate(limit);
                return (hits, i + 1, true);
            }
        }
        return (hits, paths.len(), false);
    }
    let threads = workers();
    let mut scanned = 0;
    for chunk in paths.chunks(threads * 8) {
        let cursor = AtomicUsize::new(0);
        let results: Mutex<Vec<(usize, Vec<SearchHit>)>> = Mutex::new(Vec::new());
        thread::scope(|scope| {
            for _ in 0..threads.min(chunk.len()) {
                scope.spawn(|| {
                    loop {
                        let i = cursor.fetch_add(1, Ordering::Relaxed);
                        if i >= chunk.len() {
                            break;
                        }
                        let mut local = Vec::new();
                        scan_one(root, &chunk[i], matcher, limit, &mut local);
                        if !local.is_empty() {
                            lock(&results).push((i, local));
                        }
                    }
                });
            }
        });
        scanned += chunk.len();
        let mut found = results.into_inner().unwrap_or_else(|p| p.into_inner());
        found.sort_by_key(|(i, _)| *i);
        for (_, list) in found {
            for hit in list {
                hits.push(hit);
                if hits.len() >= limit {
                    return (hits, scanned, true);
                }
            }
        }
    }
    (hits, scanned, false)
}

/// Searches `root`, using the index when it is ready and a plain walk otherwise.
pub fn search(root: &Path, query: &Query) -> Result<SearchOutput, String> {
    let matcher = query.matcher()?;
    match ready(root, Duration::from_secs(30)) {
        Some(handle) => Ok(handle.search(query, &matcher)),
        None => {
            let started = Instant::now();
            let hits = crate::workspace::search_project(root, &matcher, query.limit);
            Ok(SearchOutput { truncated: hits.len() >= query.limit, hits, ms: started.elapsed().as_millis() as u64, ..Default::default() })
        }
    }
}

/// Wildcard path match: `*` stays within a folder, `**` crosses folders. A pattern without a
/// slash is also tried against the file name alone; plain text matches names containing it.
pub fn path_matches(pattern: &str, path: &str) -> bool {
    fn walk(p: &[u8], s: &[u8]) -> bool {
        match p {
            [] => s.is_empty(),
            [b'*', b'*', b'/', rest @ ..] => walk(rest, s) || (0..s.len()).any(|i| s[i] == b'/' && walk(rest, &s[i + 1..])),
            [b'*', b'*', rest @ ..] => (0..=s.len()).any(|i| walk(rest, &s[i..])),
            [b'*', rest @ ..] => (0..=s.len()).take_while(|&i| i == 0 || s[i - 1] != b'/').any(|i| walk(rest, &s[i..])),
            [c, rest @ ..] => s.first() == Some(c) && walk(rest, &s[1..]),
        }
    }
    let pattern = pattern.replace('\\', "/").to_lowercase();
    let path = path.replace('\\', "/").to_lowercase();
    let name = path.rsplit('/').next().unwrap_or(&path);
    if !pattern.contains('*') && !pattern.contains('/') {
        return name.contains(&pattern);
    }
    walk(pattern.as_bytes(), path.as_bytes()) || (!pattern.contains('/') && walk(pattern.as_bytes(), name.as_bytes()))
}

// ---------------------------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------------------------

/// Where the index of the shown project stands, for the Explorer status line.
#[tauri::command]
pub fn index_status(state: State<'_, AppState>) -> Result<Status, String> {
    let root = project_root(&state)?;
    Ok(handle_for(&root).status())
}

/// Checks the disk for changes made outside Neru (the window calls this when it regains focus).
#[tauri::command]
pub async fn index_refresh(state: State<'_, AppState>) -> Result<Status, String> {
    let root = project_root(&state)?;
    tauri::async_runtime::spawn_blocking(move || {
        let handle = handle_for(&root);
        handle.rescan(true);
        handle.status()
    })
    .await
    .map_err(|e| e.to_string())
}

/// Fuzzy file finder over the index (the Explorer filter and the `@` picker).
#[tauri::command]
pub async fn search_files(query: String, limit: Option<usize>, state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let root = project_root(&state)?;
    let limit = limit.unwrap_or(60).clamp(1, 500);
    tauri::async_runtime::spawn_blocking(move || match ready(&root, Duration::from_secs(20)) {
        Some(handle) => handle.fuzzy(&query, limit),
        None => Vec::new(),
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_project(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("neru-index-{name}-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        root.canonicalize().unwrap()
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn sample() -> PathBuf {
        let root = temp_project("sample");
        write(&root, ".gitignore", "secret.txt\nbuild/\n");
        write(&root, "src/main.rs", "use std::io;\n\npub struct App {\n    name: String,\n}\n\nimpl App {\n    pub fn run(&self) {}\n}\n\npub fn start() {\n    println!(\"Hello Neru\");\n}\n");
        write(&root, "src/lib/util.ts", "export function slugify(input: string) {\n  return input.toLowerCase();\n}\n\nexport const Button = () => null\nexport interface Props { id: string }\nclass Cache {}\n");
        write(&root, "web/app.py", "class Server:\n    def handle(self):\n        pass\n\ndef main():\n    print('hello neru')\n");
        write(&root, "README.md", "# Neru\n\nSome text\n\n```\n# not a heading\n```\n\n## Usage\n");
        write(&root, "secret.txt", "hello neru secret\n");
        write(&root, "build/out.js", "hello neru built\n");
        write(&root, "node_modules/dep/index.js", "hello neru dependency\n");
        write(&root, "assets/logo.png", "\u{0}\u{0}PNG");
        root
    }

    fn paths(index: &Index) -> Vec<String> {
        index.records().map(|r| r.path.clone()).collect()
    }

    #[test]
    fn builds_and_respects_ignore_rules() {
        let root = sample();
        let index = build_index(&root);
        let all = paths(&index);
        assert!(all.contains(&"src/main.rs".to_string()));
        assert!(all.contains(&".gitignore".to_string()));
        assert!(all.contains(&"assets/logo.png".to_string()), "binary files are listed");
        assert!(!index.file("assets/logo.png").unwrap().text);
        assert!(!all.iter().any(|p| p.starts_with("node_modules") || p.starts_with("build/") || p == "secret.txt"), "{all:?}");
        assert_eq!(index.file("src/main.rs").unwrap().lang, "rust");
        assert_eq!(index.file("src/main.rs").unwrap().lines, 13);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn extracts_symbols_for_common_languages() {
        let root = sample();
        let index = build_index(&root);
        let names = |path: &str| index.file(path).unwrap().symbols.iter().map(|s| format!("{}:{}", s.kind, s.name)).collect::<Vec<_>>();
        assert_eq!(names("src/main.rs"), ["struct:App", "impl:App", "function:run", "function:start"]);
        let ts = names("src/lib/util.ts");
        assert!(ts.contains(&"function:slugify".to_string()) && ts.contains(&"function:Button".to_string()) && ts.contains(&"interface:Props".to_string()) && ts.contains(&"class:Cache".to_string()), "{ts:?}");
        let py = names("web/app.py");
        assert_eq!(py, ["class:Server", "method:handle", "function:main"]);
        assert_eq!(names("README.md"), ["heading:Neru", "heading:Usage"]);
        let run = index.file("src/main.rs").unwrap().symbols.iter().find(|s| s.name == "run").unwrap();
        assert_eq!((run.line, run.depth), (8, 1));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn search_uses_trigrams_and_matches_a_full_scan() {
        let root = sample();
        let handle = Handle::new(root.clone());
        handle.build();
        let query = Query::literal("hello neru", 100);
        let matcher = query.matcher().unwrap();
        let out = handle.search(&query, &matcher);
        let found: Vec<String> = out.hits.iter().map(|h| format!("{}:{}", h.path, h.line)).collect();
        assert_eq!(found, ["src/main.rs:12", "web/app.py:6"], "case-insensitive, ignored files skipped");
        assert_eq!(out.candidates, 2, "the trigram index narrows the scan to files that can match");
        // Same answer as a full walk.
        let slow = crate::workspace::search_project(&root, &matcher, 100);
        assert_eq!(slow.iter().map(|h| format!("{}:{}", h.path, h.line)).collect::<Vec<_>>(), found);
        // Regex, whole word and case options.
        let mut regex = Query::literal("fn (run|start)", 100);
        regex.regex = true;
        let out = handle.search(&regex, &regex.matcher().unwrap());
        assert_eq!(out.hits.len(), 2);
        assert!(out.candidates <= 1);
        let mut word = Query::literal("run", 100);
        word.whole_word = true;
        word.case_sensitive = true;
        assert_eq!(handle.search(&word, &word.matcher().unwrap()).hits.len(), 1);
        let mut two = Query::literal("io", 100);
        two.glob = Some("*.rs".into());
        assert_eq!(handle.search(&two, &two.matcher().unwrap()).hits.len(), 1, "short queries scan every text file");
        // Limits truncate.
        let limited = Query::literal("e", 3);
        let out = handle.search(&limited, &limited.matcher().unwrap());
        assert_eq!(out.hits.len(), 3);
        assert!(out.truncated);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn files_past_the_index_budget_are_still_searched() {
        let root = sample();
        let stats = scan(&root, None);
        // A spent budget: nothing gets trigram data.
        let spent = AtomicU64::new(INDEX_BUDGET_BYTES);
        let mut index = Index::from_records(analyze_all(&root, &stats, None, Some(&spent)));
        assert!(index.records().filter(|r| r.text).all(|r| !r.indexed));
        let needles = needles_for("hello neru", false, false);
        let ids = index.candidates(needles.as_deref());
        let found: Vec<&str> = ids.iter().filter_map(|&id| index.path_of(id)).collect();
        assert!(found.contains(&"src/main.rs") && found.contains(&"web/app.py"), "{found:?}");
        // Replacing a loose file with an indexed one moves it out of the always-scan set.
        index.upsert(analyze(&root, stats.iter().find(|s| s.path == "src/main.rs").unwrap(), None));
        let ids = index.candidates(needles.as_deref());
        assert!(ids.iter().filter_map(|&id| index.path_of(id)).any(|p| p == "src/main.rs"));
        index.remove("web/app.py");
        assert!(!index.candidates(needles.as_deref()).iter().filter_map(|&id| index.path_of(id)).any(|p| p == "web/app.py"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn needles_from_patterns() {
        assert_eq!(needles_for("Hello", false, false), Some(vec![b"hello".to_vec()]));
        assert_eq!(needles_for("hi", false, false), None);
        assert_eq!(needles_for("é-thing", false, false), None);
        let alt = needles_for("foo|barbaz", true, true).unwrap();
        assert!(alt.contains(&b"foo".to_vec()) && alt.contains(&b"barbaz".to_vec()));
        assert!(needles_for(r"\d+", true, false).is_none());
        assert!(needles_for("(", true, false).is_none());
    }

    #[test]
    fn incremental_updates_keep_the_index_correct() {
        let root = sample();
        let handle = Handle::new(root.clone());
        handle.build();
        let query = Query::literal("zebra_marker", 50);
        let matcher = query.matcher().unwrap();
        assert!(handle.search(&query, &matcher).hits.is_empty());

        write(&root, "src/new/zoo.ts", "export function zebra_marker() {}\n");
        handle.note_changed(&["src/new/zoo.ts".to_string()]);
        let out = handle.search(&query, &matcher);
        assert_eq!(out.hits.len(), 1);
        assert_eq!(out.candidates, 1);
        assert_eq!(handle.symbols("zebra", 5).len(), 1);

        // Editing replaces the old content in the index.
        write(&root, "src/new/zoo.ts", "export function giraffe() {}\n");
        handle.note_changed(&["src/new/zoo.ts".to_string()]);
        assert!(handle.search(&query, &matcher).hits.is_empty());
        assert_eq!(handle.symbols("giraffe", 5).len(), 1);

        // Deleting a folder removes everything under it.
        fs::remove_dir_all(root.join("src/new")).unwrap();
        handle.note_changed(&["src/new".to_string()]);
        assert!(handle.symbols("giraffe", 5).is_empty());
        assert!(handle.read().file("src/new/zoo.ts").is_none());

        // Renames: old path gone, new path present.
        fs::rename(root.join("web/app.py"), root.join("web/server.py")).unwrap();
        handle.note_changed(&["web/app.py".to_string(), "web/server.py".to_string()]);
        assert!(handle.read().file("web/app.py").is_none() && handle.read().file("web/server.py").is_some());

        // Ignored and skipped paths never enter the index.
        write(&root, "build/x.js", "zebra_marker\n");
        write(&root, "node_modules/x/y.js", "zebra_marker\n");
        handle.note_changed(&["build/x.js".to_string(), "node_modules/x/y.js".to_string()]);
        assert!(handle.search(&query, &matcher).hits.is_empty());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rescan_finds_changes_made_outside_the_app() {
        let root = sample();
        let handle = Handle::new(root.clone());
        handle.build();
        write(&root, "src/outside.rs", "pub fn from_editor() {}\n");
        fs::remove_file(root.join("README.md")).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        write(&root, "src/main.rs", "pub fn rewritten_elsewhere() {}\n");
        let changed = handle.rescan(false);
        assert!(changed.contains(&"src/outside.rs".to_string()) && changed.contains(&"README.md".to_string()) && changed.contains(&"src/main.rs".to_string()), "{changed:?}");
        assert!(handle.read().file("README.md").is_none());
        assert_eq!(handle.symbols("rewritten_elsewhere", 3).len(), 1);
        assert!(handle.symbols("App", 3).is_empty(), "old symbols of a rewritten file are gone");
        assert!(handle.rescan(false).is_empty(), "a second scan finds nothing new");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn project_map_fits_its_budget_and_degrades_gracefully() {
        let root = temp_project("map");
        for folder in 0..30 {
            for file in 0..6 {
                write(&root, &format!("pkg{folder}/mod{file}.ts"), &format!("export function feature{folder}_{file}() {{}}\nexport class Thing{file} {{}}\n"));
            }
        }
        write(&root, "package-lock.json", "{}\n");
        let index = build_index(&root);
        let full = index.render_map("", 100_000);
        assert!(full.contains("feature3_2") && full.contains("Thing5") && !full.contains("package-lock"));
        for budget in [400, 1_500, 4_000] {
            let map = index.render_map("", budget);
            assert!(map.len() <= budget + 200, "budget {budget} produced {} chars", map.len());
            assert!(!map.is_empty());
        }
        let small = index.render_map("", 1_500);
        assert!(small.contains("pkg0/") || small.contains("./"), "{small}");
        assert!(index.render_map("pkg2", 5_000).contains("feature2_1"));
        assert!(!index.render_map("pkg2", 5_000).contains("pkg3"));
        let outline = index.outline("pkg1/mod0.ts").unwrap();
        assert!(outline.contains("L1 function feature1_0") && outline.contains("L2 class Thing0"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn fuzzy_finder_prefers_file_names() {
        let root = sample();
        let index = build_index(&root);
        assert_eq!(index.fuzzy_files("main", 3)[0], "src/main.rs");
        assert_eq!(index.fuzzy_files("utl", 3)[0], "src/lib/util.ts");
        assert!(index.fuzzy_files("zzzz", 3).is_empty());
        assert!(path_matches("src/**/*.ts", "src/lib/util.ts"));
        let _ = fs::remove_dir_all(root);
    }

    /// Timing on the Neru app itself. Run with
    /// `cargo test index_benchmark -- --nocapture` to see the numbers.
    #[test]
    fn index_benchmark_on_the_neru_app() {
        // NERU_BENCH_ROOT points the same measurements at any other project (e.g. a big repository).
        let root = std::env::var_os("NERU_BENCH_ROOT").map(PathBuf::from).unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()).canonicalize().unwrap();
        let started = Instant::now();
        let handle = Handle::new(root.clone());
        handle.build();
        let build = started.elapsed();
        let status = handle.status();
        let mut report = format!("index: {} files, {} symbols, {} KB in {} ms", status.files, status.symbols, status.bytes / 1024, build.as_millis());
        assert!(status.files > 50, "expected the sources to be indexed");

        let absent = ["zzzz", "absent", "marker"].join("-");
        let queries: [(&str, bool); 4] = [("execute_read_tool", false), ("fn\\s+\\w+_tool", true), ("useState", false), (absent.as_str(), false)];
        for (text, regex) in queries {
            let mut query = Query::literal(text, 500);
            query.regex = regex;
            let matcher = query.matcher().unwrap();
            let t = Instant::now();
            let out = handle.search(&query, &matcher);
            let indexed = t.elapsed();
            let t = Instant::now();
            let slow = crate::workspace::search_project(&root, &matcher, 500);
            let walked = t.elapsed();
            assert_eq!(out.hits.len(), slow.len(), "index and full walk disagree for {text}");
            report.push_str(&format!("\n  search {text:?}: {} hits, {} of {} files read, {} us indexed vs {} us walking", out.hits.len(), out.candidates, out.total_files, indexed.as_micros(), walked.as_micros()));
        }
        let t = Instant::now();
        let changed = handle.rescan(false);
        report.push_str(&format!("\n  stat rescan: {} us ({} changed)", t.elapsed().as_micros(), changed.len()));
        let t = Instant::now();
        let map = handle.map("", 8_000);
        report.push_str(&format!("\n  project map: {} chars in {} us", map.len(), t.elapsed().as_micros()));
        let t = Instant::now();
        let fuzzy = handle.fuzzy("filetree", 5);
        report.push_str(&format!("\n  fuzzy file lookup: {:?} in {} us", fuzzy.first(), t.elapsed().as_micros()));
        eprintln!("{report}");
        assert!(build < Duration::from_secs(20));
    }
}
