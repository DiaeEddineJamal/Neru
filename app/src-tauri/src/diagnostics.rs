//! Edit-time checks: tree-sitter syntax errors for the languages Neru parses, plus the language
//! server's errors when one runs (lsp.rs). The agent sees them in the result of the edit that
//! caused them, so a broken file is fixed in the next step instead of at the next build.

use std::path::Path;

use serde_json::Value;
use tree_sitter::{Language, Node, Parser, Tree};

/// Most problems one check reports; the model fixes the first ones and checks again.
const MAX_REPORTED: usize = 8;

/// The tree-sitter grammar for a project path, when Neru has one.
pub fn grammar(path: &str) -> Option<Language> {
    let ext = path.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase())?;
    Some(match ext.as_str() {
        "ts" | "mts" | "cts" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX.into(),
        "js" | "jsx" | "mjs" | "cjs" => tree_sitter_javascript::LANGUAGE.into(),
        "py" | "pyw" => tree_sitter_python::LANGUAGE.into(),
        "rs" => tree_sitter_rust::LANGUAGE.into(),
        "go" => tree_sitter_go::LANGUAGE.into(),
        "json" => tree_sitter_json::LANGUAGE.into(),
        _ => return None,
    })
}

pub fn parse(path: &str, text: &str) -> Option<Tree> {
    let mut parser = Parser::new();
    parser.set_language(&grammar(path)?).ok()?;
    parser.parse(text, None)
}

/// One syntax problem: 1-based line and column, and what is wrong.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

/// Syntax errors in `text`, or None when the language has no grammar here.
pub fn syntax_errors(path: &str, text: &str) -> Option<Vec<Problem>> {
    let tree = parse(path, text)?;
    let mut found = Vec::new();
    collect(tree.root_node(), text, &mut found);
    Some(found)
}

