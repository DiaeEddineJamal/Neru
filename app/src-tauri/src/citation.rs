//! Bibliographies from the numbered sources of an answer: APA 7, MLA 9, Chicago (author-date), IEEE and
//! Harvard (Cite Them Right) as Markdown, plus BibTeX and RIS for reference managers. The style the user
//! picks is kept in `research.json` in the data folder.

use std::{collections::HashSet, fs, path::Path};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::web::Source;

pub const STYLES: [&str; 5] = ["apa", "mla", "chicago", "ieee", "harvard"];

/// A name split for citing: given names and the family name ("van Gogh" keeps its particle).
struct Name {
    given: Vec<String>,
    family: String,
}

fn split_name(name: &str) -> Name {
    let name = name.trim();
    if let Some((family, given)) = name.split_once(',') {
        return Name { given: given.split_whitespace().map(str::to_string).collect(), family: family.trim().to_string() };
    }
    let words: Vec<&str> = name.split_whitespace().collect();
    if words.len() <= 1 {
        return Name { given: Vec::new(), family: name.to_string() };
    }
    let mut start = words.len() - 1;
    while start > 1 && matches!(words[start - 1].to_lowercase().as_str(), "van" | "von" | "de" | "der" | "den" | "da" | "del" | "della" | "di" | "du" | "le" | "la" | "bin" | "al") {
        start -= 1;
    }
    Name { given: words[..start].iter().map(|word| word.to_string()).collect(), family: words[start..].join(" ") }
}

/// "Jean-Paul Q." → ["J.-P.", "Q."].
fn initials(given: &[String]) -> Vec<String> {
    given
        .iter()
        .map(|word| word.trim_end_matches('.').split('-').filter_map(|part| part.chars().next()).map(|c| format!("{}.", c.to_uppercase())).collect::<Vec<_>>().join("-"))
        .filter(|initial| !initial.is_empty())
        .collect()
}

impl Name {
    /// "Doe, J. Q."
    fn family_initials(&self, spaced: bool) -> String {
        let letters = initials(&self.given).join(if spaced { " " } else { "" });
        if letters.is_empty() { self.family.clone() } else { format!("{}, {letters}", self.family) }
    }
    /// "J. Q. Doe"
    fn initials_family(&self) -> String {
        let letters = initials(&self.given).join(" ");
        if letters.is_empty() { self.family.clone() } else { format!("{letters} {}", self.family) }
    }
    /// "Doe, Jane Q."
    fn inverted(&self) -> String {
        if self.given.is_empty() { self.family.clone() } else { format!("{}, {}", self.family, self.given.join(" ")) }
    }
    /// "Jane Q. Doe"
    fn natural(&self) -> String {
        if self.given.is_empty() { self.family.clone() } else { format!("{} {}", self.given.join(" "), self.family) }
    }
}

fn names(source: &Source) -> Vec<Name> {
    source.authors.iter().filter(|name| !name.trim().is_empty()).map(|name| split_name(name)).collect()
}

/// Joins with commas and a final conjunction: "A, B, & C" (`serial`) or "A, B and C".
fn join_list(items: &[String], conjunction: &str, serial: bool) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        2 if conjunction == "&" => format!("{}, & {}", items[0], items[1]),
        2 => format!("{} {conjunction} {}", items[0], items[1]),
        _ => format!("{}{} {conjunction} {}", items[..items.len() - 1].join(", "), if serial { "," } else { "" }, items[items.len() - 1]),
    }
}

/// Ends a title with a period unless it already ends a sentence.
fn sentence(text: &str) -> String {
    let text = text.trim();
    if text.ends_with(['.', '?', '!']) { text.to_string() } else { format!("{text}.") }
}

fn year(source: &Source) -> String {
    source.year.map(|year| year.to_string()).unwrap_or_else(|| "n.d.".into())
}

fn link(source: &Source) -> String {
    if source.doi.is_empty() { source.url.clone() } else { format!("https://doi.org/{}", source.doi) }
}

/// Where it appeared: the journal, or the site for a web page.
fn venue(source: &Source) -> String {
    if !source.venue.is_empty() { source.venue.clone() } else if source.authors.is_empty() && source.doi.is_empty() { source.domain.clone() } else { String::new() }
}

fn italic(text: &str) -> String {
    if text.is_empty() { String::new() } else { format!("*{text}*") }
}

