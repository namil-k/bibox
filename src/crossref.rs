use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct CrossrefResponse {
    message: CrossrefMessage,
}

#[derive(Debug, Deserialize)]
struct CrossrefMessage {
    title: Option<Vec<String>>,
    author: Option<Vec<CrossrefAuthor>>,
    #[serde(rename = "published-print")]
    published_print: Option<CrossrefDate>,
    #[serde(rename = "published-online")]
    published_online: Option<CrossrefDate>,
    #[serde(rename = "container-title")]
    container_title: Option<Vec<String>>,
    #[serde(rename = "publisher")]
    publisher: Option<String>,
    volume: Option<String>,
    issue: Option<String>,
    page: Option<String>,
    #[serde(rename = "event")]
    event: Option<CrossrefEvent>,
    #[serde(rename = "type")]
    entry_type: Option<String>,
    #[serde(rename = "DOI")]
    doi: Option<String>,
    #[serde(rename = "URL")]
    url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CrossrefAuthor {
    given: Option<String>,
    family: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CrossrefDate {
    #[serde(rename = "date-parts")]
    date_parts: Option<Vec<Vec<i32>>>,
}

#[derive(Debug, Deserialize)]
struct CrossrefEvent {
    name: Option<String>,
}

#[derive(Debug)]
pub struct Metadata {
    pub title: Option<String>,
    pub authors: Vec<String>,
    pub year: Option<u32>,
    pub journal: Option<String>,
    pub publisher: Option<String>,
    pub volume: Option<String>,
    pub number: Option<String>,
    pub pages: Option<String>,
    pub booktitle: Option<String>,
    pub doi: String,
    pub url: Option<String>,
    pub entry_type: String,
}

pub async fn fetch_metadata(doi: &str) -> Result<Metadata> {
    let client = reqwest::Client::new();
    let url = format!("https://api.crossref.org/works/{}", doi);

    let resp = client
        .get(&url)
        .header(
            "User-Agent",
            "bibox/0.1 (https://github.com/user/bibox; mailto:user@example.com)",
        )
        .send()
        .await
        .context("Crossref API request failed")?;

    if !resp.status().is_success() {
        anyhow::bail!("DOI not found: {} (HTTP {})", doi, resp.status());
    }

    let data: CrossrefResponse = resp.json().await.context("Crossref response parse failed")?;
    let msg = data.message;

    let title = msg.title.and_then(|v| v.into_iter().next());

    let authors: Vec<String> = msg
        .author
        .unwrap_or_default()
        .into_iter()
        .map(|a| {
            match (a.family, a.given) {
                (Some(f), Some(g)) => format!("{}, {}", f, g),
                (Some(f), None) => f,
                (None, Some(g)) => g,
                (None, None) => String::from("Unknown"),
            }
        })
        .collect();

    let year = msg
        .published_print
        .as_ref()
        .or(msg.published_online.as_ref())
        .and_then(|d| d.date_parts.as_ref())
        .and_then(|dp| dp.first())
        .and_then(|parts| parts.first())
        .map(|&y| y as u32);

    let entry_type = match msg.entry_type.as_deref().unwrap_or("") {
        "journal-article" => "article",
        "book" | "book-chapter" => "book",
        "proceedings-article" => "inproceedings",
        _ => "misc",
    }
    .to_string();

    let container_first = msg.container_title.and_then(|v| v.into_iter().next());

    let journal = if entry_type == "article" {
        container_first.clone()
    } else {
        None
    };

    let booktitle = if entry_type == "inproceedings" {
        msg.event
            .and_then(|e| e.name)
            .or(container_first)
    } else {
        None
    };

    Ok(Metadata {
        title,
        authors,
        year,
        journal,
        publisher: msg.publisher,
        volume: msg.volume,
        number: msg.issue,
        pages: msg.page,
        booktitle,
        doi: msg.doi.unwrap_or_else(|| doi.to_string()),
        url: msg.url,
        entry_type,
    })
}

// ── Crossref search ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CrossrefSearchResponse {
    message: CrossrefSearchMessage,
}

#[derive(Debug, Deserialize)]
struct CrossrefSearchMessage {
    items: Vec<CrossrefMessage>,
}

#[derive(Debug)]
pub struct SearchResult {
    pub doi: String,
    pub title: String,
    pub authors: Vec<String>,
    pub year: Option<u32>,
    pub venue: Option<String>,
}

impl SearchResult {
    /// Format for interactive display: "Author (Year) Title — Venue"
    pub fn display(&self, max_title: usize, max_venue: usize) -> String {
        let author = if self.authors.is_empty() {
            "Unknown".to_string()
        } else {
            let first = self.authors[0]
                .split(',')
                .next()
                .unwrap_or(&self.authors[0])
                .trim()
                .to_string();
            if self.authors.len() > 1 {
                format!("{} et al.", first)
            } else {
                first
            }
        };
        let year = self.year.map(|y| y.to_string()).unwrap_or_else(|| "n.d.".into());
        let title = if self.title.chars().count() > max_title {
            let truncated: String = self.title.chars().take(max_title - 3).collect();
            format!("{}...", truncated)
        } else {
            self.title.clone()
        };
        let venue_part = match &self.venue {
            Some(v) if !v.is_empty() => {
                let v = if v.chars().count() > max_venue {
                    let truncated: String = v.chars().take(max_venue - 3).collect();
                    format!("{}...", truncated)
                } else {
                    v.clone()
                };
                format!(" — {}", v)
            }
            _ => String::new(),
        };
        format!("{} ({}) {}{}", author, year, title, venue_part)
    }
}

pub async fn search_by_title(query: &str, limit: usize) -> Result<Vec<SearchResult>> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://api.crossref.org/works?query.bibliographic={}&rows={}",
        urlencoding::encode(query),
        limit
    );

    let resp = client
        .get(&url)
        .header(
            "User-Agent",
            "bibox/0.1 (https://github.com/user/bibox; mailto:user@example.com)",
        )
        .send()
        .await
        .context("Crossref search request failed")?;

    if !resp.status().is_success() {
        anyhow::bail!("Crossref search failed: HTTP {}", resp.status());
    }

    let data: CrossrefSearchResponse = resp.json().await.context("Crossref search response parse failed")?;

    let results: Vec<SearchResult> = data
        .message
        .items
        .into_iter()
        .filter_map(|msg| {
            let doi = msg.doi?;
            let title = msg.title.and_then(|v| v.into_iter().next()).unwrap_or_default();
            if title.is_empty() {
                return None;
            }
            let authors: Vec<String> = msg
                .author
                .unwrap_or_default()
                .into_iter()
                .map(|a| match (a.family, a.given) {
                    (Some(f), Some(g)) => format!("{}, {}", f, g),
                    (Some(f), None) => f,
                    (None, Some(g)) => g,
                    (None, None) => "Unknown".to_string(),
                })
                .collect();
            let year = msg
                .published_print
                .as_ref()
                .or(msg.published_online.as_ref())
                .and_then(|d| d.date_parts.as_ref())
                .and_then(|dp| dp.first())
                .and_then(|parts| parts.first())
                .map(|&y| y as u32);
            let venue = msg
                .event
                .and_then(|e| e.name)
                .or_else(|| msg.container_title.and_then(|v| v.into_iter().next()));
            Some(SearchResult {
                doi,
                title,
                authors,
                year,
                venue,
            })
        })
        .collect();

    Ok(results)
}
