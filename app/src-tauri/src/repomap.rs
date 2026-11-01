//! The ranked repo map (Aider's idea): real signatures from tree-sitter, a graph of which files
//! use the names other files define, and PageRank over it, personalized toward the files the
//! agent is working with. `project_map` shows the definitions that matter most for the task
//! within a small budget, instead of the alphabetical first N.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
};

use regex::Regex;
use tree_sitter::Node;

use crate::index::{FileRec, Symbol};

const MAX_SYMBOLS: usize = 300;
const SIGNATURE_CHARS: usize = 120;
/// Files bigger than this are left out of the reference graph.
const MAX_REF_BYTES: u64 = 400_000;

// ---------------------------------------------------------------------------------------------
// Signatures
// ---------------------------------------------------------------------------------------------

/// Definitions in a file, with signatures, for the languages with a grammar; None for others
/// (the index then falls back to its regex outline).
pub fn symbols(path: &str, text: &str) -> Option<Vec<Symbol>> {
    if path.ends_with(".json") {
        return None;
    }
    let tree = crate::diagnostics::parse(path, text)?;
    let mut out = Vec::new();
    walk(tree.root_node(), text, 0, &mut out);
    Some(out)
}

fn name_of(node: Node, field: &str, text: &str) -> Option<String> {
    let name = node.child_by_field_name(field)?;
    text.get(name.byte_range()).map(str::to_string)
}

/// The declaration's head: its text up to the body, on one line.
fn signature(node: Node, text: &str) -> String {
    let end = node.child_by_field_name("body").map_or(node.end_byte(), |body| body.start_byte());
    let head = text.get(node.start_byte()..end).unwrap_or("");
    let line: String = head.split_whitespace().collect::<Vec<_>>().join(" ");
    let line = line.trim_end_matches(['{', ':', '=', ' ']).to_string();
    if line.chars().count() > SIGNATURE_CHARS { format!("{}…", line.chars().take(SIGNATURE_CHARS).collect::<String>()) } else { line }
}

fn push(out: &mut Vec<Symbol>, node: Node, text: &str, name: String, kind: &'static str, depth: u8) {
    if out.len() < MAX_SYMBOLS && !name.is_empty() {
        out.push(Symbol { name, kind, line: node.start_position().row as u32 + 1, depth, signature: signature(node, text) });
    }
}

fn walk(node: Node, text: &str, depth: u8, out: &mut Vec<Symbol>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if out.len() >= MAX_SYMBOLS {
            return;
        }
        visit(child, text, depth, out);
    }
}

