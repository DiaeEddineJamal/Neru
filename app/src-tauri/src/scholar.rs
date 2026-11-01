//! Academic sources for Research mode: OpenAlex, Semantic Scholar, arXiv, Crossref, PubMed (E-utilities)
//! and Unpaywall, all free and without keys. Each host is paced to its published rate (arXiv asks for one
//! request every three seconds), answers are cached for an hour, and nothing is sent when the privacy rule
//! keeps requests on this machine. Open-access PDFs are saved under the data folder and read in the window
//! with the same PDF.js extraction as attached documents (src/lib/documents.ts).

use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex, OnceLock},
    time::{Duration, Instant},
};

use reqwest::{Client, Url};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};

use crate::web::{self, Source};

const USER_AGENT: &str = concat!("Neru/", env!("CARGO_PKG_VERSION"), " (research mode)");
const CACHE_TTL: Duration = Duration::from_secs(3_600);
const CACHE_ENTRIES: usize = 300;
const MAX_PDF_BYTES: usize = 25_000_000;
const PER_SOURCE: usize = 8;

/// One paper, as every source reports it.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Paper {
    /// `doi:…`, `arxiv:…`, `pmid:…`, `openalex:W…` or `s2:…`.
    pub id: String,
    pub title: String,
    /// Display names, "Given Family".
    pub authors: Vec<String>,
    pub year: Option<i32>,
    pub venue: String,
    /// Bare DOI, lower case, without the https://doi.org/ prefix.
    pub doi: String,
    pub url: String,
    /// An open-access full text (PDF or HTML), when one is known.
    pub pdf_url: String,
    pub abstract_text: String,
    pub cited_by: Option<u64>,
    /// Which service reported it first.
    pub source: String,
}

impl Paper {
    /// The structured citation record shown with the answer and used for the bibliography.
    pub fn to_source(&self, number: usize) -> Source {
        Source {
            id: format!("source-{number}"),
            title: self.title.clone(),
            url: self.link(),
            domain: web::domain(&self.link()),
            authors: self.authors.clone(),
            year: self.year,
            venue: self.venue.clone(),
            doi: self.doi.clone(),
        }
    }

    pub fn link(&self) -> String {
        if !self.doi.is_empty() {
            format!("https://doi.org/{}", self.doi)
        } else {
            self.url.clone()
        }
    }

    /// A compact block for the model: who, when, where, and the abstract.
    pub fn describe(&self, number: usize, abstract_chars: usize) -> String {
        let mut text = format!("[{number}] {}", self.title);
        if !self.authors.is_empty() {
            let names = if self.authors.len() > 4 { format!("{} et al.", self.authors[..3].join(", ")) } else { self.authors.join(", ") };
            text.push_str(&format!("\n{names}"));
        }
        let mut meta = Vec::new();
        if let Some(year) = self.year {
            meta.push(year.to_string());
        }
        if !self.venue.is_empty() {
            meta.push(self.venue.clone());
        }
        if let Some(count) = self.cited_by {
            meta.push(format!("cited by {count}"));
        }
        if !self.pdf_url.is_empty() {
            meta.push("open access".into());
        }
        if !meta.is_empty() {
            text.push_str(&format!("\n{}", meta.join(" · ")));
        }
        text.push_str(&format!("\nid: {} · {}", self.id, self.link()));
        if !self.abstract_text.is_empty() && abstract_chars > 0 {
            text.push_str(&format!("\nAbstract: {}", clip(&self.abstract_text, abstract_chars)));
        }
        text
    }

    /// Fills what this record lacks from another record of the same paper.
    fn merge(&mut self, other: &Paper) {
        if self.authors.is_empty() {
            self.authors = other.authors.clone();
        }
        if self.year.is_none() {
            self.year = other.year;
        }
        for (mine, theirs) in [(&mut self.venue, &other.venue), (&mut self.doi, &other.doi), (&mut self.url, &other.url), (&mut self.pdf_url, &other.pdf_url), (&mut self.abstract_text, &other.abstract_text)] {
            if mine.is_empty() {
                *mine = theirs.clone();
            }
        }
        self.cited_by = self.cited_by.max(other.cited_by);
    }
}

pub fn clip(text: &str, chars: usize) -> String {
    let mut out: String = text.chars().take(chars).collect();
    if out.len() < text.len() {
        out.push('…');
    }
    out
}

/// What the user can pass to search_papers.
#[derive(Clone, Debug, Default)]
pub struct Query {
    pub text: String,
    pub from_year: Option<i32>,
    pub to_year: Option<i32>,
    pub field: String,
}

impl Query {
    pub fn from_args(args: &Value) -> Self {
        let year = |key: &str| args[key].as_i64().or_else(|| args[key].as_str().and_then(|text| text.trim().parse().ok())).map(|year| year as i32).filter(|year| (1000..=2200).contains(year));
        Self {
            text: args["query"].as_str().unwrap_or("").trim().to_string(),
            from_year: year("from_year"),
            to_year: year("to_year"),
            field: args["field"].as_str().unwrap_or("").trim().to_string(),
        }
    }

    fn in_range(&self, paper: &Paper) -> bool {
        match paper.year {
            Some(year) => self.from_year.is_none_or(|from| year >= from) && self.to_year.is_none_or(|to| year <= to),
            None => self.from_year.is_none() && self.to_year.is_none(),
        }
    }

    fn medical(&self) -> bool {
        let field = format!("{} {}", self.field, self.text).to_lowercase();
        ["medic", "health", "clinic", "biolog", "nurs", "pharma", "disease", "patient", "neuro", "genet", "psychiat", "epidem"].iter().any(|word| field.contains(word))
    }
}

