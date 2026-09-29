use std::{net::IpAddr, time::Duration};

use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36 Neru/0.1";
const MAX_PAGE_BYTES: usize = 2_000_000;

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub id: String,
    pub title: String,
    pub url: String,
    pub domain: String,
}

pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

pub fn domain(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|url| {
            url.host_str()
                .map(|host| host.trim_start_matches("www.").to_string())
        })
        .unwrap_or_default()
}

/// Adds a source once and returns its 1-based citation number.
pub fn cite(sources: &mut Vec<Source>, title: &str, url: &str) -> usize {
    if let Some(position) = sources.iter().position(|source| source.url == url) {
        return position + 1;
    }
    sources.push(Source {
        id: format!("source-{}", sources.len() + 1),
        title: if title.trim().is_empty() {
            domain(url)
        } else {
            title.trim().to_string()
        },
        url: url.to_string(),
        domain: domain(url),
    });
    sources.len()
}

fn client() -> Result<Client, String> {
    Client::builder()
        .user_agent(USER_AGENT)
        .timeout(Duration::from_secs(20))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| e.to_string())
}

pub async fn search(query: &str) -> Result<Vec<SearchHit>, String> {
    let query = query.trim();
    if query.is_empty() || query.len() > 400 {
        return Err("Search query must be 1–400 characters".into());
    }
    let mut url = Url::parse("https://html.duckduckgo.com/html/").unwrap();
    url.query_pairs_mut().append_pair("q", query);
    let response = client()?
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Web search failed: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("Web search returned HTTP {}", response.status()));
    }
    let html = response.text().await.map_err(|e| e.to_string())?;
    let hits = parse_duckduckgo(&html);
    if hits.is_empty() && html.contains("anomaly") {
        return Err("The search provider asked for verification; try again shortly".into());
    }
    Ok(hits)
}

fn parse_duckduckgo(html: &str) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    let mut rest = html;
    while let Some(start) = rest.find("class=\"result__a\"") {
        rest = &rest[start..];
        let tag_end = rest.find('>').unwrap_or(rest.len());
        let tag = &rest[..tag_end];
        let href = attribute(tag, "href").unwrap_or_default();
        let close = rest.find("</a>").unwrap_or(rest.len());
        let title = clean_text(&rest[tag_end.min(close)..close].trim_start_matches('>'));
        rest = &rest[close..];
        let next_result = rest.find("class=\"result__a\"").unwrap_or(rest.len());
        let snippet = rest[..next_result]
            .find("class=\"result__snippet\"")
            .map(|at| {
                let block = &rest[at..next_result];
                let open = block.find('>').map_or(0, |i| i + 1);
                let end = block.find("</a>").unwrap_or(block.len());
                clean_text(&block[open.min(end)..end])
            })
            .unwrap_or_default();
        let Some(url) = result_url(&decode_entities(&href)) else {
            continue;
        };
        if hits.iter().any(|hit: &SearchHit| hit.url == url) {
            continue;
        }
        hits.push(SearchHit {
            title,
            url,
            snippet,
        });
        if hits.len() == 8 {
            break;
        }
    }
    hits
}

fn attribute(tag: &str, name: &str) -> Option<String> {
    let marker = format!("{name}=\"");
    let start = tag.find(&marker)? + marker.len();
    let end = tag[start..].find('"')? + start;
    Some(tag[start..end].to_string())
}

fn result_url(href: &str) -> Option<String> {
    let absolute = if href.starts_with("//") {
        format!("https:{href}")
    } else {
        href.to_string()
    };
    let url = Url::parse(&absolute).ok()?;
    if url
        .host_str()
        .is_some_and(|host| host.ends_with("duckduckgo.com"))
    {
        if url.path().starts_with("/y.js") {
            return None;
        }
        let target = url
            .query_pairs()
            .find(|(key, _)| key == "uddg")?
            .1
            .to_string();
        return Url::parse(&target)
            .ok()
            .filter(|u| matches!(u.scheme(), "http" | "https"))
            .map(|u| u.to_string());
    }
    matches!(url.scheme(), "http" | "https").then(|| url.to_string())
}

fn blocked_host(url: &Url) -> bool {
    let Some(host) = url.host_str() else {
        return true;
    };
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_lowercase();
    if host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
    {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
        }
        Ok(IpAddr::V6(ip)) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || (ip.segments()[0] & 0xfe00) == 0xfc00
                || (ip.segments()[0] & 0xffc0) == 0xfe80
        }
        Err(_) => false,
    }
}

/// Opens an http(s) link in the system browser instead of navigating the app window.
#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    let parsed = Url::parse(url.trim()).map_err(|_| "Invalid link")?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Only http and https links can be opened".into());
    }
    let target = parsed.to_string();
    #[cfg(windows)]
    let mut command = {
        let mut command = std::process::Command::new("rundll32.exe");
        command.args(["url.dll,FileProtocolHandler", &target]);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = std::process::Command::new("open");
        command.arg(&target);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(&target);
        command
    };
    command.spawn().map(|_| ()).map_err(|e| e.to_string())
}

pub struct Page {
    pub title: String,
    pub url: String,
    pub text: String,
}

