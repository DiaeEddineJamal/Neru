//! Jupyter notebooks (`.ipynb`) for the agent: `read_notebook` lists the cells with their outputs,
//! and `propose_notebook_edit` replaces, inserts or deletes one cell by index. The edit becomes an
//! ordinary write proposal of the whole file, so the user reviews the diff like any other change.
//! The file stays a valid notebook: Jupyter's layout (one-space indent, sorted keys, source as a
//! list of lines), cell ids on nbformat 4.5+, and a changed code cell loses its stale outputs.

use serde_json::{Map, Value, json};

const OUTPUT_CHARS: usize = 2_000;
const READ_CHARS: usize = 40_000;

fn parse(text: &str) -> Result<Value, String> {
    let notebook: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("Not a valid notebook: {e}"))?;
    if !notebook["cells"].is_array() {
        return Err("Not a valid notebook: it has no cells list".into());
    }
    Ok(notebook)
}

/// A cell's `source`, which Jupyter stores as one string or a list of lines.
fn joined(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(lines) => lines.iter().filter_map(Value::as_str).collect(),
        _ => String::new(),
    }
}

/// Text as Jupyter writes it: lines that keep their `\n`, the last one without.
fn split_lines(text: &str) -> Value {
    Value::Array(text.split_inclusive('\n').map(|line| json!(line)).collect())
}

fn language(notebook: &Value) -> String {
    notebook["metadata"]["kernelspec"]["language"]
        .as_str()
        .or_else(|| notebook["metadata"]["language_info"]["name"].as_str())
        .unwrap_or("python")
        .to_string()
}

fn output_text(output: &Value) -> String {
    let text = match output["output_type"].as_str() {
        Some("stream") => joined(&output["text"]),
        Some("error") => format!("{}: {}", output["ename"].as_str().unwrap_or("Error"), output["evalue"].as_str().unwrap_or("")),
        Some("execute_result" | "display_data") => {
            let data = &output["data"];
            if data.get("text/plain").is_some() {
                joined(&data["text/plain"])
            } else {
                let kinds: Vec<&str> = data.as_object().map(|map| map.keys().map(String::as_str).collect()).unwrap_or_default();
                format!("[{}]", kinds.join(", "))
            }
        }
        _ => String::new(),
    };
    let mut text: String = text.trim_end().to_string();
    if text.chars().count() > OUTPUT_CHARS {
        text = text.chars().take(OUTPUT_CHARS).collect::<String>() + "\n[output truncated]";
    }
    text
}

/// The notebook as the model reads it.
pub(crate) fn describe(text: &str) -> Result<String, String> {
    let notebook = parse(text)?;
    let cells = notebook["cells"].as_array().cloned().unwrap_or_default();
    let fence = language(&notebook);
    let mut out = format!("[Notebook with {} cells, {fence}]\n", cells.len());
    for (index, cell) in cells.iter().enumerate() {
        let kind = cell["cell_type"].as_str().unwrap_or("code");
        let source = joined(&cell["source"]);
        let count = cell["execution_count"].as_u64().map(|n| format!(", run {n}")).unwrap_or_default();
        out.push_str(&format!("\n## Cell {index} [{kind}{count}]\n"));
        if kind == "code" {
            out.push_str(&format!("```{fence}\n{source}\n```\n"));
        } else {
            out.push_str(&format!("{source}\n"));
        }
        let outputs: Vec<String> = cell["outputs"].as_array().into_iter().flatten().map(output_text).filter(|text| !text.is_empty()).collect();
        if !outputs.is_empty() {
            out.push_str(&format!("Output:\n```\n{}\n```\n", outputs.join("\n")));
        }
        if out.len() > READ_CHARS {
            out.push_str(&format!("\n[Stopped after cell {index} of {}; the rest is too long to show.]", cells.len()));
            break;
        }
    }
    Ok(out)
}