pub const SCHOLAR_TOOLS: &str = r#"[
    {"type":"function","function":{"name":"search_papers","description":"Search scholarly literature (OpenAlex, Semantic Scholar, arXiv, and PubMed for medicine and biology). Returns numbered papers with authors, year, venue, DOI and abstract; cite them inline as [n]. Prefer this over web_search for academic questions.","parameters":{"type":"object","properties":{"query":{"type":"string","description":"Keywords, not a full sentence"},"from_year":{"type":"integer"},"to_year":{"type":"integer"},"field":{"type":"string","description":"Optional discipline, e.g. psychology, computer science, medicine"}},"required":["query"]}}},
    {"type":"function","function":{"name":"get_paper","description":"Look up one paper by DOI, arXiv id, PubMed id (pmid:…), OpenAlex id or a paper id from search_papers. Returns its full record and abstract as a numbered source.","parameters":{"type":"object","properties":{"id":{"type":"string"}},"required":["id"]}}},
    {"type":"function","function":{"name":"read_paper","description":"Read the open-access full text of a paper (by id from search_papers, DOI, or a PDF/HTML link). Use for the few papers that matter most; it is slow. Returns a numbered source.","parameters":{"type":"object","properties":{"id":{"type":"string"}},"required":["id"]}}}
]"#;

pub fn is_tool(name: &str) -> bool {
    matches!(name, "search_papers" | "get_paper" | "read_paper")
}

// ---------- requests ----------

static CACHE: LazyLock<Mutex<HashMap<String, (Instant, String)>>> = LazyLock::new(Default::default);
static LAST: LazyLock<Mutex<HashMap<String, Instant>>> = LazyLock::new(Default::default);

/// The gap each host asks for between requests.
fn interval(host: &str) -> Duration {
    match host {
        h if h.ends_with("arxiv.org") => Duration::from_millis(3_000),
        h if h.ends_with("semanticscholar.org") => Duration::from_millis(1_100),
        h if h.ends_with("ncbi.nlm.nih.gov") => Duration::from_millis(340),
        _ => Duration::from_millis(120),
    }
}

/// Waits for this host's turn. Concurrent callers queue behind each other.
async fn pace(host: &str) {
    let wait = {
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        let next = last.get(host).map_or(now, |at| (*at + interval(host)).max(now));
        last.insert(host.to_string(), next);
        next - now
    };
    if !wait.is_zero() {
        tokio::time::sleep(wait).await;
    }
}

/// Requests to academic services follow the same rule as model requests (privacy.rs): none when only this
/// machine is allowed. Only search terms and paper ids leave; secrets in them are refused.
pub fn allowed() -> Result<(), String> {
    if crate::privacy::rule_for(None) == "local" {
        return Err("Your privacy rule keeps every request on this machine, so academic sources are off. Change it in Settings → Privacy.".into());
    }
    Ok(())
}

fn client() -> Result<Client, String> {
    Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(25))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| e.to_string())
}

/// GETs `url` as text, paced and cached.
async fn get_text(url: &Url) -> Result<String, String> {
    allowed()?;
    let key = url.to_string();
    if let Some((at, body)) = CACHE.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        if at.elapsed() < CACHE_TTL {
            return Ok(body.clone());
        }
    }
    let host = url.host_str().unwrap_or("").to_string();
    pace(&host).await;
    let response = client()?.get(url.clone()).send().await.map_err(|e| format!("{host}: {e}"))?;
    let status = response.status();
    if status.as_u16() == 429 {
        return Err(format!("{host} is busy (rate limited); try again in a minute"));
    }
    if !status.is_success() {
        return Err(format!("{host} returned HTTP {status}"));
    }
    let body = response.text().await.map_err(|e| e.to_string())?;
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if cache.len() >= CACHE_ENTRIES {
        if let Some(oldest) = cache.iter().min_by_key(|(_, (at, _))| *at).map(|(key, _)| key.clone()) {
            cache.remove(&oldest);
        }
    }
    cache.insert(key, (Instant::now(), body.clone()));
    Ok(body)
}

async fn get_json(url: &Url) -> Result<Value, String> {
    serde_json::from_str(&get_text(url).await?).map_err(|e| format!("Unexpected reply from {}: {e}", url.host_str().unwrap_or("")))
}

fn url(base: &str, pairs: &[(&str, String)]) -> Url {
    let mut url = Url::parse(base).expect("static base url");
    {
        let mut query = url.query_pairs_mut();
        for (key, value) in pairs {
            if !value.is_empty() {
                query.append_pair(key, value);
            }
        }
    }
    url
}

// ---------- ids ----------

/// What an id the model passes refers to.
#[derive(Debug, PartialEq)]
pub enum PaperId {
    Doi(String),
    Arxiv(String),
    Pmid(String),
    OpenAlex(String),
    SemanticScholar(String),
    Link(String),
}

pub fn parse_id(raw: &str) -> Option<PaperId> {
    let id = raw.trim();
    let lower = id.to_lowercase();
    if let Some(doi) = bare_doi(id) {
        return Some(PaperId::Doi(doi));
    }
    if let Some(rest) = lower.strip_prefix("arxiv:") {
        return Some(PaperId::Arxiv(rest.trim().to_string()));
    }
    if let Some(at) = lower.find("arxiv.org/abs/").or_else(|| lower.find("arxiv.org/pdf/")) {
        return Some(PaperId::Arxiv(id[at + 14..].trim_end_matches(".pdf").trim_end_matches('/').to_string()));
    }
    if let Some(rest) = lower.strip_prefix("pmid:") {
        return Some(PaperId::Pmid(rest.trim().to_string()));
    }
    if let Some(rest) = lower.strip_prefix("openalex:") {
        return Some(PaperId::OpenAlex(rest.trim().to_uppercase()));
    }
    if let Some(at) = lower.find("openalex.org/") {
        return Some(PaperId::OpenAlex(id[at + 13..].trim_start_matches("works/").to_uppercase()));
    }
    if let Some(rest) = id.strip_prefix("s2:") {
        return Some(PaperId::SemanticScholar(rest.trim().to_string()));
    }
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return Some(PaperId::Link(id.to_string()));
    }
    // New-style arXiv ids (2401.01234 or 2401.01234v2).
    let digits = id.split('v').next().unwrap_or("");
    if digits.len() >= 9 && digits.as_bytes()[4] == b'.' && digits.chars().filter(|c| *c != '.').all(|c| c.is_ascii_digit()) {
        return Some(PaperId::Arxiv(id.to_string()));
    }
    if !id.is_empty() && id.len() <= 9 && id.chars().all(|c| c.is_ascii_digit()) {
        return Some(PaperId::Pmid(id.to_string()));
    }
    if id.len() == 40 && id.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some(PaperId::SemanticScholar(id.to_string()));
    }
    if id.len() > 1 && (id.starts_with('W') || id.starts_with('w')) && id[1..].chars().all(|c| c.is_ascii_digit()) {
        return Some(PaperId::OpenAlex(id.to_uppercase()));
    }
    None
}

