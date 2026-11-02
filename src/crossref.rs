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
    issued: Option<CrossrefDate>,
    published: Option<CrossrefDate>,
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
    created: Option<CrossrefDate>,
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
    // Crossref sends [[null]] when it has no date, so each part may be null.
    #[serde(rename = "date-parts")]
    date_parts: Option<Vec<Vec<Option<i32>>>>,
}

impl CrossrefDate {
    fn year(&self) -> Option<u32> {
        let y = (*self.date_parts.as_ref()?.first()?.first()?)?;
        (y > 0).then_some(y as u32)
    }
}

#[derive(Debug, Deserialize)]
struct CrossrefEvent {
    name: Option<String>,
    start: Option<CrossrefDate>,
}

/// A 19xx or 20xx year standing on its own in a venue name, as in "(VR 2006)".
fn year_in_text(s: &str) -> Option<u32> {
    let re = regex::Regex::new(r"\b(19|20)\d{2}\b").ok()?;
    re.find(s)?.as_str().parse().ok()
}

/// The publication year, trying Crossref's date fields from most to least
/// authoritative, then a year written in the event or venue name. `created`
/// (when the DOI was registered) is deliberately left out: see `year_hint`.
fn publication_year(msg: &CrossrefMessage) -> Option<u32> {
    let event = msg.event.as_ref();
    [
        msg.issued.as_ref(),
        msg.published.as_ref(),
        msg.published_print.as_ref(),
        msg.published_online.as_ref(),
        event.and_then(|e| e.start.as_ref()),
    ]
    .into_iter()
    .flatten()
    .find_map(CrossrefDate::year)
    .or_else(|| event.and_then(|e| e.name.as_deref()).and_then(year_in_text))
    .or_else(|| {
        msg.container_title.iter().flatten().find_map(|t| year_in_text(t))
    })
}

#[derive(Debug)]
pub struct Metadata {
    pub title: Option<String>,
    pub authors: Vec<String>,
    pub year: Option<u32>,
    /// The year the DOI was registered, offered when `year` is None. It is not
    /// the publication year: an old paper registered late would get it wrong.
    pub year_hint: Option<u32>,
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
    Ok(metadata_from_message(data.message, doi))
}