fn new_cell(notebook: &Value, kind: &str, source: &str, seed: usize) -> Value {
    let mut cell = Map::new();
    cell.insert("cell_type".into(), json!(kind));
    cell.insert("metadata".into(), json!({}));
    cell.insert("source".into(), split_lines(source));
    if kind == "code" {
        cell.insert("execution_count".into(), Value::Null);
        cell.insert("outputs".into(), json!([]));
    }
    let versioned = notebook["nbformat"].as_u64().unwrap_or(4) > 4 || notebook["nbformat_minor"].as_u64().unwrap_or(0) >= 5;
    if versioned {
        cell.insert("id".into(), json!(cell_id(notebook, seed)));
    }
    Value::Object(cell)
}

/// A short id no other cell uses, as nbformat 4.5 asks for.
fn cell_id(notebook: &Value, seed: usize) -> String {
    let taken: Vec<&str> = notebook["cells"].as_array().into_iter().flatten().filter_map(|cell| cell["id"].as_str()).collect();
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0) as usize;
    (0..).map(|n| format!("{:08x}", (nanos ^ (seed.wrapping_mul(2_654_435_761))).wrapping_add(n) as u32)).find(|id| !taken.contains(&id.as_str())).unwrap_or_default()
}

/// Jupyter's own layout, so the diff shows only the cell that changed.
fn render(notebook: &Value) -> Result<String, String> {
    let mut out = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, formatter);
    serde::Serialize::serialize(notebook, &mut serializer).map_err(|e| e.to_string())?;
    let mut text = String::from_utf8(out).map_err(|e| e.to_string())?;
    text.push('\n');
    Ok(text)
}