/// "10.1000/xyz" from a DOI, a doi: id or a doi.org link.
pub fn bare_doi(text: &str) -> Option<String> {
    let lower = text.trim().to_lowercase();
    let start = lower.find("10.")?;
    let prefix = &lower[..start];
    if !(prefix.is_empty() || prefix.ends_with("doi:") || prefix.ends_with("doi.org/") || prefix.ends_with("doi: ")) {
        return None;
    }
    let doi = lower[start..].trim().trim_end_matches(['.', ',', ';']).to_string();
    doi.contains('/').then_some(doi)
}

// ---------- OpenAlex ----------

pub fn parse_openalex_work(work: &Value) -> Option<Paper> {
    let title = work["title"].as_str().or_else(|| work["display_name"].as_str())?.trim().to_string();
    if title.is_empty() {
        return None;
    }
    let doi = work["doi"].as_str().and_then(bare_doi).unwrap_or_default();
    let openalex = work["id"].as_str().unwrap_or("").rsplit('/').next().unwrap_or("").to_string();
    let location = &work["primary_location"];
    let pdf = work["best_oa_location"]["pdf_url"].as_str().or_else(|| work["open_access"]["oa_url"].as_str()).unwrap_or("");
    Some(Paper {
        id: if doi.is_empty() { format!("openalex:{openalex}") } else { format!("doi:{doi}") },
        title: collapse(&title),
        authors: work["authorships"].as_array().into_iter().flatten().filter_map(|item| item["author"]["display_name"].as_str()).map(str::to_string).collect(),
        year: work["publication_year"].as_i64().map(|year| year as i32),
        venue: location["source"]["display_name"].as_str().unwrap_or("").to_string(),
        url: location["landing_page_url"].as_str().map(str::to_string).unwrap_or_else(|| work["id"].as_str().unwrap_or("").to_string()),
        doi,
        pdf_url: pdf.to_string(),
        abstract_text: inverted_abstract(&work["abstract_inverted_index"]),
        cited_by: work["cited_by_count"].as_u64(),
        source: "OpenAlex".into(),
    })
}

pub fn parse_openalex(body: &Value) -> Vec<Paper> {
    body["results"].as_array().into_iter().flatten().filter_map(parse_openalex_work).collect()
}

/// OpenAlex ships abstracts as {word: [positions]}.
fn inverted_abstract(index: &Value) -> String {
    let Some(map) = index.as_object() else { return String::new() };
    let mut words: Vec<(u64, &str)> = map.iter().flat_map(|(word, positions)| positions.as_array().into_iter().flatten().filter_map(|at| at.as_u64()).map(move |at| (at, word.as_str()))).collect();
    words.sort_unstable_by_key(|(at, _)| *at);
    words.into_iter().map(|(_, word)| word).collect::<Vec<_>>().join(" ")
}

async fn search_openalex(query: &Query) -> Result<Vec<Paper>, String> {
    let mut filters = Vec::new();
    if let Some(from) = query.from_year {
        filters.push(format!("from_publication_date:{from}-01-01"));
    }
    if let Some(to) = query.to_year {
        filters.push(format!("to_publication_date:{to}-12-31"));
    }
    let text = if query.field.is_empty() { query.text.clone() } else { format!("{} {}", query.text, query.field) };
    let url = url("https://api.openalex.org/works", &[("search", text), ("per_page", PER_SOURCE.to_string()), ("filter", filters.join(","))]);
    Ok(parse_openalex(&get_json(&url).await?))
}

// ---------- Semantic Scholar ----------

const S2_FIELDS: &str = "title,authors,year,venue,externalIds,url,abstract,citationCount,openAccessPdf";

pub fn parse_s2_paper(item: &Value) -> Option<Paper> {
    let title = item["title"].as_str()?.trim().to_string();
    if title.is_empty() {
        return None;
    }
    let ids = &item["externalIds"];
    let doi = ids["DOI"].as_str().and_then(bare_doi).unwrap_or_default();
    let arxiv = ids["ArXiv"].as_str().unwrap_or("");
    let id = if !doi.is_empty() {
        format!("doi:{doi}")
    } else if !arxiv.is_empty() {
        format!("arxiv:{arxiv}")
    } else {
        format!("s2:{}", item["paperId"].as_str().unwrap_or(""))
    };
    Some(Paper {
        id,
        title: collapse(&title),
        authors: item["authors"].as_array().into_iter().flatten().filter_map(|author| author["name"].as_str()).map(str::to_string).collect(),
        year: item["year"].as_i64().map(|year| year as i32),
        venue: item["venue"].as_str().unwrap_or("").to_string(),
        doi,
        url: item["url"].as_str().unwrap_or("").to_string(),
        pdf_url: item["openAccessPdf"]["url"].as_str().unwrap_or("").to_string(),
        abstract_text: item["abstract"].as_str().map(collapse).unwrap_or_default(),
        cited_by: item["citationCount"].as_u64(),
        source: "Semantic Scholar".into(),
    })
}

pub fn parse_s2_search(body: &Value) -> Vec<Paper> {
    body["data"].as_array().into_iter().flatten().filter_map(parse_s2_paper).collect()
}

async fn search_s2(query: &Query) -> Result<Vec<Paper>, String> {
    let years = match (query.from_year, query.to_year) {
        (Some(from), Some(to)) => format!("{from}-{to}"),
        (Some(from), None) => format!("{from}-"),
        (None, Some(to)) => format!("-{to}"),
        _ => String::new(),
    };
    let url = url("https://api.semanticscholar.org/graph/v1/paper/search", &[("query", query.text.clone()), ("limit", PER_SOURCE.to_string()), ("fields", S2_FIELDS.into()), ("year", years), ("fieldsOfStudy", s2_field(&query.field))]);
    Ok(parse_s2_search(&get_json(&url).await?))
}

