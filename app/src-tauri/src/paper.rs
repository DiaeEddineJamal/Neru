//! Academic papers for Chat's Research mode. After research.rs gathers numbered sources, the selected
//! model outlines a paper, writes it one section at a time (so each request stays within a free
//! model's output limit), and writes the abstract last. Citations become in-text citations in the
//! student's style with a reference list of the works cited. The paper is saved with the session and
//! opened in the window's document panel (ArtifactPane.tsx), which previews it and exports Word, PDF,
//! HTML, LaTeX and Markdown.

use std::{collections::BTreeSet, fs, path::PathBuf, time::Duration};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::AppHandle;

use crate::{
    ProviderConfig,
    citation,
    research::{Brief, Deliverable, Progress, ask_model},
    web::Source,
    workspace::data_dir,
};

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Section {
    pub heading: String,
    /// Markdown: paragraphs, ### subheadings, lists and tables. Citations already in the paper's style.
    pub body: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Reference {
    /// The source's number in the research notes.
    pub number: usize,
    /// The formatted entry (Markdown italics).
    pub text: String,
    pub source: Source,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Paper {
    pub id: String,
    pub title: String,
    /// "Literature review", "Research paper"…
    pub kind: String,
    pub authors: Vec<String>,
    pub affiliation: String,
    /// "1 November 2026".
    pub date: String,
    pub abstract_text: String,
    pub keywords: Vec<String>,
    pub sections: Vec<Section>,
    pub references: Vec<Reference>,
    /// apa, mla, chicago, ieee or harvard.
    pub style: String,
    /// The file the student asked for, if any: docx, pdf, md, html or tex.
    pub format: String,
    pub language: String,
    pub question: String,
    pub words: usize,
    pub created: u64,
}

/// What the chat shows for a paper: the card that opens it.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Card {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub words: usize,
    pub sources: usize,
    pub format: String,
}

impl Paper {
    pub fn card(&self) -> Card {
        Card { id: self.id.clone(), title: self.title.clone(), kind: self.kind.clone(), words: self.words, sources: self.references.len(), format: self.format.clone() }
    }

    /// The whole paper as Markdown, for the conversation history and the Markdown export.
    pub fn markdown(&self) -> String {
        let mut text = format!("# {}\n\n", self.title);
        if !self.authors.is_empty() {
            text.push_str(&format!("{}\n\n", self.authors.join(", ")));
        }
        text.push_str(&format!("## Abstract\n\n{}\n\n", self.abstract_text.trim()));
        if !self.keywords.is_empty() {
            text.push_str(&format!("*Keywords:* {}\n\n", self.keywords.join(", ")));
        }
        for section in &self.sections {
            text.push_str(&format!("## {}\n\n{}\n\n", section.heading, section.body.trim()));
        }
        if !self.references.is_empty() {
            text.push_str("## References\n\n");
            for reference in &self.references {
                text.push_str(&format!("{}\n\n", reference.text));
            }
        }
        text.trim_end().to_string()
    }
}

// ---------- storage ----------

fn folder(session: &str) -> Result<PathBuf, String> {
    uuid::Uuid::parse_str(session).map_err(|_| "Invalid session id")?;
    Ok(data_dir()?.join("sessions").join(format!("{session}.papers")))
}

fn safe_id(id: &str) -> Result<&str, String> {
    if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err("Invalid paper id".into());
    }
    Ok(id)
}