pub fn apa(source: &Source) -> String {
    let people = names(source);
    let listed: Vec<String> = people.iter().map(|name| name.family_initials(true)).collect();
    // APA lists up to 20 authors; beyond that the first 19, an ellipsis, and the last.
    let authors = if listed.len() > 20 { format!("{}, … {}", listed[..19].join(", "), listed[listed.len() - 1]) } else { join_list(&listed, "&", true) };
    let venue = venue(source);
    let mut parts = Vec::new();
    if authors.is_empty() {
        parts.push(sentence(&source.title));
        parts.push(format!("({}).", year(source)));
    } else {
        parts.push(format!("{authors} ({}).", year(source)));
        parts.push(sentence(&source.title));
    }
    if !venue.is_empty() {
        parts.push(format!("{}.", italic(&venue)));
    }
    parts.push(link(source));
    parts.join(" ")
}

pub fn mla(source: &Source) -> String {
    let people = names(source);
    let authors = match people.len() {
        0 => String::new(),
        1 => people[0].inverted(),
        2 => format!("{}, and {}", people[0].inverted(), people[1].natural()),
        _ => format!("{}, et al", people[0].inverted()),
    };
    let mut text = String::new();
    if !authors.is_empty() {
        text.push_str(&sentence(&authors));
        text.push(' ');
    }
    text.push_str(&format!("“{}”", sentence(&source.title)));
    let tail: Vec<String> = [italic(&venue(source)), source.year.map(|year| year.to_string()).unwrap_or_default(), link(source)].into_iter().filter(|part| !part.is_empty()).collect();
    if !tail.is_empty() {
        text.push_str(&format!(" {}.", tail.join(", ")));
    }
    text
}

pub fn chicago(source: &Source) -> String {
    let people = names(source);
    let mut listed: Vec<String> = people.iter().enumerate().map(|(index, name)| if index == 0 { name.inverted() } else { name.natural() }).collect();
    if listed.len() > 10 {
        listed.truncate(7);
        listed = vec![format!("{} et al", listed.join(", "))];
    }
    // The first name is inverted, so even two authors take a comma before "and".
    let authors = if listed.len() == 2 { format!("{}, and {}", listed[0], listed[1]) } else { join_list(&listed, "and", true) };
    let mut parts = Vec::new();
    if authors.is_empty() {
        parts.push(format!("“{}”", sentence(&source.title)));
        parts.push(format!("{}.", year(source)));
    } else {
        parts.push(sentence(&authors));
        parts.push(format!("{}.", year(source)));
        parts.push(format!("“{}”", sentence(&source.title)));
    }
    let venue = venue(source);
    if !venue.is_empty() {
        parts.push(format!("{}.", italic(&venue)));
    }
    let link = link(source);
    if !link.is_empty() {
        parts.push(format!("{link}."));
    }
    parts.join(" ")
}

pub fn ieee(source: &Source) -> String {
    let people = names(source);
    let authors = if people.len() > 6 {
        format!("{} *et al.*", people[0].initials_family())
    } else {
        join_list(&people.iter().map(Name::initials_family).collect::<Vec<_>>(), "and", true)
    };
    let mut parts = Vec::new();
    if !authors.is_empty() {
        parts.push(format!("{authors},"));
    }
    let title = source.title.trim().trim_end_matches('.');
    let venue = venue(source);
    let mut tail = Vec::new();
    if !venue.is_empty() {
        tail.push(italic(&venue));
    }
    if let Some(year) = source.year {
        tail.push(year.to_string());
    }
    if tail.is_empty() {
        parts.push(format!("“{title}.”"));
    } else {
        parts.push(format!("“{title},” {}.", tail.join(", ")));
    }
    if !source.doi.is_empty() {
        parts.push(format!("doi: {}.", source.doi));
    } else if !source.url.is_empty() {
        parts.push(format!("[Online]. Available: {}", source.url));
    }
    parts.join(" ")
}

pub fn harvard(source: &Source) -> String {
    let people = names(source);
    let authors = if people.len() >= 4 {
        format!("{} et al.", people[0].family_initials(false))
    } else {
        join_list(&people.iter().map(|name| name.family_initials(false)).collect::<Vec<_>>(), "and", false)
    };
    let mut parts = Vec::new();
    if authors.is_empty() {
        parts.push(format!("{} ({})", source.title.trim().trim_end_matches('.'), year(source)));
    } else {
        parts.push(format!("{authors} ({})", year(source)));
        parts.push(format!("‘{}’,", source.title.trim().trim_end_matches('.')));
    }
    let venue = venue(source);
    let mut text = parts.join(" ");
    if !venue.is_empty() {
        text.push_str(&format!(" {}.", italic(&venue)));
    } else if !text.ends_with(',') {
        text.push('.');
    } else {
        text.pop();
        text.push('.');
    }
    let link = link(source);
    if !link.is_empty() {
        text.push_str(&format!(" Available at: {link}."));
    }
    text
}