/// Semantic Scholar's fieldsOfStudy names, or empty when the field is not one of them.
fn s2_field(field: &str) -> String {
    const FIELDS: [&str; 23] = [
        "Computer Science", "Medicine", "Chemistry", "Biology", "Materials Science", "Physics", "Geology", "Psychology", "Art", "History", "Geography", "Sociology", "Business", "Political Science", "Economics", "Philosophy", "Mathematics", "Engineering", "Environmental Science", "Agricultural and Food Sciences", "Education", "Law", "Linguistics",
    ];
    let field = field.trim().to_lowercase();
    if field.is_empty() {
        return String::new();
    }
    FIELDS.iter().find(|name| name.to_lowercase() == field || name.to_lowercase().starts_with(&field)).map(|name| name.to_string()).unwrap_or_default()
}

// ---------- arXiv ----------

/// arXiv answers in Atom XML; a few tags are all a citation needs.
pub fn parse_arxiv(xml: &str) -> Vec<Paper> {
    xml.split("<entry>")
        .skip(1)
        .filter_map(|entry| {
            let entry = entry.split("</entry>").next().unwrap_or(entry);
            let title = collapse(&xml_text(tag(entry, "title")?));
            let abs = tag(entry, "id").unwrap_or_default();
            let id = abs.rsplit("/abs/").next().unwrap_or("").trim().to_string();
            if title.is_empty() || id.is_empty() {
                return None;
            }
            let bare = id.split('v').next().filter(|_| id.contains('.')).unwrap_or(&id).to_string();
            let doi = tag(entry, "arxiv:doi").and_then(|doi| bare_doi(&doi)).unwrap_or_default();
            let authors = entry.split("<author>").skip(1).filter_map(|author| tag(author, "name")).map(|name| collapse(&xml_text(name))).collect();
            let pdf = entry
                .split("<link")
                .skip(1)
                .map(|link| link.split('>').next().unwrap_or(""))
                .find(|link| link.contains("title=\"pdf\""))
                .and_then(|link| attribute(link, "href"))
                .unwrap_or_else(|| format!("https://arxiv.org/pdf/{id}"));
            Some(Paper {
                id: format!("arxiv:{bare}"),
                title,
                authors,
                year: tag(entry, "published").and_then(|date| date.get(..4).and_then(|year| year.parse().ok())),
                venue: tag(entry, "arxiv:journal_ref").map(|text| collapse(&xml_text(text))).unwrap_or_else(|| "arXiv".into()),
                doi,
                url: format!("https://arxiv.org/abs/{bare}"),
                pdf_url: pdf.replace("http://", "https://"),
                abstract_text: tag(entry, "summary").map(|text| collapse(&xml_text(text))).unwrap_or_default(),
                cited_by: None,
                source: "arXiv".into(),
            })
        })
        .collect()
}

async fn search_arxiv(query: &Query) -> Result<Vec<Paper>, String> {
    let terms = query.text.split_whitespace().map(|word| format!("all:{word}")).collect::<Vec<_>>().join(" AND ");
    let url = url("https://export.arxiv.org/api/query", &[("search_query", terms), ("max_results", PER_SOURCE.to_string()), ("sortBy", "relevance".into())]);
    Ok(parse_arxiv(&get_text(&url).await?).into_iter().filter(|paper| query.in_range(paper)).collect())
}

// ---------- Crossref ----------

pub fn parse_crossref_work(item: &Value) -> Option<Paper> {
    let title = item["title"].as_array().and_then(|titles| titles.first()).and_then(Value::as_str).or_else(|| item["title"].as_str())?.trim().to_string();
    if title.is_empty() {
        return None;
    }
    let doi = item["DOI"].as_str().and_then(bare_doi).unwrap_or_default();
    let year = ["published-print", "published-online", "issued", "created"].iter().find_map(|key| item[key]["date-parts"][0][0].as_i64()).map(|year| year as i32);
    let pdf = item["link"].as_array().into_iter().flatten().find(|link| link["content-type"].as_str() == Some("application/pdf")).and_then(|link| link["URL"].as_str()).unwrap_or("");
    Some(Paper {
        id: format!("doi:{doi}"),
        title: collapse(&xml_text(title)),
        authors: item["author"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|author| match (author["given"].as_str(), author["family"].as_str(), author["name"].as_str()) {
                (Some(given), Some(family), _) => Some(format!("{given} {family}")),
                (None, Some(family), _) => Some(family.to_string()),
                (_, _, Some(name)) => Some(name.to_string()),
                _ => None,
            })
            .collect(),
        year,
        venue: item["container-title"].as_array().and_then(|names| names.first()).and_then(Value::as_str).unwrap_or("").to_string(),
        url: item["URL"].as_str().unwrap_or("").to_string(),
        doi,
        pdf_url: if item["license"].is_array() { pdf.to_string() } else { String::new() },
        abstract_text: item["abstract"].as_str().map(|text| collapse(&strip_tags(text))).unwrap_or_default(),
        cited_by: item["is-referenced-by-count"].as_u64(),
        source: "Crossref".into(),
    })
}

pub fn parse_crossref(body: &Value) -> Vec<Paper> {
    let message = &body["message"];
    match message["items"].as_array() {
        Some(items) => items.iter().filter_map(parse_crossref_work).collect(),
        None => parse_crossref_work(message).into_iter().collect(),
    }
}

async fn search_crossref(query: &Query) -> Result<Vec<Paper>, String> {
    let mut filters = Vec::new();
    if let Some(from) = query.from_year {
        filters.push(format!("from-pub-date:{from}"));
    }
    if let Some(to) = query.to_year {
        filters.push(format!("until-pub-date:{to}"));
    }
    let url = url("https://api.crossref.org/works", &[("query", query.text.clone()), ("rows", PER_SOURCE.to_string()), ("filter", filters.join(","))]);
    Ok(parse_crossref(&get_json(&url).await?))
}

// ---------- PubMed ----------