fn metadata_from_message(msg: CrossrefMessage, doi: &str) -> Metadata {
    use crate::bibtex::decode_html_entities as decode;
    let year = publication_year(&msg);
    let year_hint = msg.created.as_ref().and_then(CrossrefDate::year);

    let title = msg.title.and_then(|v| v.into_iter().next()).map(|t| decode(&t));

    let authors: Vec<String> = msg
        .author
        .unwrap_or_default()
        .into_iter()
        .map(|a| {
            decode(&match (a.family, a.given) {
                (Some(f), Some(g)) => format!("{}, {}", f, g),
                (Some(f), None) => f,
                (None, Some(g)) => g,
                (None, None) => String::from("Unknown"),
            })
        })
        .collect();

    let entry_type = match msg.entry_type.as_deref().unwrap_or("") {
        "journal-article" => "article",
        "book" | "book-chapter" => "book",
        "proceedings-article" => "inproceedings",
        _ => "misc",
    }
    .to_string();

    let container_first = msg.container_title.and_then(|v| v.into_iter().next()).map(|t| decode(&t));

    let journal = if entry_type == "article" {
        container_first.clone()
    } else {
        None
    };

    let booktitle = if entry_type == "inproceedings" {
        msg.event
            .and_then(|e| e.name)
            .map(|n| decode(&n))
            .or(container_first)
    } else {
        None
    };

    Metadata {
        title,
        authors,
        year,
        year_hint,
        journal,
        publisher: msg.publisher.map(|p| decode(&p)),
        volume: msg.volume,
        number: msg.issue,
        pages: msg.page,
        booktitle,
        doi: msg.doi.unwrap_or_else(|| doi.to_string()),
        url: msg.url,
        entry_type,
    }
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
            let year = publication_year(&msg);
            let doi = msg.doi?;
            let title = msg.title.and_then(|v| v.into_iter().next()).map(|t| crate::bibtex::decode_html_entities(&t)).unwrap_or_default();
            if title.is_empty() {
                return None;
            }
            let authors: Vec<String> = msg
                .author
                .unwrap_or_default()
                .into_iter()
                .map(|a| crate::bibtex::decode_html_entities(&match (a.family, a.given) {
                    (Some(f), Some(g)) => format!("{}, {}", f, g),
                    (Some(f), None) => f,
                    (None, Some(g)) => g,
                    (None, None) => "Unknown".to_string(),
                }))
                .collect();
            let venue = msg
                .event
                .and_then(|e| e.name)
                .or_else(|| msg.container_title.and_then(|v| v.into_iter().next()))
                .map(|v| crate::bibtex::decode_html_entities(&v));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Metadata {
        let msg: CrossrefMessage = serde_json::from_str(json).unwrap();
        metadata_from_message(msg, "10.0/x")
    }

    #[test]
    fn year_comes_from_the_event_name_when_every_date_field_is_empty() {
        // Shape of the real response for 10.1109/VR.2006.47: issued is [[null]],
        // no published-*, only created (the DOI registration date) carries a year.
        let m = parse(r#"{
            "title": ["Design and Evaluation of Navigation Techniques for Multiscale Virtual Environments"],
            "issued": {"date-parts": [[null]]},
            "created": {"date-parts": [[2006, 4, 28]]},
            "event": {"name": "IEEE Virtual Reality Conference (VR 2006)", "location": "Alexandria, VA, USA"},
            "container-title": ["IEEE Virtual Reality Conference (VR 2006)"],
            "type": "proceedings-article"
        }"#);
        assert_eq!(m.year, Some(2006));
    }

    #[test]
    fn year_prefers_issued_over_the_other_date_fields() {
        let m = parse(r#"{
            "issued": {"date-parts": [[2019, 5]]},
            "published-online": {"date-parts": [[2018, 12]]},
            "created": {"date-parts": [[2018, 11]]}
        }"#);
        assert_eq!(m.year, Some(2019));
    }

    #[test]
    fn year_falls_back_to_event_start_before_the_venue_text() {
        let m = parse(r#"{
            "event": {"name": "CHI PLAY '16", "start": {"date-parts": [[2016, 10, 16]]}},
            "container-title": ["Proceedings of the 2017 Something"]
        }"#);
        assert_eq!(m.year, Some(2016));
    }

    #[test]
    fn created_is_only_a_hint_never_the_year() {
        // created is when the DOI was registered; an old paper registered late
        // would get the wrong year, so it is offered, not used.
        let m = parse(r#"{
            "issued": {"date-parts": [[null]]},
            "created": {"date-parts": [[2008, 3, 1]]},
            "container-title": ["Journal of Old Things"]
        }"#);
        assert_eq!(m.year, None);
        assert_eq!(m.year_hint, Some(2008));
    }

    #[test]
    fn html_entities_are_decoded_in_title_authors_and_venue() {
        // 10.1145/2967934.2968105 comes back as "Point &amp; Teleport ...".
        let m = parse(r#"{
            "title": ["Point &amp; Teleport Locomotion Technique for Virtual Reality"],
            "author": [{"family": "O&#39;Brien", "given": "Evren"}],
            "container-title": ["Proceedings of CHI PLAY &amp; Friends"],
            "publisher": "ACM &amp; Co",
            "type": "proceedings-article"
        }"#);
        assert_eq!(m.title.as_deref(), Some("Point & Teleport Locomotion Technique for Virtual Reality"));
        assert_eq!(m.authors, vec!["O'Brien, Evren".to_string()]);
        assert_eq!(m.booktitle.as_deref(), Some("Proceedings of CHI PLAY & Friends"));
        assert_eq!(m.publisher.as_deref(), Some("ACM & Co"));
    }
}
