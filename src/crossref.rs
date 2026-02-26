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
