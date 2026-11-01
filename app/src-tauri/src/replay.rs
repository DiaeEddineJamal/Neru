//! A session as something to share: a self-contained HTML replay that plays the conversation back step by
//! step (for code review or teaching), or a secret GitHub gist of the transcript, which gives a link. Secrets
//! are removed from both (privacy.rs), and images are left out.

use std::{fs, path::PathBuf};

use serde_json::json;
use tauri::State;

use crate::{AppState, Hidden, sessions, workspace::data_dir};

/// Transcript entries with secrets removed: (role, text, steps).
fn entries(state: &AppState, session_id: &str) -> Result<(String, String, Vec<(String, String, Vec<String>)>), String> {
    let shared = sessions::runtime(state, session_id)?;
    let runtime = sessions::lock(&shared)?;
    let clean = |text: &str| {
        let mut out = String::new();
        let mut at = 0;
        for (kind, range) in crate::privacy::find_secrets(text) {
            out.push_str(&text[at..range.start]);
            out.push_str(&format!("[secret removed: {kind}]"));
            at = range.end;
        }
        out.push_str(&text[at..]);
        out
    };
    let list = runtime
        .transcript
        .iter()
        .filter(|entry| !entry.content.trim().is_empty() || !entry.steps.is_empty())
        .map(|entry| (entry.role.clone(), clean(&entry.content), entry.steps.iter().map(|step| clean(step)).collect()))
        .collect();
    let project = std::path::Path::new(&runtime.summary.project_path).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_else(|| "Chat".into());
    Ok((runtime.summary.title.clone(), project, list))
}