pub fn save(session: &str, paper: &Paper) -> Result<(), String> {
    let dir = folder(session)?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.json", safe_id(&paper.id)?));
    fs::write(path, serde_json::to_vec_pretty(paper).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// Every paper written in this session, newest first: the document panel's versions.
#[tauri::command]
pub fn list_session_papers(session_id: String) -> Result<Vec<Card>, String> {
    let Ok(entries) = fs::read_dir(folder(&session_id)?) else { return Ok(Vec::new()) };
    let mut papers: Vec<Paper> = entries
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|entry| fs::read(entry.path()).ok())
        .filter_map(|bytes| serde_json::from_slice(&bytes).ok())
        .collect();
    papers.sort_by_key(|paper| std::cmp::Reverse(paper.created));
    Ok(papers.iter().map(Paper::card).collect())
}

#[tauri::command]
pub fn read_session_paper(session_id: String, id: String) -> Result<Paper, String> {
    let path = folder(&session_id)?.join(format!("{}.json", safe_id(&id)?));
    serde_json::from_slice(&fs::read(path).map_err(|_| "This paper is no longer saved".to_string())?).map_err(|e| e.to_string())
}

// ---------- writing ----------

#[derive(Debug, Default, PartialEq)]
struct Outline {
    title: String,
    keywords: Vec<String>,
    sections: Vec<(String, String, usize)>,
}

fn json_in(text: &str) -> Option<Value> {
    let body = text.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```");
    body.find('{').zip(body.rfind('}')).filter(|(start, end)| start < end).and_then(|(start, end)| serde_json::from_str(&body[start..=end]).ok())
}

/// The planner's outline, or a standard structure for the genre when the reply cannot be read.
fn parse_outline(text: &str, question: &str, kind: &str, words: usize) -> Outline {
    let mut outline = Outline::default();
    if let Some(value) = json_in(text) {
        outline.title = value["title"].as_str().unwrap_or("").trim().to_string();
        outline.keywords = value["keywords"].as_array().into_iter().flatten().filter_map(Value::as_str).map(|word| word.trim().to_string()).filter(|word| !word.is_empty()).take(6).collect();
        for item in value["sections"].as_array().into_iter().flatten() {
            let heading = item["heading"].as_str().unwrap_or("").trim().trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ' ').trim().to_string();
            if heading.is_empty() || heading.eq_ignore_ascii_case("references") || heading.eq_ignore_ascii_case("abstract") {
                continue;
            }
            let goal = item["goal"].as_str().unwrap_or("").trim().to_string();
            let share = item["words"].as_u64().unwrap_or(0) as usize;
            outline.sections.push((heading, goal, share));
        }
    }
    outline.sections.truncate(9);
    if outline.sections.len() < 3 {
        let review = kind.to_lowercase().contains("review");
        outline.sections = [
            ("Introduction", "Context, why the question matters, the aim and the questions this paper answers, and how it is organised"),
            (if review { "Review method" } else { "Background and theoretical framework" }, if review { "How the literature was found and selected: databases, search terms, period, inclusion criteria, and how many sources were analysed" } else { "Key concepts, theories and prior work the paper builds on" }),
            ("Findings from the literature", "The main evidence, organised by theme, comparing studies and their methods"),
            ("Discussion", "What the evidence means together, where studies disagree and why, and implications"),
            ("Limitations", "Limits of the evidence and of this paper"),
            ("Conclusion", "Answer to the research question, contributions, and directions for future research"),
        ]
        .into_iter()
        .map(|(heading, goal)| (heading.to_string(), goal.to_string(), 0))
        .collect();
    }
    if outline.title.is_empty() {
        outline.title = question.lines().next().unwrap_or("Research paper").chars().take(140).collect();
    }
    // Share the length out: the introduction and conclusion are shorter, findings and discussion longer.
    let weight = |heading: &str| {
        let heading = heading.to_lowercase();
        if heading.contains("intro") || heading.contains("conclu") || heading.contains("limit") || heading.contains("method") { 2 } else { 3 }
    };
    let total: usize = outline.sections.iter().map(|(heading, _, _)| weight(heading)).sum();
    for (heading, _, share) in &mut outline.sections {
        if *share == 0 || *share > words {
            *share = (words * weight(heading) / total.max(1)).clamp(150, 1_600);
        }
    }
    outline
}

/// Swaps the notes' [n] citations for in-text citations in `style`, and collects the numbers cited.
fn cite(text: &str, sources: &[Source], style: &str, cited: &mut BTreeSet<usize>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 1..];
        let Some(end) = tail.find(']') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let inside = &tail[..end];
        let numbers: Vec<usize> = inside.split([',', ';', '–', '-']).map(str::trim).filter_map(|part| part.parse().ok()).collect();
        let valid = !numbers.is_empty() && inside.chars().all(|c| c.is_ascii_digit() || matches!(c, ',' | ';' | ' ' | '–' | '-')) && numbers.iter().all(|number| (1..=sources.len()).contains(number));
        // A link's [text](url) or a stray bracket stays as written.
        if valid && !tail[end + 1..].starts_with('(') {
            cited.extend(numbers.iter().copied());
            let group: Vec<(usize, &Source)> = numbers.iter().map(|number| (*number, &sources[number - 1])).collect();
            out.push_str(&citation::in_text(&group, style));
        } else {
            out.push('[');
            out.push_str(inside);
            out.push(']');
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Drops a heading the model repeated at the top of its section.
fn strip_heading(body: &str, heading: &str) -> String {
    let trimmed = body.trim();
    let first = trimmed.lines().next().unwrap_or("");
    let bare = first.trim_start_matches('#').trim().trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ' ').trim_matches('*').trim();
    let body = if bare.eq_ignore_ascii_case(heading) || first.starts_with("# ") || first.starts_with("## ") { trimmed[first.len()..].trim() } else { trimmed };
    // A References list the model added anyway is Neru's job.
    let body = body.split("\n## References").next().unwrap_or(body);
    body.split("\n### References").next().unwrap_or(body).trim().to_string()
}

fn words_in(text: &str) -> usize {
    text.split_whitespace().count()
}

/// One model call, retried once after a pause when a free model is busy.
async fn ask(client: &reqwest::Client, config: &ProviderConfig, system: &str, user: String) -> Result<String, String> {
    match ask_model(client, config, system, user.clone()).await {
        Ok(text) if !text.trim().is_empty() => Ok(text),
        first => {
            if matches!(&first, Err(error) if error == "cancelled") {
                return first;
            }
            tokio::time::sleep(Duration::from_secs(12)).await;
            ask_model(client, config, system, user).await
        }
    }
}

fn today() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    // Civil date from days since 1970 (Howard Hinnant's algorithm).
    let days = (secs / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    format!("{day} {} {year}", MONTHS[(month - 1) as usize])
}

/// Writes the paper from the gathered research, showing each step in the research panel.
#[allow(clippy::too_many_arguments)]
pub async fn write(app: &AppHandle, session: &str, client: &reqwest::Client, config: &ProviderConfig, question: &str, brief: &Brief, free: bool, about_user: &str) -> Result<Paper, String> {
    let want: &Deliverable = &brief.plan.deliverable;
    let style = citation::get_citation_style();
    let kind = if want.kind.is_empty() { "Literature review".to_string() } else { capitalise(&want.kind) };
    let language = if want.language.is_empty() { "the language of the student's request".to_string() } else { want.language.clone() };
    let words = if want.words > 0 { want.words } else if free { 2_400 } else { 3_600 };
    let mut progress = Progress { app, session, steps: brief.live.clone() };
    let profile = crate::extras::get_profile();
    let search_log = format!(
        "How the literature was gathered (true, for the method section): on {} Neru searched OpenAlex, Semantic Scholar, arXiv, Crossref and PubMed{} for these sub-questions: {}. Field: {}. {} sources were retained and numbered in the notes; the most cited open-access papers were read in full.",
        today(),
        if brief.sources.iter().any(|source| source.authors.is_empty()) { ", plus the public web" } else { "" },
        brief.plan.questions.iter().map(|question| format!("“{question}”")).collect::<Vec<_>>().join("; "),
        if brief.plan.field.is_empty() { "not specified" } else { brief.plan.field.as_str() },
        brief.sources.len()
    );

    // 1. Outline.
    let row = progress.add("Outlining the paper", "running");
    let system = format!(
        "You are an academic writer planning a {kind} of about {words} words for a university student, in {language}. Using only the research notes, design a rigorous structure with 5–8 numbered top-level sections appropriate to the genre (for example Introduction; Review method or Methodology; thematic sections that synthesise the evidence; Discussion; Limitations; Conclusion). Do not include Abstract or References sections. Reply with JSON only: {{\"title\": \"a precise academic title\", \"keywords\": [\"4–6 keywords\"], \"sections\": [{{\"heading\": \"Introduction\", \"goal\": \"what this section must establish, and which source numbers it should draw on\", \"words\": 350}}]}}.{about_user}"
    );
    let reply = ask(client, config, &system, format!("Student's request:\n{question}\n\n{search_log}\n\n{}", brief.notes)).await;
    let outline = match reply {
        Ok(text) => parse_outline(&text, question, &kind, words),
        Err(error) if error == "cancelled" => return Err(error),
        Err(error) => {
            progress.set(row, "error", crate::scholar::clip(&error, 120));
            parse_outline("", question, &kind, words)
        }
    };
    if progress.steps[row].status == "running" {
        progress.set(row, "done", format!("{} sections", outline.sections.len()));
    }

    // 2. Sections, in order, each seeing the outline and how the earlier ones began.
    let plan_text = outline.sections.iter().enumerate().map(|(index, (heading, goal, share))| format!("{}. {heading} (~{share} words): {goal}", index + 1)).collect::<Vec<_>>().join("\n");
    let mut sections: Vec<Section> = Vec::new();
    let mut cited = BTreeSet::new();
    for (index, (heading, goal, share)) in outline.sections.iter().enumerate() {
        let row = progress.add(format!("Writing {}. {heading}", index + 1), "running");
        let so_far = sections.iter().map(|section| format!("{}: {}", section.heading, crate::scholar::clip(&section.body.replace('\n', " "), 450))).collect::<Vec<_>>().join("\n");
        let system = format!(
            "You are an expert academic writer drafting one section of a {kind} titled “{}”, in {language}, in a formal scholarly register. Write section {}. {heading} — about {share} words — fulfilling this goal: {goal}. Write well-developed paragraphs that synthesise and compare sources (methods, samples, effect sizes, contexts) rather than summarising them one by one; use ### subheadings only when the section is long, a Markdown table only when it genuinely clarifies a comparison, and bullet lists sparingly. Support every factual claim with citations to the numbered notes as [n] or [n, m]; cite only numbers that exist in the notes, never invent studies, authors, data or quotations, and say plainly when evidence is limited. Do not write the section heading, an abstract or a reference list. Do not repeat what earlier sections already said.{about_user}",
            outline.title,
            index + 1
        );
        let user = format!(
            "Student's request:\n{question}\n\nOutline:\n{plan_text}\n\n{}Earlier sections (openings):\n{}\n\nResearch notes:\n{}",
            if heading.to_lowercase().contains("method") { format!("{search_log}\n\n") } else { String::new() },
            if so_far.is_empty() { "(none yet)".to_string() } else { so_far },
            brief.notes
        );
        match ask(client, config, &system, user).await {
            Ok(text) => {
                let body = cite(&strip_heading(&text, heading), &brief.sources, &style, &mut cited);
                progress.set(row, "done", format!("{} words", words_in(&body)));
                sections.push(Section { heading: heading.clone(), body });
            }
            Err(error) if error == "cancelled" => return Err(error),
            Err(error) => {
                progress.set(row, "error", crate::scholar::clip(&error, 120));
                return Err(format!("Writing “{heading}” failed: {error}"));
            }
        }
    }

    // 3. Abstract, from the finished sections.
    let row = progress.add("Writing the abstract", "running");
    let body_text = sections.iter().map(|section| format!("## {}\n{}", section.heading, crate::scholar::clip(&section.body, 1_400))).collect::<Vec<_>>().join("\n\n");
    let system = format!("You write the abstract of a {kind} titled “{}”, in {language}: one paragraph of 170–250 words covering background, aim, method (how the literature was gathered and analysed), main findings and conclusion. No citations, headings or bullet points.", outline.title);
    let abstract_text = match ask(client, config, &system, body_text).await {
        Ok(text) => {
            progress.set(row, "done", "");
            strip_heading(&text, "Abstract").trim_start_matches("**Abstract**").trim_start_matches("Abstract:").trim().to_string()
        }
        Err(error) if error == "cancelled" => return Err(error),
        Err(error) => {
            progress.set(row, "error", crate::scholar::clip(&error, 120));
            String::new()
        }
    };

    // 4. References: the works cited, in the style's order.
    let picked: Vec<(usize, &Source)> = cited.iter().map(|number| (*number, &brief.sources[number - 1])).collect();
    let mut references: Vec<Reference> = picked.iter().map(|(number, source)| Reference { number: *number, text: citation::format(source, &style), source: (*source).clone() }).collect();
    if style == "ieee" {
        for reference in &mut references {
            reference.text = format!("[{}] {}", reference.number, reference.text);
        }
    } else {
        references.sort_by_key(|reference| reference.text.trim_start_matches(['“', '‘', '*']).to_lowercase());
    }

    let words = words_in(&abstract_text) + sections.iter().map(|section| words_in(&section.body)).sum::<usize>();
    let paper = Paper {
        id: uuid::Uuid::new_v4().to_string(),
        title: outline.title,
        kind,
        authors: if profile.name.trim().is_empty() { Vec::new() } else { vec![profile.name.trim().to_string()] },
        affiliation: if profile.personalize { profile.occupation.trim().to_string() } else { String::new() },
        date: today(),
        abstract_text,
        keywords: outline.keywords,
        sections,
        references,
        style,
        format: want.format.clone(),
        language: want.language.clone(),
        question: question.chars().take(2_000).collect(),
        words,
        created: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
    };
    save(session, &paper)?;
    progress.add("Saved the paper", "done");
    Ok(paper)
}

fn capitalise(text: &str) -> String {
    let mut chars = text.trim().chars();
    chars.next().map(|first| first.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// The chat message for a finished paper: a short note and the card that opens it.
pub fn message(paper: &Paper) -> String {
    let format = match paper.format.as_str() {
        "docx" => " Download it as a Word document from the panel.",
        "pdf" => " Download it as a PDF from the panel.",
        "" => " Download it as Word, PDF, LaTeX, HTML or Markdown from the panel.",
        _ => " Download it from the panel.",
    };
    let card = serde_json::to_string(&paper.card()).unwrap_or_default();
    format!(
        "I wrote **{}**, a {}-word {} drawing on {} sources, cited in {}.{format}\n\n```neru-document\n{card}\n```",
        paper.title,
        paper.words,
        paper.kind.to_lowercase(),
        paper.references.len(),
        paper.style.to_uppercase()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(author: &str, year: i32) -> Source {
        Source { id: format!("s-{author}"), title: format!("A study by {author}"), url: "https://doi.org/10.1/x".into(), domain: "doi.org".into(), authors: vec![format!("Jane {author}")], year: Some(year), venue: "Journal".into(), doi: "10.1/x".into() }
    }

    #[test]
    fn citations_become_the_styles_in_text_form() {
        let sources = vec![source("Doe", 2020), source("Lee", 2018)];
        let mut cited = BTreeSet::new();
        let text = cite("Sleep helps memory [1, 2]. See [link](https://x.org) and [3]. One more [2].", &sources, "apa", &mut cited);
        assert_eq!(text, "Sleep helps memory (Doe, 2020; Lee, 2018). See [link](https://x.org) and [3]. One more (Lee, 2018).");
        assert_eq!(cited.into_iter().collect::<Vec<_>>(), vec![1, 2]);
        let mut cited = BTreeSet::new();
        assert_eq!(cite("Shown [2].", &sources, "ieee", &mut cited), "Shown [2].");
    }

    #[test]
    fn outlines_parse_or_fall_back_to_the_genre() {
        let outline = parse_outline(r#"{"title":"Sleep and learning","keywords":["sleep","memory"],"sections":[{"heading":"1. Introduction","goal":"why","words":300},{"heading":"Evidence","goal":"what"},{"heading":"Conclusion","goal":"so"},{"heading":"References"}]}"#, "q", "literature review", 2_400);
        assert_eq!(outline.title, "Sleep and learning");
        assert_eq!(outline.sections.iter().map(|(heading, _, _)| heading.as_str()).collect::<Vec<_>>(), vec!["Introduction", "Evidence", "Conclusion"]);
        assert_eq!(outline.sections[0].2, 300);
        assert!(outline.sections[1].2 >= 150);
        let fallback = parse_outline("not json", "Does sleep help?", "literature review", 2_400);
        assert_eq!(fallback.title, "Does sleep help?");
        assert!(fallback.sections.iter().any(|(heading, _, _)| heading == "Review method"));
    }

    #[test]
    fn repeated_headings_and_reference_lists_are_dropped() {
        assert_eq!(strip_heading("## 2. Discussion\n\nText here.", "Discussion"), "Text here.");
        assert_eq!(strip_heading("Text.\n\n## References\n[1] x", "Discussion"), "Text.");
        assert_eq!(strip_heading("**Introduction**\nBody", "Introduction"), "Body");
    }

    #[test]
    fn markdown_has_abstract_sections_and_references() {
        let paper = Paper { title: "T".into(), abstract_text: "A.".into(), sections: vec![Section { heading: "Intro".into(), body: "B.".into() }], references: vec![Reference { number: 1, text: "Doe, J. (2020).".into(), source: source("Doe", 2020) }], ..Paper::default() };
        let text = paper.markdown();
        assert!(text.starts_with("# T") && text.contains("## Abstract\n\nA.") && text.contains("## Intro\n\nB.") && text.ends_with("Doe, J. (2020)."));
    }

    #[test]
    fn dates_read_as_day_month_year() {
        let date = today();
        assert!(date.split(' ').count() == 3 && date.ends_with(|c: char| c.is_ascii_digit()));
    }
}
