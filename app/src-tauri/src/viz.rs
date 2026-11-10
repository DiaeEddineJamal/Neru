//! System-prompt guidance for drawing charts and interactive widgets inline in the conversation.
//! The window renders ```chart (native SVG) and ```html-widget (sandboxed iframe) fences; see
//! app/src/components/neru/Visual.tsx.

/// Guidance for models that can follow a strict output format.
pub const PROMPT: &str = r#"## Charts and interactive visuals
When a chart or small interactive demo would explain data or an idea better than prose or a table, you can draw it inline. Use at most a few per reply, and still state the key numbers in text.

Chart: a ```chart fence holding one JSON object. "type" is bar, line, area, pie or scatter. "labels" names the x categories (pie: the slices). "series" is a list of {"name", "data"}; data are plain numbers, one per label. Scatter data are [x, y] pairs. Optional: "title", "x" and "y" (axis titles), "stacked": true (bar, area). Up to 8 series. JSON only: no comments, no trailing commas.
```chart
{"type":"bar","title":"Sales","labels":["Q1","Q2"],"series":[{"name":"2025","data":[3,5]}],"y":"USD (k)"}
```

Widget: a ```html-widget fence holding a small self-contained HTML document (under 200 KB) with inline <style> and <script>. It runs sandboxed with no network: no external scripts, fonts, images or fetch; use inline SVG or data: URLs. Prefer addEventListener over inline onclick. CSS variables --text, --muted, --border, --surface and --accent follow the app theme.
```html-widget
<input type="range" id="r"><output id="o"></output>
<script>r.oninput=()=>o.textContent=r.value</script>
```

Diagram: a ```mermaid fence for flowcharts, sequence, class, state, ER, Gantt, mind maps, timelines and pie charts.

Files: for a Word, PDF, PowerPoint or Excel file the user will keep, use the create_document tool (format docx, pdf, pptx or xlsx); it opens in the document panel, draws its charts, and downloads as that file."#;

/// A short version for small and local models: charts and diagrams only, one example each, no
/// sandboxed widgets (they rarely write those well).
pub const LEAN_PROMPT: &str = r#"## Charts and diagrams
To show data as a chart, write a ```chart fence with one JSON object, like:
```chart
{"type":"bar","title":"Sales","labels":["Q1","Q2","Q3"],"series":[{"name":"2025","data":[3,5,4]}]}
```
"type" is bar, line, area, pie or scatter; "data" has one plain number per label. Valid JSON only.
To draw a flowchart or process, write a ```mermaid fence, like:
```mermaid
flowchart LR
  A[Start] --> B[Step] --> C[Done]
```
For a Word, PDF, PowerPoint or Excel file, call create_document with format docx, pdf, pptx or xlsx."#;

/// The visual guidance for a model: the full guide, or a short one with examples for small and
/// local models (the window shows a broken fence as code, so a miss costs little).
pub fn prompt_for(model_is_small: bool) -> &'static str {
    if model_is_small {
        LEAN_PROMPT
    } else {
        PROMPT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_models_get_the_short_guide() {
        assert_eq!(prompt_for(true), LEAN_PROMPT);
        assert_eq!(prompt_for(false), PROMPT);
        for fence in ["```chart", "```mermaid", "create_document"] {
            assert!(LEAN_PROMPT.contains(fence), "missing {fence}");
        }
        assert!(!LEAN_PROMPT.contains("html-widget"));
        assert!(LEAN_PROMPT.split_whitespace().count() <= 120);
    }

    #[test]
    fn prompt_names_every_fence_and_stays_short() {
        for fence in ["```chart", "```html-widget", "```mermaid"] {
            assert!(PROMPT.contains(fence), "missing {fence}");
        }
        let words = PROMPT.split_whitespace().count();
        assert!(words <= 320, "prompt is {words} words");
    }

    #[test]
    fn chart_example_is_valid_json() {
        let start = PROMPT.find("```chart\n").unwrap() + "```chart\n".len();
        let end = start + PROMPT[start..].find("\n```").unwrap();
        let spec: serde_json::Value = serde_json::from_str(&PROMPT[start..end]).unwrap();
        assert_eq!(spec["type"], "bar");
        assert_eq!(spec["series"][0]["data"].as_array().unwrap().len(), spec["labels"].as_array().unwrap().len());
    }
}