fn file_name(title: &str) -> String {
    let slug: String = title.chars().map(|c| if c.is_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    let slug = slug.split('-').filter(|part| !part.is_empty()).take(8).collect::<Vec<_>>().join("-");
    if slug.is_empty() { "session".into() } else { slug }
}

/// The transcript as Markdown, for a gist.
pub fn markdown(title: &str, project: &str, list: &[(String, String, Vec<String>)]) -> String {
    let mut out = format!("# {title}\n\n*A Neru session in {project}. Secrets were removed before sharing.*\n");
    for (role, text, steps) in list {
        match role.as_str() {
            "user" => out.push_str(&format!("\n---\n\n### You\n\n{text}\n")),
            "note" => out.push_str(&format!("\n> {}\n", text.replace('\n', " "))),
            _ => {
                out.push_str("\n### Neru\n\n");
                if !steps.is_empty() {
                    out.push_str("<details><summary>");
                    out.push_str(&format!("{} step{}", steps.len(), if steps.len() == 1 { "" } else { "s" }));
                    out.push_str("</summary>\n\n");
                    for step in steps {
                        out.push_str(&format!("- {}\n", step.replace('\n', " ")));
                    }
                    out.push_str("\n</details>\n\n");
                }
                out.push_str(&format!("{text}\n"));
            }
        }
    }
    out
}

/// The replay page: the transcript as JSON and a small player that shows it one step at a time.
pub fn html(title: &str, project: &str, list: &[(String, String, Vec<String>)]) -> String {
    let data = json!({
        "title": title,
        "project": project,
        "entries": list.iter().map(|(role, text, steps)| json!({"role": role, "text": text, "steps": steps})).collect::<Vec<_>>(),
    });
    // `</` inside a script would end it early.
    let data = data.to_string().replace("</", "<\\/");
    let escape = |text: &str| text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    REPLAY_TEMPLATE.replace("{{TITLE}}", &escape(title)).replace("{{DATA}}", &data)
}

const REPLAY_TEMPLATE: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{{TITLE}} · Neru replay</title>
<style>
:root { --bg:#f5f4ef; --surface:#ecebe4; --text:#1f2420; --muted:#6b726c; --moss:#54745b; --border:#d9d8cf; --code:#e6e5dd; }
@media (prefers-color-scheme: dark) { :root { --bg:#151515; --surface:#1e1f1d; --text:#ecebe4; --muted:#9aa19b; --moss:#8ea291; --border:#2c2e2b; --code:#232522; } }
* { box-sizing: border-box; }
body { margin:0; background:var(--bg); color:var(--text); font:16px/1.55 system-ui, -apple-system, "Segoe UI", sans-serif; }
header { position:sticky; top:0; background:var(--bg); border-bottom:1px solid var(--border); padding:14px 16px; display:flex; flex-wrap:wrap; gap:10px; align-items:center; z-index:1; }
header h1 { font:600 17px/1.3 inherit; margin:0; flex:1 1 220px; }
header small { display:block; color:var(--muted); font-weight:400; font-size:13px; }
button, select { font:inherit; font-size:14px; border-radius:10px; border:1px solid var(--border); background:var(--surface); color:var(--text); padding:7px 12px; cursor:pointer; }
button.primary { background:var(--moss); color:#fff; border-color:transparent; }
input[type=range] { flex:1 1 160px; accent-color:var(--moss); }
main { max-width:780px; margin:0 auto; padding:20px 16px 80px; }
.msg { margin:14px 0; animation:in .25s ease-out; }
@keyframes in { from { opacity:0; transform:translateY(4px); } }
@media (prefers-reduced-motion: reduce) { .msg { animation:none; } }
.user { margin-left:auto; max-width:85%; width:fit-content; background:var(--surface); border-radius:16px; padding:10px 14px; white-space:pre-wrap; }
.note { color:var(--muted); font-size:14px; text-align:center; }
.steps { color:var(--muted); font-size:14px; margin:0 0 6px; padding-left:18px; }
.text { white-space:pre-wrap; overflow-wrap:anywhere; }
pre { background:var(--code); border-radius:10px; padding:10px 12px; overflow-x:auto; white-space:pre; font:13px/1.5 ui-monospace, "JetBrains Mono", Consolas, monospace; }
footer { color:var(--muted); font-size:13px; text-align:center; padding:24px; }
</style>
</head>
<body>
<header>
  <h1 id="title"></h1>
  <button class="primary" id="play" aria-label="Play">Play</button>
  <button id="next" aria-label="Next step">Next</button>
  <select id="speed" aria-label="Speed"><option value="2200">Slow</option><option value="1200" selected>Normal</option><option value="500">Fast</option></select>
  <input type="range" id="seek" min="0" value="0" aria-label="Position">
</header>
<main id="log" aria-live="polite"></main>
<footer>Made with Neru · secrets were removed before sharing</footer>
<script id="data" type="application/json">{{DATA}}</script>
<script>
const data = JSON.parse(document.getElementById('data').textContent);
const log = document.getElementById('log'), seek = document.getElementById('seek'), play = document.getElementById('play');
const t = document.getElementById('title'); t.textContent = data.title; const sub = document.createElement('small'); sub.textContent = data.project + ' · ' + data.entries.length + ' steps'; t.appendChild(sub);
seek.max = data.entries.length;
let shown = 0, timer = null;
function block(text) {
  const out = document.createElement('div'); out.className = 'text';
  text.split(/```[a-zA-Z0-9_-]*\n?/).forEach((part, i) => {
    if (i % 2) { const pre = document.createElement('pre'); pre.textContent = part.replace(/\n$/, ''); out.appendChild(pre); }
    else if (part) out.appendChild(document.createTextNode(part));
  });
  return out;
}
function render(entry) {
  const el = document.createElement('div'); el.className = 'msg ' + (entry.role === 'user' ? 'user' : entry.role === 'note' ? 'note' : 'assistant');
  if (entry.role === 'user' || entry.role === 'note') el.textContent = entry.text;
  else {
    if (entry.steps.length) { const ul = document.createElement('ul'); ul.className = 'steps'; entry.steps.forEach(s => { const li = document.createElement('li'); li.textContent = s; ul.appendChild(li); }); el.appendChild(ul); }
    el.appendChild(block(entry.text));
  }
  return el;
}
function show(n) {
  n = Math.max(0, Math.min(n, data.entries.length));
  if (n < shown) { log.innerHTML = ''; shown = 0; }
  while (shown < n) log.appendChild(render(data.entries[shown++]));
  seek.value = shown;
  if (shown && shown === n) log.lastChild?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  if (shown >= data.entries.length) stop();
}
function stop() { clearInterval(timer); timer = null; play.textContent = shown >= data.entries.length ? 'Replay' : 'Play'; }
play.onclick = () => {
  if (timer) return stop();
  if (shown >= data.entries.length) show(0);
  play.textContent = 'Pause';
  show(shown + 1);
  timer = setInterval(() => show(shown + 1), Number(document.getElementById('speed').value));
};
document.getElementById('next').onclick = () => { stop(); show(shown + 1); };
seek.oninput = () => { stop(); show(Number(seek.value)); };
show(1);
</script>
</body>
</html>
"#;

/// Writes the replay page next to Neru's data and returns its path.
#[tauri::command]
pub fn export_replay(session_id: String, state: State<'_, AppState>) -> Result<String, String> {
    let (title, project, list) = entries(&state, &session_id)?;
    let dir: PathBuf = data_dir()?.join("replays");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}-{}.html", file_name(&title), &session_id[..8.min(session_id.len())]));
    fs::write(&path, html(&title, &project, &list)).map_err(|e| e.to_string())?;
    Ok(crate::workspace::shown(&path))
}

/// Publishes the transcript as a secret gist with the GitHub CLI and returns its link. Anyone with the link
/// can read it, so the window asks first.
#[tauri::command]
pub async fn share_replay_gist(session_id: String, state: State<'_, AppState>) -> Result<String, String> {
    let (title, project, list) = entries(&state, &session_id)?;
    let dir = data_dir()?.join("replays");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.md", file_name(&title)));
    fs::write(&path, markdown(&title, &project, &list)).map_err(|e| e.to_string())?;
    let output = tokio::process::Command::new("gh")
        .hidden()
        .args(["gist", "create", "--desc", &format!("Neru session: {title}")])
        .arg(&path)
        .env("GH_PROMPT_DISABLED", "1")
        .output()
        .await
        .map_err(|_| "Install the GitHub CLI (gh) and run `gh auth login` to share a link".to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    String::from_utf8_lossy(&output.stdout).lines().map(str::trim).find(|line| line.starts_with("https://")).map(str::to_string).ok_or_else(|| "GitHub did not return a link".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_keep_the_story_and_drop_script_breakouts() {
        let list = vec![
            ("user".to_string(), "Fix the </script> bug".to_string(), vec![]),
            ("assistant".to_string(), "Done.\n```ts\nconst a = 1\n```".to_string(), vec!["Edited src/a.ts".to_string()]),
        ];
        let page = html("Fix <b>", "app", &list);
        assert!(page.contains("<title>Fix &lt;b&gt; · Neru replay</title>"));
        assert!(!page.contains("Fix the </script>"), "a closing tag in the data would end the script");
        let md = markdown("Fix", "app", &list);
        assert!(md.contains("### You\n\nFix the </script> bug") && md.contains("- Edited src/a.ts"));
        assert_eq!(file_name("Add rate limiting!! to API"), "add-rate-limiting-to-api");
    }
}