pub fn format(source: &Source, style: &str) -> String {
    match style {
        "mla" => mla(source),
        "chicago" => chicago(source),
        "ieee" => ieee(source),
        "harvard" => harvard(source),
        _ => apa(source),
    }
}

/// Every source in `style`. Numbered keeps the answer's [n] order; otherwise author-date styles are
/// sorted by author as a reference list is (IEEE always keeps its numbers).
pub fn bibliography(sources: &[(usize, &Source)], style: &str, numbered: bool) -> Vec<String> {
    if numbered || style == "ieee" {
        return sources.iter().map(|(number, source)| format!("[{number}] {}", format(source, style))).collect();
    }
    let mut entries: Vec<String> = sources.iter().map(|(_, source)| format(source, style)).collect();
    entries.sort_by_key(|entry| entry.trim_start_matches(['“', '‘', '*']).to_lowercase());
    entries
}

/// An in-text citation for one or more sources: "(Doe & Smith, 2020; Lee et al., 2019)" in APA and
/// Harvard, "(Doe and Smith 2020)" in Chicago author-date, "(Doe and Smith)" in MLA, "[1], [4]" in IEEE.
pub fn in_text(group: &[(usize, &Source)], style: &str) -> String {
    if style == "ieee" {
        return group.iter().map(|(number, _)| format!("[{number}]")).collect::<Vec<_>>().join(", ");
    }
    let one = |source: &Source| {
        let families: Vec<String> = names(source).into_iter().map(|name| name.family).collect();
        let who = match families.len() {
            0 => {
                let title: String = source.title.split_whitespace().take(4).collect::<Vec<_>>().join(" ");
                if title.is_empty() { source.domain.clone() } else { format!("“{title}”") }
            }
            1 => families[0].clone(),
            2 => format!("{} {} {}", families[0], if matches!(style, "apa" | "harvard") { "&" } else { "and" }, families[1]),
            _ => format!("{} et al.", families[0]),
        };
        let year = year(source);
        match style {
            "mla" => who,
            "chicago" => format!("{who} {year}"),
            _ => format!("{who}, {year}"),
        }
    };
    format!("({})", group.iter().map(|(_, source)| one(source)).collect::<Vec<_>>().join("; "))
}

// ---------- BibTeX and RIS ----------

fn bibtex_escape(text: &str) -> String {
    text.chars()
        .flat_map(|c| match c {
            '&' | '%' | '$' | '#' | '_' => vec!['\\', c],
            '{' | '}' => vec![],
            _ => vec![c],
        })
        .collect()
}

fn bibtex_key(source: &Source, taken: &mut HashSet<String>) -> String {
    let family = names(source).first().map(|name| name.family.clone()).unwrap_or_else(|| source.domain.clone());
    let word = source.title.split_whitespace().find(|word| word.len() > 3).unwrap_or("source");
    let clean = |text: &str| text.to_lowercase().chars().filter(char::is_ascii_alphanumeric).collect::<String>();
    let base = format!("{}{}{}", clean(&family), source.year.map(|year| year.to_string()).unwrap_or_default(), clean(word));
    let base = if base.is_empty() { "source".to_string() } else { base };
    let mut key = base.clone();
    let mut suffix = b'a';
    while !taken.insert(key.clone()) {
        key = format!("{base}{}", suffix as char);
        suffix += 1;
    }
    key
}

pub fn bibtex(sources: &[&Source]) -> String {
    let mut taken = HashSet::new();
    sources
        .iter()
        .map(|source| {
            let scholarly = !source.authors.is_empty() || !source.doi.is_empty();
            let kind = if !scholarly { "misc" } else if source.venue.is_empty() || source.venue.eq_ignore_ascii_case("arxiv") { "misc" } else { "article" };
            let mut fields = Vec::new();
            if !source.authors.is_empty() {
                fields.push(("author", names(source).iter().map(Name::inverted).collect::<Vec<_>>().join(" and ")));
            }
            fields.push(("title", format!("{{{}}}", bibtex_escape(source.title.trim()))));
            if kind == "article" {
                fields.push(("journal", bibtex_escape(&source.venue)));
            } else if !source.venue.is_empty() {
                fields.push(("howpublished", bibtex_escape(&source.venue)));
            }
            if let Some(year) = source.year {
                fields.push(("year", year.to_string()));
            }
            if !source.doi.is_empty() {
                fields.push(("doi", source.doi.clone()));
            }
            if !source.url.is_empty() {
                fields.push(("url", source.url.clone()));
            }
            let body = fields.iter().map(|(name, value)| format!("  {name} = {{{value}}}")).collect::<Vec<_>>().join(",\n");
            format!("@{kind}{{{},\n{body}\n}}", bibtex_key(source, &mut taken))
        })
        .collect::<Vec<_>>()
        .join("\n\n")
        + "\n"
}