/// The notebook after one cell edit, from `propose_notebook_edit`'s arguments.
pub(crate) fn apply(text: &str, args: &Value) -> Result<String, String> {
    let mut notebook = parse(text)?;
    let index = args["cell_index"].as_u64().ok_or("cell_index is required")? as usize;
    let mode = args["edit_mode"].as_str().unwrap_or("replace");
    let kind = args["cell_type"].as_str();
    if kind.is_some_and(|kind| !matches!(kind, "code" | "markdown" | "raw")) {
        return Err("cell_type must be code, markdown or raw".into());
    }
    let source = args["source"].as_str();
    let count = notebook["cells"].as_array().map_or(0, Vec::len);
    match mode {
        "insert" => {
            if index > count {
                return Err(format!("cell_index {index} is past the end; the notebook has {count} cells"));
            }
            let cell = new_cell(&notebook, kind.unwrap_or("code"), source.ok_or("source is required to insert a cell")?, index);
            notebook["cells"].as_array_mut().ok_or("No cells")?.insert(index, cell);
        }
        "delete" => {
            if index >= count {
                return Err(format!("There is no cell {index}; the notebook has {count} cells"));
            }
            notebook["cells"].as_array_mut().ok_or("No cells")?.remove(index);
        }
        "replace" => {
            if index >= count {
                return Err(format!("There is no cell {index}; the notebook has {count} cells. Use edit_mode insert to add one."));
            }
            let source = source.ok_or("source is required to replace a cell")?;
            let current = notebook["cells"][index].clone();
            let kind = kind.unwrap_or_else(|| current["cell_type"].as_str().unwrap_or("code")).to_string();
            let mut cell = current.as_object().cloned().unwrap_or_default();
            cell.insert("cell_type".into(), json!(kind));
            cell.insert("source".into(), split_lines(source));
            cell.entry("metadata").or_insert_with(|| json!({}));
            if kind == "code" {
                // The old outputs belong to the old code.
                cell.insert("outputs".into(), json!([]));
                cell.insert("execution_count".into(), Value::Null);
            } else {
                cell.remove("outputs");
                cell.remove("execution_count");
            }
            notebook["cells"][index] = Value::Object(cell);
        }
        other => return Err(format!("Unknown edit_mode {other}; use replace, insert or delete")),
    }
    render(&notebook)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOTEBOOK: &str = r##"{
 "cells": [
  {"cell_type": "markdown", "id": "a1", "metadata": {}, "source": ["# Title\n", "Intro"]},
  {"cell_type": "code", "execution_count": 2, "id": "b2", "metadata": {}, "outputs": [{"name": "stdout", "output_type": "stream", "text": ["3\n"]}, {"data": {"image/png": "AAAA"}, "metadata": {}, "output_type": "display_data"}], "source": ["x = 1\n", "print(x + 2)"]}
 ],
 "metadata": {"kernelspec": {"language": "python", "name": "python3"}},
 "nbformat": 4,
 "nbformat_minor": 5
}"##;

    #[test]
    fn describes_cells_with_outputs() {
        let text = describe(NOTEBOOK).unwrap();
        assert!(text.starts_with("[Notebook with 2 cells, python]"));
        assert!(text.contains("## Cell 0 [markdown]\n# Title\nIntro"));
        assert!(text.contains("## Cell 1 [code, run 2]\n```python\nx = 1\nprint(x + 2)\n```"));
        assert!(text.contains("Output:\n```\n3\n[image/png]\n```"));
        assert!(describe("{\"nope\": 1}").is_err());
    }

    #[test]
    fn replacing_code_clears_stale_outputs_and_keeps_the_rest() {
        let out = apply(NOTEBOOK, &json!({"cell_index": 1, "source": "y = 5\nprint(y)\n"})).unwrap();
        let notebook: Value = serde_json::from_str(&out).unwrap();
        let cell = &notebook["cells"][1];
        assert_eq!(cell["source"], json!(["y = 5\n", "print(y)\n"]));
        assert_eq!(cell["outputs"], json!([]));
        assert!(cell["execution_count"].is_null());
        assert_eq!(cell["id"], "b2");
        assert_eq!(notebook["cells"][0]["source"], json!(["# Title\n", "Intro"]));
        assert_eq!(notebook["nbformat_minor"], 5);
        // Jupyter's layout: one-space indent and a final newline.
        assert!(out.starts_with("{\n \"cells\": [") && out.ends_with("}\n"));
    }

    #[test]
    fn switching_a_cell_to_markdown_drops_code_fields() {
        let out = apply(NOTEBOOK, &json!({"cell_index": 1, "cell_type": "markdown", "source": "Notes"})).unwrap();
        let cell = &serde_json::from_str::<Value>(&out).unwrap()["cells"][1];
        assert_eq!(cell["cell_type"], "markdown");
        assert!(cell.get("outputs").is_none() && cell.get("execution_count").is_none());
    }

    #[test]
    fn inserts_and_deletes_by_index() {
        let out = apply(NOTEBOOK, &json!({"cell_index": 2, "edit_mode": "insert", "source": "z = 3"})).unwrap();
        let notebook: Value = serde_json::from_str(&out).unwrap();
        let added = &notebook["cells"][2];
        assert_eq!(added["cell_type"], "code");
        assert_eq!(added["outputs"], json!([]));
        assert!(added["id"].as_str().is_some_and(|id| !id.is_empty() && id != "a1" && id != "b2"));
        let out = apply(&out, &json!({"cell_index": 0, "edit_mode": "delete"})).unwrap();
        let notebook: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(notebook["cells"].as_array().unwrap().len(), 2);
        assert_eq!(notebook["cells"][0]["id"], "b2");
    }

    #[test]
    fn refuses_bad_indexes_and_modes() {
        assert!(apply(NOTEBOOK, &json!({"cell_index": 2, "source": "x"})).is_err());
        assert!(apply(NOTEBOOK, &json!({"cell_index": 5, "edit_mode": "insert", "source": "x"})).is_err());
        assert!(apply(NOTEBOOK, &json!({"cell_index": 2, "edit_mode": "delete"})).is_err());
        assert!(apply(NOTEBOOK, &json!({"cell_index": 0, "edit_mode": "move"})).is_err());
        assert!(apply(NOTEBOOK, &json!({"cell_index": 0, "cell_type": "sql", "source": "x"})).is_err());
    }

    #[test]
    fn old_notebooks_get_no_cell_ids() {
        let old = r#"{"cells": [], "metadata": {}, "nbformat": 4, "nbformat_minor": 2}"#;
        let out = apply(old, &json!({"cell_index": 0, "edit_mode": "insert", "cell_type": "markdown", "source": "Hi"})).unwrap();
        let cell = &serde_json::from_str::<Value>(&out).unwrap()["cells"][0];
        assert!(cell.get("id").is_none());
        assert_eq!(cell["source"], json!(["Hi"]));
    }
}