pub async fn fetch(url: &str) -> Result<Page, String> {
    let parsed = Url::parse(url.trim()).map_err(|_| "Enter a full http or https URL")?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("Only http and https pages can be fetched".into());
    }
    if blocked_host(&parsed) {
        return Err("Local and private network addresses cannot be fetched".into());
    }
    let mut response = client()?
        .get(parsed.clone())
        .send()
        .await
        .map_err(|e| format!("Fetch failed: {e}"))?;
    let final_url = response.url().clone();
    if blocked_host(&final_url) {
        return Err("The page redirected to a private network address".into());
    }
    if !response.status().is_success() {
        return Err(format!("Page returned HTTP {}", response.status()));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_lowercase();
    if !(content_type.is_empty()
        || content_type.contains("text")
        || content_type.contains("json")
        || content_type.contains("xml"))
    {
        return Err(format!("Unsupported content type: {content_type}"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        bytes.extend_from_slice(&chunk);
        if bytes.len() > MAX_PAGE_BYTES {
            break;
        }
    }
    let raw = String::from_utf8_lossy(&bytes).to_string();
    let (title, text) = if content_type.contains("html") || raw.trim_start().starts_with('<') {
        let lower = raw.to_ascii_lowercase();
        let title = lower
            .find("<title")
            .and_then(|start| {
                let open = raw[start..].find('>')? + start + 1;
                let end = lower[open..].find("</title>")? + open;
                Some(clean_text(&raw[open..end]))
            })
            .unwrap_or_default();
        (title, html_to_text(&raw))
    } else {
        (String::new(), raw)
    };
    Ok(Page {
        title,
        url: final_url.to_string(),
        text: text.chars().take(20_000).collect(),
    })
}

fn html_to_text(html: &str) -> String {
    let mut output = String::with_capacity(html.len() / 3);
    let lower = html.to_ascii_lowercase();
    let mut index = 0;
    while index < html.len() {
        let Some(offset) = html[index..].find('<') else {
            output.push_str(&html[index..]);
            break;
        };
        output.push_str(&html[index..index + offset]);
        index += offset;
        let skip_block = ["script", "style", "noscript", "svg", "head"]
            .into_iter()
            .find(|tag| lower[index + 1..].starts_with(tag));
        if let Some(tag) = skip_block {
            let close = format!("</{tag}");
            index = lower[index..]
                .find(&close)
                .map_or(html.len(), |end| index + end);
        }
        let end = html[index..]
            .find('>')
            .map_or(html.len(), |end| index + end + 1);
        let tag = &lower[index..end.min(lower.len())];
        if [
            "<p", "<br", "<div", "<li", "<h1", "<h2", "<h3", "<h4", "<tr", "</p", "</div", "</li",
            "</h",
        ]
        .iter()
        .any(|block| tag.starts_with(block))
        {
            output.push('\n');
        }
        index = end;
    }
    let decoded = decode_entities(&output);
    let mut text = String::new();
    let mut blank = false;
    for line in decoded.lines() {
        let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if line.is_empty() {
            if !blank && !text.is_empty() {
                text.push('\n');
            }
            blank = true;
        } else {
            text.push_str(&line);
            text.push('\n');
            blank = false;
        }
    }
    text
}

fn clean_text(fragment: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    for ch in fragment.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }
    decode_entities(&text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn decode_entities(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        output.push_str(&rest[..start]);
        rest = &rest[start..];
        let Some(end) = rest[..rest.len().min(12)].find(';') else {
            output.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" | "#x27" => Some('\''),
            "nbsp" => Some(' '),
            _ if entity.starts_with("#x") => u32::from_str_radix(&entity[2..], 16)
                .ok()
                .and_then(char::from_u32),
            _ if entity.starts_with('#') => entity[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match decoded {
            Some(ch) => {
                output.push(ch);
                rest = &rest[end + 1..];
            }
            None => {
                output.push('&');
                rest = &rest[1..];
            }
        }
    }
    output.push_str(rest);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_duckduckgo_results() {
        let html = r#"<div><a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fmotion.dev%2Fdocs&amp;rut=x">Motion <b>docs</b></a>
            <a class="result__snippet" href="x">Animate &amp; transition</a></div>
            <a class="result__a" href="https://duckduckgo.com/y.js?ad=1">Ad</a>"#;
        let hits = parse_duckduckgo(html);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].url, "https://motion.dev/docs");
        assert_eq!(hits[0].title, "Motion docs");
        assert_eq!(hits[0].snippet, "Animate & transition");
    }

    #[test]
    fn cites_each_url_once() {
        let mut sources = Vec::new();
        assert_eq!(cite(&mut sources, "A", "https://www.a.com/x"), 1);
        assert_eq!(cite(&mut sources, "B", "https://b.com"), 2);
        assert_eq!(cite(&mut sources, "A again", "https://www.a.com/x"), 1);
        assert_eq!(sources[0].domain, "a.com");
    }

    #[test]
    fn blocks_private_hosts_and_strips_markup() {
        assert!(blocked_host(&Url::parse("http://127.0.0.1:3000").unwrap()));
        assert!(blocked_host(&Url::parse("http://192.168.1.4").unwrap()));
        assert!(blocked_host(&Url::parse("http://localhost").unwrap()));
        assert!(!blocked_host(&Url::parse("https://example.com").unwrap()));
        let text = html_to_text(
            "<html><head><title>x</title></head><body><script>bad()</script><p>Hello&nbsp;<b>world</b></p><p>Next</p></body></html>",
        );
        assert!(text.contains("Hello world"));
        assert!(text.contains("Next"));
        assert!(!text.contains("bad()"));
    }
}