pub fn ris(sources: &[&Source]) -> String {
    let mut out = String::new();
    for source in sources {
        let scholarly = !source.authors.is_empty() || !source.doi.is_empty();
        out.push_str(&format!("TY  - {}\r\n", if !scholarly { "ELEC" } else if source.venue.is_empty() { "GEN" } else { "JOUR" }));
        for name in names(source) {
            out.push_str(&format!("AU  - {}\r\n", name.inverted()));
        }
        out.push_str(&format!("TI  - {}\r\n", source.title.trim()));
        let venue = venue(source);
        if !venue.is_empty() {
            out.push_str(&format!("{}  - {venue}\r\n", if scholarly { "JO" } else { "T2" }));
        }
        if let Some(year) = source.year {
            out.push_str(&format!("PY  - {year}\r\n"));
        }
        if !source.doi.is_empty() {
            out.push_str(&format!("DO  - {}\r\n", source.doi));
        }
        if !source.url.is_empty() {
            out.push_str(&format!("UR  - {}\r\n", source.url));
        }
        out.push_str("ER  - \r\n\r\n");
    }
    out
}

// ---------- answers ----------

/// The [n] numbers an answer cites, in order of first use.
pub fn cited_numbers(content: &str) -> Vec<usize> {
    let marker = Regex::new(r"\[(\d+(?:\s*[,–-]\s*\d+)*)\]").expect("cite pattern");
    let mut seen = Vec::new();
    for caps in marker.captures_iter(content) {
        for part in caps[1].split(',') {
            let bounds: Vec<usize> = part.split(['–', '-']).filter_map(|n| n.trim().parse().ok()).collect();
            let range = match bounds.as_slice() {
                [one] => *one..=*one,
                [from, to] if from <= to && to - from < 50 => *from..=*to,
                _ => continue,
            };
            for number in range {
                if !seen.contains(&number) {
                    seen.push(number);
                }
            }
        }
    }
    seen
}

/// Appends a References section in `style` for the sources the answer cites (all of them when it cites
/// none by number), keeping the answer's numbers.
pub fn with_references(content: &str, sources: &[Source], style: &str) -> String {
    if sources.is_empty() || content.contains("\n## References") {
        return content.to_string();
    }
    let mut cited: Vec<usize> = cited_numbers(content).into_iter().filter(|number| (1..=sources.len()).contains(number)).collect();
    if cited.is_empty() {
        return content.to_string();
    }
    cited.sort_unstable();
    let picked: Vec<(usize, &Source)> = cited.into_iter().map(|number| (number, &sources[number - 1])).collect();
    let list = bibliography(&picked, style, true).into_iter().map(|entry| format!("{entry}  ")).collect::<Vec<_>>().join("\n");
    format!("{}\n\n## References\n\n{list}", content.trim_end())
}

// ---------- commands ----------

#[derive(Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Prefs {
    citation_style: String,
}

fn prefs_path() -> Result<std::path::PathBuf, String> {
    Ok(crate::workspace::data_dir()?.join("research.json"))
}

/// The citation style the user picked (APA when none).
#[tauri::command]
pub fn get_citation_style() -> String {
    prefs_path()
        .ok()
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Prefs>(&text).ok())
        .map(|prefs| prefs.citation_style)
        .filter(|style| STYLES.contains(&style.as_str()))
        .unwrap_or_else(|| "apa".into())
}