pub fn parse_pubmed_ids(body: &Value) -> Vec<String> {
    body["esearchresult"]["idlist"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()
}

pub fn parse_pubmed_summary(body: &Value) -> Vec<Paper> {
    let result = &body["result"];
    result["uids"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|uid| {
            let item = &result[uid];
            let title = collapse(&strip_tags(item["title"].as_str()?)).trim_end_matches('.').to_string();
            if title.is_empty() {
                return None;
            }
            let ids = item["articleids"].as_array();
            let id_of = |kind: &str| ids.into_iter().flatten().find(|id| id["idtype"].as_str() == Some(kind)).and_then(|id| id["value"].as_str()).unwrap_or("").to_string();
            let doi = bare_doi(&id_of("doi")).unwrap_or_default();
            let pmc = id_of("pmc");
            Some(Paper {
                id: format!("pmid:{uid}"),
                title,
                authors: item["authors"].as_array().into_iter().flatten().filter(|author| author["authtype"].as_str().is_none_or(|kind| kind == "Author")).filter_map(|author| author["name"].as_str()).map(pubmed_name).collect(),
                year: item["pubdate"].as_str().and_then(|date| date.get(..4)).and_then(|year| year.parse().ok()),
                venue: item["fulljournalname"].as_str().or_else(|| item["source"].as_str()).unwrap_or("").to_string(),
                doi,
                url: format!("https://pubmed.ncbi.nlm.nih.gov/{uid}/"),
                pdf_url: if pmc.is_empty() { String::new() } else { format!("https://www.ncbi.nlm.nih.gov/pmc/articles/{pmc}/") },
                abstract_text: String::new(),
                cited_by: None,
                source: "PubMed".into(),
            })
        })
        .collect()
}

/// PubMed writes "Smith JA"; citations want "J. A. Smith" order, so "JA Smith" → "J A Smith".
fn pubmed_name(name: &str) -> String {
    let mut parts: Vec<&str> = name.split_whitespace().collect();
    match parts.pop() {
        Some(initials) if parts.len() >= 1 && initials.len() <= 3 && initials.chars().all(|c| c.is_ascii_uppercase()) => {
            let given = initials.chars().map(|c| c.to_string()).collect::<Vec<_>>().join(" ");
            format!("{given} {}", parts.join(" "))
        }
        Some(last) => {
            parts.push(last);
            parts.join(" ")
        }
        None => String::new(),
    }
}

async fn search_pubmed(query: &Query) -> Result<Vec<Paper>, String> {
    let (min, max) = (query.from_year.map(|year| year.to_string()).unwrap_or_default(), query.to_year.map(|year| year.to_string()).unwrap_or_default());
    let dated = !min.is_empty() || !max.is_empty();
    let search = url(
        "https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esearch.fcgi",
        &[("db", "pubmed".into()), ("term", query.text.clone()), ("retmode", "json".into()), ("retmax", PER_SOURCE.to_string()), ("sort", "relevance".into()), ("datetype", if dated { "pdat".into() } else { String::new() }), ("mindate", if dated { if min.is_empty() { "1800".into() } else { min } } else { String::new() }), ("maxdate", if dated { if max.is_empty() { "3000".into() } else { max } } else { String::new() })],
    );
    let ids = parse_pubmed_ids(&get_json(&search).await?);
    pubmed_records(&ids).await
}

async fn pubmed_records(ids: &[String]) -> Result<Vec<Paper>, String> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let summary = url("https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esummary.fcgi", &[("db", "pubmed".into()), ("id", ids.join(",")), ("retmode", "json".into())]);
    Ok(parse_pubmed_summary(&get_json(&summary).await?))
}

async fn pubmed_abstract(pmid: &str) -> Result<String, String> {
    let url = url("https://eutils.ncbi.nlm.nih.gov/entrez/eutils/efetch.fcgi", &[("db", "pubmed".into()), ("id", pmid.to_string()), ("rettype", "abstract".into()), ("retmode", "text".into())]);
    Ok(get_text(&url).await?.trim().to_string())
}

// ---------- Unpaywall ----------

/// The best open-access link Unpaywall knows for a DOI: a PDF when there is one, else the landing page.
pub fn parse_unpaywall(body: &Value) -> Option<String> {
    let best = &body["best_oa_location"];
    best["url_for_pdf"].as_str().or_else(|| best["url"].as_str()).filter(|link| !link.is_empty()).map(str::to_string)
}

/// Unpaywall requires a real contact address with each request, so it is asked only when one is set in
/// NERU_CONTACT_EMAIL. OpenAlex already carries Unpaywall's open-access links for most papers.
async fn unpaywall(doi: &str) -> Option<String> {
    let contact = std::env::var("NERU_CONTACT_EMAIL").ok().filter(|email| email.contains('@'))?;
    let url = url(&format!("https://api.unpaywall.org/v2/{doi}"), &[("email", contact)]);
    parse_unpaywall(&get_json(&url).await.ok()?)
}

// ---------- search, look up, read ----------

fn title_key(title: &str) -> String {
    title.to_lowercase().chars().filter(|c| c.is_alphanumeric()).take(80).collect()
}

/// Merges result lists, one record per paper (same DOI or same title), keeping each list's order and
/// taking turns between lists so no single source crowds out the rest.
pub fn merge(lists: Vec<Vec<Paper>>, limit: usize) -> Vec<Paper> {
    let mut merged: Vec<Paper> = Vec::new();
    let longest = lists.iter().map(Vec::len).max().unwrap_or(0);
    for rank in 0..longest {
        for list in &lists {
            let Some(paper) = list.get(rank) else { continue };
            let key = title_key(&paper.title);
            match merged.iter_mut().find(|known| (!paper.doi.is_empty() && known.doi == paper.doi) || title_key(&known.title) == key) {
                Some(known) => known.merge(paper),
                None => merged.push(paper.clone()),
            }
        }
    }
    merged.truncate(limit);
    merged
}