fn visit(node: Node, text: &str, depth: u8, out: &mut Vec<Symbol>) {
    let name = |field: &str| name_of(node, field, text).unwrap_or_default();
    let body = |out: &mut Vec<Symbol>| {
        if let Some(body) = node.child_by_field_name("body") {
            walk(body, text, depth + 1, out);
        }
    };
    match node.kind() {
        // Rust
        "function_item" | "function_signature_item" => push(out, node, text, name("name"), "function", depth),
        "struct_item" => push(out, node, text, name("name"), "struct", depth),
        "enum_item" => push(out, node, text, name("name"), "enum", depth),
        "union_item" => push(out, node, text, name("name"), "union", depth),
        "type_item" => push(out, node, text, name("name"), "type", depth),
        "const_item" | "static_item" => push(out, node, text, name("name"), "const", depth),
        "macro_definition" => push(out, node, text, name("name"), "macro", depth),
        "trait_item" => {
            push(out, node, text, name("name"), "trait", depth);
            body(out);
        }
        "mod_item" => {
            push(out, node, text, name("name"), "module", depth);
            body(out);
        }
        "impl_item" => {
            let target = node.child_by_field_name("type").and_then(|t| text.get(t.byte_range())).unwrap_or("");
            let base = target.split('<').next().unwrap_or(target).rsplit("::").next().unwrap_or(target).trim().to_string();
            push(out, node, text, base, "impl", depth);
            body(out);
        }
        // TypeScript / JavaScript
        // TypeScript / JavaScript (and Go, whose functions and methods share these kinds)
        "function_declaration" | "generator_function_declaration" | "method_declaration" => push(out, node, text, name("name"), "function", depth),
        "class_declaration" | "abstract_class_declaration" | "class" => {
            push(out, node, text, name("name"), "class", depth);
            body(out);
        }
        "interface_declaration" => push(out, node, text, name("name"), "interface", depth),
        "type_alias_declaration" => push(out, node, text, name("name"), "type", depth),
        "enum_declaration" => push(out, node, text, name("name"), "enum", depth),
        "internal_module" | "module" => {
            push(out, node, text, name("name"), "namespace", depth);
            body(out);
        }
        "method_definition" | "abstract_method_signature" => push(out, node, text, name("name"), "method", depth),
        "export_statement" | "ambient_declaration" => walk(node, text, depth, out),
        "lexical_declaration" | "variable_declaration" if depth == 0 => {
            let mut cursor = node.walk();
            for declarator in node.named_children(&mut cursor).filter(|n| n.kind() == "variable_declarator") {
                let value = declarator.child_by_field_name("value").map(|v| v.kind()).unwrap_or("");
                let kind = if matches!(value, "arrow_function" | "function_expression" | "function") { "function" } else { "const" };
                // Plain top-level variables only when exported, like the regex outline did.
                if kind == "function" || node.parent().is_some_and(|p| p.kind() == "export_statement") {
                    push(out, declarator, text, name_of(declarator, "name", text).unwrap_or_default(), kind, depth);
                }
            }
        }
        // Python
        "function_definition" => push(out, node, text, name("name"), if depth == 0 { "function" } else { "method" }, depth),
        "class_definition" => {
            push(out, node, text, name("name"), "class", depth);
            body(out);
        }
        "decorated_definition" => {
            if let Some(definition) = node.child_by_field_name("definition") {
                visit(definition, text, depth, out);
            }
        }
        // Go
        "type_declaration" => {
            let mut cursor = node.walk();
            for spec in node.named_children(&mut cursor).filter(|n| n.kind() == "type_spec") {
                let kind = match spec.child_by_field_name("type").map(|t| t.kind()) {
                    Some("struct_type") => "struct",
                    Some("interface_type") => "interface",
                    _ => "type",
                };
                push(out, spec, text, name_of(spec, "name", text).unwrap_or_default(), kind, depth);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------------------------
// References and ranking
// ---------------------------------------------------------------------------------------------

static IDENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[A-Za-z_$][A-Za-z0-9_$]{2,}").expect("valid identifier pattern"));

/// How many times each identifier occurs in `text`.
pub fn references(text: &str) -> HashMap<String, u32> {
    let mut counts: HashMap<String, u32> = HashMap::new();
    for found in IDENT.find_iter(text) {
        *counts.entry(found.as_str().to_string()).or_default() += 1;
    }
    counts
}

/// One file as the ranker sees it.
pub struct Node2 {
    pub path: String,
    pub defines: Vec<String>,
    pub refs: Arc<HashMap<String, u32>>,
}

/// PageRank over "file A uses a name file B defines" edges, personalized toward `focus_files`
/// (and names in `focus_names`), then spread onto the definitions along the edges that point at
/// them. Returns (file, name, rank), best first.
pub fn rank(nodes: &[Node2], focus_files: &HashSet<String>, focus_names: &HashSet<String>) -> Vec<(String, String, f64)> {
    let n = nodes.len();
    if n == 0 {
        return Vec::new();
    }
    let mut definers: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, node) in nodes.iter().enumerate() {
        for name in &node.defines {
            let list = definers.entry(name.as_str()).or_default();
            if !list.contains(&i) {
                list.push(i);
            }
        }
    }
    // edges[from] = (to, name, weight)
    let mut edges: Vec<Vec<(usize, &str, f64)>> = vec![Vec::new(); n];
    for (from, node) in nodes.iter().enumerate() {
        for (name, count) in node.refs.iter() {
            let Some(targets) = definers.get(name.as_str()) else { continue };
            let mut weight = (*count as f64).sqrt();
            if focus_names.contains(name) {
                weight *= 10.0;
            }
            let long = name.len() >= 8 && (name.contains('_') || name.chars().any(|c| c.is_ascii_uppercase()));
            if long {
                weight *= 3.0;
            }
            if name.starts_with('_') {
                weight *= 0.1;
            }
            // `text`, `value`, `push`: usually a local of the same name, not a use of the definition.
            if name.len() < 6 && name.chars().all(|c| c.is_ascii_lowercase()) {
                weight *= 0.2;
            }
            if targets.len() > 5 {
                weight *= 0.1;
            }
            for &to in targets {
                if to != from {
                    edges[from].push((to, name.as_str(), weight / targets.len() as f64));
                }
            }
        }
    }
    let mut personal = vec![1.0; n];
    for (i, node) in nodes.iter().enumerate() {
        if focus_files.contains(&node.path) {
            personal[i] = 100.0;
        }
    }
    let total: f64 = personal.iter().sum();
    personal.iter_mut().for_each(|p| *p /= total);
    let out_weight: Vec<f64> = edges.iter().map(|list| list.iter().map(|e| e.2).sum()).collect();
    let damping = 0.85;
    let mut rank = personal.clone();
    for _ in 0..40 {
        let mut next: Vec<f64> = personal.iter().map(|p| (1.0 - damping) * p).collect();
        let mut dangling = 0.0;
        for from in 0..n {
            if out_weight[from] == 0.0 {
                dangling += rank[from];
                continue;
            }
            for &(to, _, weight) in &edges[from] {
                next[to] += damping * rank[from] * weight / out_weight[from];
            }
        }
        for (i, value) in next.iter_mut().enumerate() {
            *value += damping * dangling * personal[i];
        }
        let delta: f64 = next.iter().zip(&rank).map(|(a, b)| (a - b).abs()).sum();
        rank = next;
        if delta < 1e-9 {
            break;
        }
    }
    let mut definitions: HashMap<(usize, &str), f64> = HashMap::new();
    for from in 0..n {
        for &(to, name, weight) in &edges[from] {
            *definitions.entry((to, name)).or_default() += rank[from] * weight / out_weight[from];
        }
    }
    // Files nothing points at still show their definitions, after everything referenced.
    for (i, node) in nodes.iter().enumerate() {
        for name in &node.defines {
            definitions.entry((i, name.as_str())).or_insert(rank[i] * 1e-3);
        }
    }
    let mut ranked: Vec<(String, String, f64)> = definitions.into_iter().map(|((file, name), score)| (nodes[file].path.clone(), name.to_string(), score)).collect();
    ranked.sort_by(|a, b| b.2.total_cmp(&a.2).then_with(|| a.0.cmp(&b.0)).then_with(|| a.1.cmp(&b.1)));
    ranked
}

// ---------------------------------------------------------------------------------------------
// Focus: the files the agent read or changed lately, per project
// ---------------------------------------------------------------------------------------------

static FOCUS: LazyLock<Mutex<HashMap<PathBuf, Vec<String>>>> = LazyLock::new(Default::default);
const FOCUS_FILES: usize = 24;

/// Records that the agent works with these files, so the map leans toward their neighbours.
pub fn note_focus(root: &Path, paths: &[&str]) {
    let mut focus = FOCUS.lock().unwrap_or_else(|p| p.into_inner());
    let list = focus.entry(root.to_path_buf()).or_default();
    for path in paths {
        let path = path.trim_matches('/').replace('\\', "/");
        if path.is_empty() {
            continue;
        }
        list.retain(|known| known != &path);
        list.push(path);
    }
    let excess = list.len().saturating_sub(FOCUS_FILES);
    list.drain(..excess);
}

pub fn focus(root: &Path) -> Vec<String> {
    FOCUS.lock().unwrap_or_else(|p| p.into_inner()).get(root).cloned().unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------------------------

/// Identifier counts per file, kept while the file's mtime stays the same.
static REFS: LazyLock<Mutex<HashMap<PathBuf, HashMap<String, (i64, Arc<HashMap<String, u32>>)>>>> = LazyLock::new(Default::default);

fn refs_for(root: &Path, record: &FileRec) -> Arc<HashMap<String, u32>> {
    if let Some((mtime, refs)) = REFS.lock().unwrap_or_else(|p| p.into_inner()).get(root).and_then(|files| files.get(&record.path)) {
        if *mtime == record.mtime {
            return refs.clone();
        }
    }
    let refs = Arc::new(std::fs::read(root.join(&record.path)).map(|bytes| references(&String::from_utf8_lossy(&bytes))).unwrap_or_default());
    REFS.lock().unwrap_or_else(|p| p.into_inner()).entry(root.to_path_buf()).or_default().insert(record.path.clone(), (record.mtime, refs.clone()));
    refs
}

/// Whether other files can use a definition: not an `impl` block (its type is the definition),
/// and not a Rust item without `pub`.
fn visible_outside(record: &FileRec, symbol: &Symbol) -> bool {
    symbol.kind != "impl" && !(record.lang == "rust" && !symbol.signature.is_empty() && !symbol.signature.starts_with("pub"))
}

/// The best-ranked definitions of the project, grouped by file, within `budget` characters.
/// `focus` adds files to lean toward on top of the ones the agent touched lately.
pub fn render(root: &Path, records: &[&FileRec], focus_extra: &[String], budget: usize) -> String {
    let sources: Vec<&FileRec> = records.iter().copied().filter(|r| r.text && !r.lang.is_empty() && r.size <= MAX_REF_BYTES && !matches!(r.lang, "markdown" | "json" | "yaml" | "toml" | "css" | "html")).collect();
    if sources.is_empty() {
        return String::new();
    }
    let nodes: Vec<Node2> = sources
        .iter()
        .take(4_000)
        .map(|r| Node2 { path: r.path.clone(), defines: r.symbols.iter().filter(|s| visible_outside(r, s)).map(|s| s.name.clone()).collect(), refs: refs_for(root, r) })
        .collect();
    let mut focus_files: HashSet<String> = focus(root).into_iter().collect();
    focus_files.extend(focus_extra.iter().map(|p| p.trim_matches('/').replace('\\', "/")));
    let ranked = rank(&nodes, &focus_files, &HashSet::new());
    let by_path: HashMap<&str, &FileRec> = sources.iter().map(|r| (r.path.as_str(), *r)).collect();
    // Files in order of their best definition, each with its best definitions in line order.
    let mut order: Vec<&str> = Vec::new();
    let mut chosen: HashMap<&str, Vec<&str>> = HashMap::new();
    for (path, name, _) in ranked.iter().take(400) {
        let entry = chosen.entry(path.as_str()).or_insert_with(|| {
            order.push(path.as_str());
            Vec::new()
        });
        if entry.len() < 12 && !entry.contains(&name.as_str()) {
            entry.push(name.as_str());
        }
    }
    let header = "Ranked definitions (most used by, or nearest to, the files in play):\n";
    let mut out = header.to_string();
    for path in order {
        let Some(record) = by_path.get(path) else { continue };
        let names = &chosen[path];
        let mut block = format!("{path}:\n");
        let mut symbols: Vec<&Symbol> = record.symbols.iter().filter(|s| names.contains(&s.name.as_str())).collect();
        symbols.sort_by_key(|s| s.line);
        symbols.dedup_by_key(|s| (s.name.clone(), s.line));
        for symbol in symbols.iter().take(12) {
            let shown = if symbol.signature.is_empty() { format!("{} {}", symbol.kind, symbol.name) } else { symbol.signature.clone() };
            block.push_str(&format!("{}L{} {shown}\n", if symbol.depth > 0 { "    " } else { "  " }, symbol.line));
        }
        if out.len() + block.len() > budget {
            if out.len() == header.len() {
                continue;
            }
            break;
        }
        out.push_str(&block);
    }
    if out.len() == header.len() { String::new() } else { out }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(path: &str, text: &str) -> Vec<String> {
        symbols(path, text).unwrap().iter().map(|s| format!("{}:{}:{}", s.kind, s.name, s.depth)).collect()
    }

    #[test]
    fn extracts_signatures() {
        let rust = "pub struct App { name: String }\nimpl App {\n    pub fn run(&self, x: u32) -> bool {\n        true\n    }\n}\npub fn start() {}\n";
        assert_eq!(kinds("a.rs", rust), ["struct:App:0", "impl:App:0", "function:run:1", "function:start:0"]);
        let run = symbols("a.rs", rust).unwrap().into_iter().find(|s| s.name == "run").unwrap();
        assert_eq!((run.line, run.signature.as_str()), (3, "pub fn run(&self, x: u32) -> bool"));
        let ts = "export function slugify(text: string): string {\n  return text\n}\nexport const Button = (props: Props) => null\ninterface Props { a: string }\nexport class Cache {\n  get(key: string) { return 1 }\n}\nconst local = 1\n";
        let found = kinds("a.ts", ts);
        for expected in ["function:slugify:0", "function:Button:0", "interface:Props:0", "class:Cache:0", "method:get:1"] {
            assert!(found.contains(&expected.to_string()), "{found:?}");
        }
        assert!(!found.iter().any(|k| k.contains("local")), "{found:?}");
        assert_eq!(kinds("a.py", "class Server:\n    def handle(self):\n        pass\n\n@cache\ndef main():\n    pass\n"), ["class:Server:0", "method:handle:1", "function:main:0"]);
        assert_eq!(kinds("a.go", "package main\ntype Server struct{}\nfunc (s *Server) Handle() error { return nil }\nfunc main() {}\n"), ["struct:Server:0", "function:Handle:0", "function:main:0"]);
    }

    fn node(path: &str, defines: &[&str], refs: &[(&str, u32)]) -> Node2 {
        Node2 { path: path.into(), defines: defines.iter().map(|s| s.to_string()).collect(), refs: Arc::new(refs.iter().map(|(k, v)| (k.to_string(), *v)).collect()) }
    }

    #[test]
    fn pagerank_prefers_widely_used_and_focused_definitions() {
        let nodes = vec![
            node("core.rs", &["shared_helper", "rarely"], &[]),
            node("a.rs", &["alpha_entry"], &[("shared_helper", 3)]),
            node("b.rs", &["beta_entry"], &[("shared_helper", 2), ("alpha_entry", 1)]),
            node("c.rs", &["gamma_entry"], &[("shared_helper", 1), ("delta_only", 4)]),
            node("d.rs", &["delta_only"], &[]),
        ];
        let ranked = rank(&nodes, &HashSet::new(), &HashSet::new());
        assert_eq!(ranked[0].1, "shared_helper");
        let position = |list: &[(String, String, f64)], name: &str| list.iter().position(|r| r.1 == name).unwrap();
        assert!(position(&ranked, "shared_helper") < position(&ranked, "rarely"));
        assert!(position(&ranked, "alpha_entry") < position(&ranked, "beta_entry"));
        // Focusing on c.rs lifts what c.rs uses above what the rest of the project uses.
        let focused = rank(&nodes, &HashSet::from(["c.rs".to_string()]), &HashSet::new());
        assert!(position(&focused, "delta_only") < position(&focused, "alpha_entry"), "{focused:?}");
        assert!(position(&focused, "delta_only") < position(&ranked, "delta_only") || position(&ranked, "delta_only") > 1);
    }

    #[test]
    fn focus_keeps_recent_files() {
        let root = Path::new("/tmp/neru-focus-test");
        note_focus(root, &["a.rs", "b.rs"]);
        note_focus(root, &["a.rs"]);
        assert_eq!(focus(root), ["b.rs", "a.rs"]);
    }
}

#[cfg(test)]
mod manual {
    /// `cargo test repomap::manual -- --ignored --nocapture` prints the ranked map of Neru itself.
    #[test]
    #[ignore]
    fn ranked_map_of_neru() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let index = crate::index::build_index(&root);
        let records: Vec<&crate::index::FileRec> = index.records().collect();
        let started = std::time::Instant::now();
        let map = super::render(&root, &records, &["semantic.rs".to_string()], 6_000);
        println!("{map}\n[{} chars in {:?}]", map.len(), started.elapsed());
        assert!(!map.is_empty());
    }
}