#[tauri::command]
pub fn set_citation_style(style: String) -> Result<String, String> {
    if !STYLES.contains(&style.as_str()) {
        return Err("Unknown citation style".into());
    }
    fs::write(prefs_path()?, serde_json::to_vec_pretty(&Prefs { citation_style: style.clone() }).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(style)
}

/// Entries for an answer's sources: numbered in [n] order for display, or as a reference list to copy.
#[tauri::command]
pub fn format_bibliography(sources: Vec<Source>, style: String, numbered: bool) -> Vec<String> {
    let indexed: Vec<(usize, &Source)> = sources.iter().enumerate().map(|(index, source)| (index + 1, source)).collect();
    bibliography(&indexed, &style, numbered)
}

/// The sources as a BibTeX or RIS file's text.
#[tauri::command]
pub fn export_citations(sources: Vec<Source>, format: String) -> Result<String, String> {
    let refs: Vec<&Source> = sources.iter().collect();
    match format.as_str() {
        "bibtex" => Ok(bibtex(&refs)),
        "ris" => Ok(ris(&refs)),
        _ => Err("Export as bibtex or ris".into()),
    }
}

/// Writes a document export (a research paper as Word, PDF, HTML, LaTeX or Markdown) where the user
/// chose with the save dialog. The bytes come base64-encoded from the window.
#[tauri::command]
pub fn save_export_file(path: String, base64: String) -> Result<(), String> {
    use base64::Engine;
    let path = Path::new(&path);
    let ext = path.extension().map(|ext| ext.to_string_lossy().to_lowercase()).unwrap_or_default();
    if !path.is_absolute() || !matches!(ext.as_str(), "docx" | "pdf" | "html" | "tex" | "md" | "txt" | "bib" | "ris") {
        return Err("Save as .docx, .pdf, .html, .tex, .md, .bib or .ris".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD.decode(base64.trim()).map_err(|e| e.to_string())?;
    fs::write(path, bytes).map_err(|e| e.to_string())
}

/// Writes an export the user placed with the save dialog. Only reference and study files.
#[tauri::command]
pub fn save_study_file(path: String, contents: String) -> Result<(), String> {
    let path = Path::new(&path);
    let ext = path.extension().map(|ext| ext.to_string_lossy().to_lowercase()).unwrap_or_default();
    if !path.is_absolute() || !matches!(ext.as_str(), "bib" | "ris" | "csv" | "txt" | "md") {
        return Err("Save as .bib, .ris, .csv, .txt or .md".into());
    }
    fs::write(path, contents).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_text_citations_follow_the_style() {
        let one = paper();
        let mut many = paper();
        many.authors = vec!["Ann Lee".into(), "Bo Kim".into(), "Cy Park".into()];
        many.year = Some(2019);
        let group = [(1, &one), (2, &many)];
        assert_eq!(in_text(&group, "apa"), "(Doe & Smith, 2004; Lee et al., 2019)");
        assert_eq!(in_text(&group, "chicago"), "(Doe and Smith 2004; Lee et al. 2019)");
        assert_eq!(in_text(&group, "mla"), "(Doe and Smith; Lee et al.)");
        assert_eq!(in_text(&group, "ieee"), "[1], [2]");
    }

    fn paper() -> Source {
        Source {
            id: "source-1".into(),
            title: "Remembering and forgetting".into(),
            url: "https://doi.org/10.1037/0003-066x.59.1.29".into(),
            domain: "doi.org".into(),
            authors: vec!["Jane Q. Doe".into(), "Alan Smith".into()],
            year: Some(2004),
            venue: "American Psychologist".into(),
            doi: "10.1037/0003-066x.59.1.29".into(),
        }
    }

    fn page() -> Source {
        Source { id: "source-2".into(), title: "How sleep works".into(), url: "https://www.sleep.org/how".into(), domain: "sleep.org".into(), ..Default::default() }
    }

    fn many(count: usize) -> Source {
        Source { authors: (0..count).map(|index| format!("Ann {}", (b'A' + index as u8) as char)).collect(), ..paper() }
    }

    #[test]
    fn apa_journal_article_and_web_page() {
        assert_eq!(apa(&paper()), "Doe, J. Q., & Smith, A. (2004). Remembering and forgetting. *American Psychologist*. https://doi.org/10.1037/0003-066x.59.1.29");
        assert_eq!(apa(&page()), "How sleep works. (n.d.). *sleep.org*. https://www.sleep.org/how");
        assert!(apa(&many(3)).starts_with("A, A., B, A., & C, A. (2004)."));
    }

    #[test]
    fn mla_two_and_three_authors() {
        assert_eq!(mla(&paper()), "Doe, Jane Q., and Alan Smith. “Remembering and forgetting.” *American Psychologist*, 2004, https://doi.org/10.1037/0003-066x.59.1.29.");
        assert!(mla(&many(3)).starts_with("A, Ann, et al. “"));
        assert_eq!(mla(&page()), "“How sleep works.” *sleep.org*, https://www.sleep.org/how.");
    }

    #[test]
    fn chicago_author_date() {
        assert_eq!(chicago(&paper()), "Doe, Jane Q., and Alan Smith. 2004. “Remembering and forgetting.” *American Psychologist*. https://doi.org/10.1037/0003-066x.59.1.29.");
        assert_eq!(chicago(&many(3)).split(". 2004").next().unwrap(), "A, Ann, Ann B, and Ann C");
    }

    #[test]
    fn ieee_uses_initials_and_et_al() {
        assert_eq!(ieee(&paper()), "J. Q. Doe and A. Smith, “Remembering and forgetting,” *American Psychologist*, 2004. doi: 10.1037/0003-066x.59.1.29.");
        assert!(ieee(&many(7)).starts_with("A. A *et al.*, “"));
        assert_eq!(ieee(&many(3)).split(", “").next().unwrap(), "A. A, A. B, and A. C");
        assert_eq!(ieee(&page()), "“How sleep works,” *sleep.org*. [Online]. Available: https://www.sleep.org/how");
    }

    #[test]
    fn harvard_cite_them_right() {
        assert_eq!(harvard(&paper()), "Doe, J.Q. and Smith, A. (2004) ‘Remembering and forgetting’, *American Psychologist*. Available at: https://doi.org/10.1037/0003-066x.59.1.29.");
        assert!(harvard(&many(4)).starts_with("A, A. et al. (2004)"));
        assert_eq!(harvard(&page()), "How sleep works (n.d.) *sleep.org*. Available at: https://www.sleep.org/how.");
    }

    #[test]
    fn names_keep_particles_and_commas() {
        assert_eq!(split_name("Ludwig van Beethoven").family, "van Beethoven");
        assert_eq!(split_name("Doe, Jane").family_initials(true), "Doe, J.");
        assert_eq!(split_name("Jean-Paul Sartre").family_initials(true), "Sartre, J.-P.");
        assert_eq!(split_name("UNESCO").family_initials(true), "UNESCO");
    }

    #[test]
    fn reference_lists_sort_or_number() {
        let (a, b) = (paper(), page());
        let list = vec![(1, &a), (2, &b)];
        let numbered = bibliography(&list, "apa", true);
        assert!(numbered[0].starts_with("[1] Doe") && numbered[1].starts_with("[2] How"));
        let sorted = bibliography(&list, "apa", false);
        assert!(sorted[0].starts_with("Doe") && sorted[1].starts_with("How"));
        assert!(bibliography(&list, "ieee", false)[0].starts_with("[1] J. Q. Doe"));
    }

    #[test]
    fn bibtex_and_ris() {
        let (a, b) = (paper(), page());
        let bib = bibtex(&[&a, &b, &a]);
        assert!(bib.contains("@article{doe2004remembering,"), "{bib}");
        assert!(bib.contains("author = {Doe, Jane Q. and Smith, Alan}"));
        assert!(bib.contains("journal = {American Psychologist}"));
        assert!(bib.contains("doi = {10.1037/0003-066x.59.1.29}"));
        assert!(bib.contains("@misc{sleeporgsleep,"), "{bib}");
        assert!(bib.contains("@article{doe2004rememberinga,"), "{bib}");
        let ris = ris(&[&a, &b]);
        assert!(ris.starts_with("TY  - JOUR\r\nAU  - Doe, Jane Q.\r\nAU  - Smith, Alan\r\nTI  - Remembering and forgetting\r\nJO  - American Psychologist\r\nPY  - 2004\r\nDO  - 10.1037/0003-066x.59.1.29\r\n"), "{ris}");
        assert!(ris.contains("TY  - ELEC\r\n") && ris.contains("T2  - sleep.org\r\n"));
        assert_eq!(ris.matches("ER  - ").count(), 2);
    }

    #[test]
    fn references_follow_the_answer_numbers() {
        assert_eq!(cited_numbers("A [2], B [1, 3] and C [4–5] [2]"), vec![2, 1, 3, 4, 5]);
        let sources = vec![page(), paper(), page()];
        let text = with_references("Sleep helps memory [2].", &sources, "apa");
        assert!(text.ends_with("## References\n\n[2] Doe, J. Q., & Smith, A. (2004). Remembering and forgetting. *American Psychologist*. https://doi.org/10.1037/0003-066x.59.1.29  "), "{text}");
        assert_eq!(with_references("No citations.", &sources, "apa"), "No citations.");
    }
}