/// Searches every source that fits the query at once. Fails only when all of them fail.
pub async fn search_papers(query: &Query) -> Result<Vec<Paper>, String> {
    if query.text.is_empty() || query.text.len() > 400 {
        return Err("query must be 1–400 characters".into());
    }
    allowed()?;
    if !crate::privacy::find_secrets(&query.text).is_empty() {
        return Err("The query looks like it contains a key or password; search without it".into());
    }
    let medical = query.medical();
    let (openalex, s2, arxiv, pubmed) = tokio::join!(search_openalex(query), search_s2(query), search_arxiv(query), async {
        if medical { search_pubmed(query).await } else { Ok(Vec::new()) }
    });
    let mut errors = Vec::new();
    let mut lists = Vec::new();
    for result in [openalex, s2, pubmed, arxiv] {
        match result {
            Ok(list) => lists.push(list.into_iter().filter(|paper| query.in_range(paper)).collect()),
            Err(error) => errors.push(error),
        }
    }
    if lists.iter().all(Vec::is_empty) {
        // Crossref covers almost every DOI; it is the fallback when the others are down or empty.
        match search_crossref(query).await {
            Ok(list) => lists.push(list.into_iter().filter(|paper| query.in_range(paper)).collect()),
            Err(error) => errors.push(error),
        }
    }
    let papers = merge(lists, 10);
    if papers.is_empty() && !errors.is_empty() {
        return Err(format!("No academic source answered: {}", errors.join("; ")));
    }
    Ok(papers)
}

/// One paper's full record.
pub async fn get_paper(raw: &str) -> Result<Paper, String> {
    allowed()?;
    let id = parse_id(raw).ok_or_else(|| format!("Not a paper id: {raw}. Use a DOI, arXiv id, pmid:…, or an id from search_papers."))?;
    let mut paper = match &id {
        PaperId::Doi(doi) => {
            let openalex = Url::parse(&format!("https://api.openalex.org/works/doi:{doi}")).map_err(|e| e.to_string())?;
            match get_json(&openalex).await.ok().as_ref().and_then(parse_openalex_work) {
                Some(paper) => paper,
                None => {
                    let crossref = Url::parse(&format!("https://api.crossref.org/works/{doi}")).map_err(|e| e.to_string())?;
                    parse_crossref(&get_json(&crossref).await?).into_iter().next().ok_or("No record for this DOI")?
                }
            }
        }
        PaperId::Arxiv(arxiv) => {
            let url = url("https://export.arxiv.org/api/query", &[("id_list", arxiv.clone())]);
            parse_arxiv(&get_text(&url).await?).into_iter().next().ok_or("No arXiv paper with this id")?
        }
        PaperId::Pmid(pmid) => pubmed_records(std::slice::from_ref(pmid)).await?.into_iter().next().ok_or("No PubMed record with this id")?,
        PaperId::OpenAlex(work) => {
            let url = Url::parse(&format!("https://api.openalex.org/works/{work}")).map_err(|e| e.to_string())?;
            parse_openalex_work(&get_json(&url).await?).ok_or("No OpenAlex record with this id")?
        }
        PaperId::SemanticScholar(s2) => {
            let url = url(&format!("https://api.semanticscholar.org/graph/v1/paper/{s2}"), &[("fields", S2_FIELDS.into())]);
            parse_s2_paper(&get_json(&url).await?).ok_or("No Semantic Scholar record with this id")?
        }
        PaperId::Link(link) => return Err(format!("{link} is a link, not a paper id; use read_paper or fetch_url for it")),
    };
    if paper.abstract_text.is_empty() {
        if let PaperId::Pmid(pmid) = &id {
            paper.abstract_text = pubmed_abstract(pmid).await.unwrap_or_default();
        } else if !paper.doi.is_empty() {
            // Semantic Scholar often has the abstract when OpenAlex and Crossref do not.
            let url = url(&format!("https://api.semanticscholar.org/graph/v1/paper/DOI:{}", paper.doi), &[("fields", S2_FIELDS.into())]);
            if let Some(other) = get_json(&url).await.ok().as_ref().and_then(parse_s2_paper) {
                paper.merge(&other);
            }
        }
    }
    Ok(paper)
}

/// A paper's open-access full text, with the record it belongs to (None when given a bare link).
pub async fn read_paper(raw: &str) -> Result<(Option<Paper>, String, String), String> {
    allowed()?;
    let (paper, mut links) = match parse_id(raw) {
        Some(PaperId::Link(link)) if !link.contains("arxiv.org/") => (None, vec![link]),
        Some(_) => {
            let paper = get_paper(raw).await?;
            let mut links = Vec::new();
            if paper.id.starts_with("arxiv:") {
                let id = paper.id.trim_start_matches("arxiv:");
                // arXiv's HTML rendering is lighter than its PDF when it exists.
                links.push(format!("https://arxiv.org/html/{id}"));
                links.push(format!("https://arxiv.org/pdf/{id}"));
            }
            if !paper.pdf_url.is_empty() {
                links.push(paper.pdf_url.clone());
            }
            if !paper.doi.is_empty() {
                if let Some(link) = unpaywall(&paper.doi).await {
                    links.push(link);
                }
            }
            (Some(paper), links)
        }
        None => return Err(format!("Not a paper id or link: {raw}")),
    };
    links.dedup();
    if links.is_empty() {
        return Err("No open-access full text is known for this paper. Work from its abstract (get_paper), and say so.".into());
    }
    let mut errors = Vec::new();
    for link in links {
        match full_text(&link).await {
            Ok(text) if text.trim().len() > 500 => return Ok((paper, link, text)),
            Ok(_) => errors.push(format!("{link}: no readable text")),
            Err(error) => errors.push(format!("{link}: {error}")),
        }
    }
    Err(format!("Could not read the full text ({}). Work from the abstract and say so.", errors.join("; ")))
}

/// Text of a PDF or HTML page at `link`.
async fn full_text(link: &str) -> Result<String, String> {
    let parsed = Url::parse(link).map_err(|_| "invalid link")?;
    if !matches!(parsed.scheme(), "http" | "https") || web::blocked_host(&parsed) {
        return Err("only public http(s) links".into());
    }
    pace(parsed.host_str().unwrap_or("")).await;
    let mut response = client()?.get(parsed).send().await.map_err(|e| e.to_string())?;
    if web::blocked_host(response.url()) {
        return Err("redirected to a private address".into());
    }
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    let kind = response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|value| value.to_str().ok()).unwrap_or("").to_lowercase();
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > MAX_PDF_BYTES {
            return Err("larger than 25 MB".into());
        }
    }
    if kind.contains("pdf") || bytes.starts_with(b"%PDF") {
        return pdf_text(&bytes).await;
    }
    if kind.contains("html") || kind.contains("text") || kind.contains("xml") || kind.is_empty() {
        return Ok(web::html_to_text(&String::from_utf8_lossy(&bytes)));
    }
    Err(format!("unsupported content type {kind}"))
}