fn collect(node: Node, text: &str, found: &mut Vec<Problem>) {
    if found.len() >= 50 || !node.has_error() && !node.is_missing() {
        return;
    }
    if node.is_missing() || node.is_error() {
        let at = node.start_position();
        let message = if node.is_missing() {
            format!("missing {}", node.kind())
        } else {
            let snippet: String = text.get(node.byte_range()).unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ").chars().take(40).collect();
            if snippet.is_empty() { "syntax error".into() } else { format!("unexpected `{snippet}`") }
        };
        found.push(Problem { line: at.row + 1, column: at.column + 1, message });
        // The children of an error node are what it swallowed; reporting them too is noise.
        if node.is_error() {
            return;
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect(child, text, found);
    }
}

/// What to tell the model after `relative` changed from `before` to `after`: syntax errors the
/// edit introduced and the language server's new errors. Empty when the file is fine, or was
/// already broken the same way before the edit. Starts with a newline, like hook notes.
pub fn after_edit(root: &Path, relative: &str, before: &str, after: &str) -> String {
    let mut notes = String::new();
    if let Some(now) = syntax_errors(relative, after) {
        let was = syntax_errors(relative, before).map_or(0, |errors| errors.len());
        if now.len() > was {
            notes.push_str(&format!("\nSyntax check: {relative} no longer parses ({} error{}). Fix before moving on:", now.len(), if now.len() == 1 { "" } else { "s" }));
            for problem in now.iter().take(MAX_REPORTED) {
                notes.push_str(&format!("\n  {relative}:{}:{} {}", problem.line, problem.column, problem.message));
            }
        }
    }
    let lsp = crate::lsp::after_edit(root, relative, before, after);
    if !lsp.is_empty() {
        notes.push_str(&format!("\nLanguage server: {} new error{} in {relative}:", lsp.len(), if lsp.len() == 1 { "" } else { "s" }));
        for diagnostic in lsp.iter().take(MAX_REPORTED) {
            notes.push_str(&format!("\n  {}", diagnostic.render(relative)));
        }
    }
    notes
}

/// The `diagnostics` tool: problems in one file (syntax and the language server's), or every
/// problem the language servers reported for the project so far.
pub fn tool(root: &Path, args: &Value) -> Result<String, String> {
    let path = args["path"].as_str().unwrap_or("").trim().replace('\\', "/");
    if path.is_empty() {
        let all = crate::lsp::project_diagnostics(root);
        if all.is_empty() {
            return Ok(if crate::lsp::any_running(root) {
                "No problems reported by the language servers for the files checked so far. Pass a path to check a specific file.".into()
            } else {
                "No language server is running for this project (none installed, or no file of its language was edited yet). Pass a path to check a file's syntax.".into()
            });
        }
        let mut out = String::new();
        for (file, list) in all.iter().take(30) {
            for diagnostic in list.iter().take(MAX_REPORTED) {
                out.push_str(&diagnostic.render(file));
                out.push('\n');
            }
        }
        return Ok(out.trim_end().to_string());
    }
    let full = crate::workspace::resolve_existing(root, &path)?;
    let text = crate::workspace::read_limited(&full)?;
    let mut out = String::new();
    match syntax_errors(&path, &text) {
        Some(errors) if errors.is_empty() => out.push_str("Syntax: no errors.\n"),
        Some(errors) => {
            out.push_str(&format!("Syntax: {} error{}\n", errors.len(), if errors.len() == 1 { "" } else { "s" }));
            for problem in errors.iter().take(20) {
                out.push_str(&format!("  {path}:{}:{} {}\n", problem.line, problem.column, problem.message));
            }
        }
        None => {}
    }
    match crate::lsp::check_file(root, &path, &text) {
        Some(list) if list.is_empty() => out.push_str("Language server: no problems.\n"),
        Some(list) => {
            out.push_str(&format!("Language server: {} problem{}\n", list.len(), if list.len() == 1 { "" } else { "s" }));
            for diagnostic in list.iter().take(30) {
                out.push_str(&format!("  {}\n", diagnostic.render(&path)));
            }
        }
        None if out.is_empty() => out.push_str("No checker for this file type (no grammar, and no language server found on PATH)."),
        None => out.push_str("Language server: not available for this file (install one on PATH for type errors)."),
    }
    Ok(out.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_syntax_errors_in_each_language() {
        let broken = [
            ("a.ts", "function f(x: number) {\n  return x +;\n}\n"),
            ("a.tsx", "const A = () => <div>{</div>;\n"),
            ("a.js", "if (x {\n}\n"),
            ("a.py", "def f(:\n    pass\n"),
            ("a.rs", "fn main() {\n    let x = ;\n}\n"),
            ("a.go", "package main\nfunc main( {\n}\n"),
            ("a.json", "{\"a\": 1,, }"),
        ];
        for (path, text) in broken {
            let errors = syntax_errors(path, text).unwrap();
            assert!(!errors.is_empty(), "{path} should have a syntax error");
        }
        let fine = [
            ("a.ts", "export function f(x: number): number {\n  return x + 1;\n}\n"),
            ("a.py", "def f(x):\n    return x\n"),
            ("a.rs", "fn main() {\n    let x = 1;\n}\n"),
            ("a.go", "package main\nfunc main() {}\n"),
            ("a.json", "{\"a\": [1, 2]}"),
        ];
        for (path, text) in fine {
            assert_eq!(syntax_errors(path, text).unwrap(), vec![], "{path} should parse");
        }
        assert!(syntax_errors("a.md", "# hi").is_none());
    }

    #[test]
    fn reports_the_line_and_only_errors_the_edit_introduced() {
        let errors = syntax_errors("a.rs", "fn a() {}\n\nfn b() {\n    let = 2;\n}\n").unwrap();
        assert_eq!(errors[0].line, 4);
        let root = std::env::temp_dir();
        let note = after_edit(&root, "x.py", "def f():\n    pass\n", "def f(:\n    pass\n");
        assert!(note.contains("Syntax check") && note.contains("x.py:1:"), "{note}");
        // Already broken the same way: nothing new to report.
        assert!(!after_edit(&root, "x.py", "def f(:\n  pass\n", "def f(:\n  pass\n\n").contains("Syntax check"));
    }
}