// ---------- PDF text, read in the window ----------

static APP: OnceLock<AppHandle> = OnceLock::new();
type Reply = tokio::sync::oneshot::Sender<Result<String, String>>;
static WAITING: LazyLock<Mutex<HashMap<String, Reply>>> = LazyLock::new(Default::default);

pub fn set_app(app: AppHandle) {
    let _ = APP.set(app);
}

/// Saves the PDF under the data folder and asks the window to extract its text with PDF.js.
async fn pdf_text(bytes: &[u8]) -> Result<String, String> {
    let app = APP.get().ok_or("PDFs can only be read in the Neru app")?;
    let digest = <sha2::Sha256 as sha2::Digest>::digest(bytes);
    let name: String = digest.iter().take(10).map(|byte| format!("{byte:02x}")).collect();
    let dir = crate::workspace::data_dir()?.join("papers");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{name}.pdf"));
    if !path.exists() {
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    }
    let (send, receive) = tokio::sync::oneshot::channel();
    WAITING.lock().unwrap_or_else(|e| e.into_inner()).insert(name.clone(), send);
    let _ = app.emit("scholar://pdf", serde_json::json!({ "requestId": name, "path": path.display().to_string() }));
    let reply = tokio::time::timeout(Duration::from_secs(90), receive).await;
    WAITING.lock().unwrap_or_else(|e| e.into_inner()).remove(&name);
    match reply {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err("the window closed".into()),
        Err(_) => Err("reading the PDF took too long".into()),
    }
}

/// The window's answer to a scholar://pdf request.
#[tauri::command]
pub fn scholar_pdf_text(request_id: String, text: Option<String>, error: Option<String>) {
    if let Some(reply) = WAITING.lock().unwrap_or_else(|e| e.into_inner()).remove(&request_id) {
        let _ = reply.send(match (text, error) {
            (Some(text), _) => Ok(text),
            (None, error) => Err(error.unwrap_or_else(|| "no text".into())),
        });
    }
}

// ---------- tool results ----------

/// Numbers each paper as a source and lists them for the model.
pub fn cite_papers(sources: &mut Vec<Source>, papers: &[Paper], abstract_chars: usize) -> String {
    if papers.is_empty() {
        return "No papers found. Try broader keywords, synonyms, or no year range.".into();
    }
    papers.iter().map(|paper| paper.describe(cite_paper(sources, paper), abstract_chars)).collect::<Vec<_>>().join("\n\n")
}

/// Adds a paper once (matched by link) and returns its 1-based number.
pub fn cite_paper(sources: &mut Vec<Source>, paper: &Paper) -> usize {
    let link = paper.link();
    if let Some(position) = sources.iter().position(|source| source.url == link) {
        let known = &mut sources[position];
        if known.authors.is_empty() {
            *known = paper.to_source(position + 1);
        }
        return position + 1;
    }
    sources.push(paper.to_source(sources.len() + 1));
    sources.len()
}

// ---------- text helpers ----------

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strip_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_tag = false;
    for ch in text.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    xml_text(out)
}

fn xml_text(text: impl AsRef<str>) -> String {
    let text = text.as_ref();
    let text = text.strip_prefix("<![CDATA[").and_then(|rest| rest.strip_suffix("]]>")).unwrap_or(text);
    text.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&#39;", "'").replace("&amp;", "&")
}

/// The text inside the first <name …>…</name> in `xml`.
fn tag(xml: &str, name: &str) -> Option<String> {
    let mut rest = xml;
    loop {
        let start = rest.find(&format!("<{name}"))?;
        let after = &rest[start + name.len() + 1..];
        // Skip longer tag names that share the prefix (<title> vs <titles>).
        if after.starts_with(['>', ' ', '/', '\n', '\t']) {
            let open = after.find('>')?;
            if after[..open].ends_with('/') {
                return Some(String::new());
            }
            let body = &after[open + 1..];
            let end = body.find(&format!("</{name}>"))?;
            return Some(body[..end].trim().to_string());
        }
        rest = after;
    }
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let marker = format!("{name}=\"");
    let start = tag.find(&marker)? + marker.len();
    let end = tag[start..].find('"')? + start;
    Some(xml_text(&tag[start..end]))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scholar").join(name)).expect("fixture")
    }

    fn json(name: &str) -> Value {
        serde_json::from_str(&fixture(name)).expect("fixture json")
    }

    #[test]
    fn parses_openalex_with_inverted_abstract() {
        let papers = parse_openalex(&json("openalex.json"));
        assert_eq!(papers.len(), 2);
        let first = &papers[0];
        assert_eq!(first.id, "doi:10.1038/s41586-021-03819-2");
        assert_eq!(first.title, "Highly accurate protein structure prediction with AlphaFold");
        assert_eq!(first.authors, vec!["John Jumper", "Richard Evans"]);
        assert_eq!(first.year, Some(2021));
        assert_eq!(first.venue, "Nature");
        assert_eq!(first.cited_by, Some(25000));
        assert_eq!(first.abstract_text, "Proteins are essential to life");
        assert_eq!(first.pdf_url, "https://www.nature.com/articles/s41586-021-03819-2.pdf");
        // No DOI: the OpenAlex id stands in.
        assert_eq!(papers[1].id, "openalex:W2000000002");
        assert!(papers[1].abstract_text.is_empty());
    }

    #[test]
    fn parses_semantic_scholar() {
        let papers = parse_s2_search(&json("semantic_scholar.json"));
        assert_eq!(papers.len(), 2);
        assert_eq!(papers[0].id, "doi:10.48550/arxiv.1706.03762");
        assert_eq!(papers[0].authors[0], "Ashish Vaswani");
        assert_eq!(papers[0].year, Some(2017));
        assert_eq!(papers[0].pdf_url, "https://arxiv.org/pdf/1706.03762");
        assert_eq!(papers[1].id, "arxiv:1810.04805");
        assert_eq!(papers[1].abstract_text, "We introduce BERT.");
    }

    #[test]
    fn parses_arxiv_atom() {
        let papers = parse_arxiv(&fixture("arxiv.xml"));
        assert_eq!(papers.len(), 1);
        let paper = &papers[0];
        assert_eq!(paper.id, "arxiv:1706.03762");
        assert_eq!(paper.title, "Attention Is All You Need");
        assert_eq!(paper.authors, vec!["Ashish Vaswani", "Noam Shazeer"]);
        assert_eq!(paper.year, Some(2017));
        assert_eq!(paper.venue, "arXiv");
        assert_eq!(paper.pdf_url, "https://arxiv.org/pdf/1706.03762v7");
        assert_eq!(paper.url, "https://arxiv.org/abs/1706.03762");
        assert!(paper.abstract_text.starts_with("The dominant sequence transduction models"));
        assert!(paper.abstract_text.contains("encoder & decoder"));
    }

    #[test]
    fn parses_crossref_single_and_list() {
        let single = parse_crossref(&json("crossref_work.json"));
        assert_eq!(single.len(), 1);
        let paper = &single[0];
        assert_eq!(paper.doi, "10.1037/0003-066x.59.1.29");
        assert_eq!(paper.authors, vec!["Jane Doe", "Alan Smith"]);
        assert_eq!(paper.year, Some(2004));
        assert_eq!(paper.venue, "American Psychologist");
        assert_eq!(paper.abstract_text, "A short abstract.");
        assert_eq!(paper.cited_by, Some(12));
        let list = parse_crossref(&serde_json::json!({"message":{"items":[{"title":["A"],"DOI":"10.1/a"},{"title":[]}]}}));
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn parses_pubmed_search_and_summary() {
        assert_eq!(parse_pubmed_ids(&json("pubmed_esearch.json")), vec!["31452104", "30000000"]);
        let papers = parse_pubmed_summary(&json("pubmed_esummary.json"));
        assert_eq!(papers.len(), 1);
        let paper = &papers[0];
        assert_eq!(paper.id, "pmid:31452104");
        assert_eq!(paper.title, "Sleep and memory consolidation");
        assert_eq!(paper.authors, vec!["J A Smith", "B Lee"]);
        assert_eq!(paper.year, Some(2019));
        assert_eq!(paper.venue, "Nature reviews. Neuroscience");
        assert_eq!(paper.doi, "10.1038/s41583-019-0201-1");
        assert_eq!(paper.pdf_url, "https://www.ncbi.nlm.nih.gov/pmc/articles/PMC6700000/");
    }

    #[test]
    fn parses_unpaywall() {
        assert_eq!(parse_unpaywall(&json("unpaywall.json")).as_deref(), Some("https://example.org/paper.pdf"));
        assert_eq!(parse_unpaywall(&serde_json::json!({"best_oa_location": null})), None);
    }

    #[test]
    fn reads_ids_in_every_form() {
        assert_eq!(parse_id("https://doi.org/10.1038/ABC.1"), Some(PaperId::Doi("10.1038/abc.1".into())));
        assert_eq!(parse_id("doi:10.1/x"), Some(PaperId::Doi("10.1/x".into())));
        assert_eq!(parse_id("arXiv:1706.03762"), Some(PaperId::Arxiv("1706.03762".into())));
        assert_eq!(parse_id("1706.03762v7"), Some(PaperId::Arxiv("1706.03762v7".into())));
        assert_eq!(parse_id("https://arxiv.org/abs/1706.03762"), Some(PaperId::Arxiv("1706.03762".into())));
        assert_eq!(parse_id("pmid:31452104"), Some(PaperId::Pmid("31452104".into())));
        assert_eq!(parse_id("31452104"), Some(PaperId::Pmid("31452104".into())));
        assert_eq!(parse_id("openalex:W123"), Some(PaperId::OpenAlex("W123".into())));
        assert_eq!(parse_id("https://example.org/a.pdf"), Some(PaperId::Link("https://example.org/a.pdf".into())));
        assert_eq!(parse_id("hello world"), None);
    }

    #[test]
    fn merges_the_same_paper_from_two_sources() {
        let a = Paper { title: "Attention Is All You Need".into(), doi: "10.1/x".into(), source: "OpenAlex".into(), ..Default::default() };
        let b = Paper { title: "Attention is all you need.".into(), abstract_text: "abs".into(), pdf_url: "https://arxiv.org/pdf/1".into(), ..Default::default() };
        let c = Paper { title: "Other".into(), ..Default::default() };
        let merged = merge(vec![vec![a], vec![b, c]], 10);
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].abstract_text, "abs");
        assert_eq!(merged[0].pdf_url, "https://arxiv.org/pdf/1");
    }

    #[test]
    fn query_reads_years_and_filters() {
        let query = Query::from_args(&serde_json::json!({"query":" sleep ","from_year":2018,"to_year":"2020","field":"Medicine"}));
        assert_eq!((query.text.as_str(), query.from_year, query.to_year), ("sleep", Some(2018), Some(2020)));
        assert!(query.medical());
        assert!(query.in_range(&Paper { year: Some(2019), ..Default::default() }));
        assert!(!query.in_range(&Paper { year: Some(2021), ..Default::default() }));
        assert_eq!(s2_field("computer"), "Computer Science");
        assert_eq!(s2_field("astrology"), "");
    }

    #[test]
    fn cites_papers_as_structured_sources() {
        let mut sources = Vec::new();
        let paper = Paper { title: "T".into(), authors: vec!["Jane Doe".into()], year: Some(2020), venue: "V".into(), doi: "10.1/x".into(), ..Default::default() };
        assert_eq!(cite_paper(&mut sources, &paper), 1);
        assert_eq!(cite_paper(&mut sources, &paper), 1);
        assert_eq!(sources[0].url, "https://doi.org/10.1/x");
        assert_eq!(sources[0].doi, "10.1/x");
        assert_eq!(sources[0].year, Some(2020));
        let text = cite_papers(&mut sources, &[paper], 100);
        assert!(text.starts_with("[1] T\nJane Doe\n2020 · V"), "{text}");
    }
}
