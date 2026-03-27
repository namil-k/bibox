use anyhow::{Context, Result};
use regex::Regex;

#[derive(Debug)]
pub struct ArxivResult {
    pub pdf_url: String,
    pub title: String,
    pub arxiv_id: String,
    pub authors: Vec<String>,
    pub year: Option<u32>,
    pub doi: Option<String>,
}

/// Build a compact search term from a full title.
/// Uses the part before ":" (subtitle separator) if present,
/// otherwise takes the first 6 words. Strips punctuation arXiv dislikes.
fn compact_title_query(title: &str) -> String {
    // If there's a colon, use only the main title part (before colon)
    let base = if let Some(pos) = title.find(':') {
        &title[..pos]
    } else {
        title
    };

    // Take at most 6 words and strip special chars except alphanumeric/spaces
    let words: Vec<&str> = base.split_whitespace().take(6).collect();
    let query = words.join(" ");

    // Remove characters that confuse arXiv query parser
    query
        .chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .trim()
        .to_string()
}

/// Search arXiv by title and return up to `max_results` entries with PDF links.
pub async fn search_by_title(title: &str, max_results: usize) -> Result<Vec<ArxivResult>> {
    let client = reqwest::Client::new();
    let query = compact_title_query(title);

    let resp = client
        .get("https://export.arxiv.org/api/query")
        .query(&[
            ("search_query", format!("ti:{}", query)),
            ("max_results", max_results.to_string()),
            ("sortBy", "relevance".to_string()),
        ])
        .send()
        .await
        .context("arXiv API request failed")?;

    if !resp.status().is_success() {
        anyhow::bail!("arXiv API error: HTTP {}", resp.status());
    }

    let xml = resp.text().await.context("arXiv response read failed")?;
    parse_arxiv_response(&xml)
}

/// Fetch a single arXiv entry by ID (e.g., "2301.12345").
pub async fn fetch_by_id(arxiv_id: &str) -> Result<Option<ArxivResult>> {
    let client = reqwest::Client::new();
    let resp = client
        .get("https://export.arxiv.org/api/query")
        .query(&[("id_list", arxiv_id)])
        .send()
        .await
        .context("arXiv API request failed")?;

    if !resp.status().is_success() {
        anyhow::bail!("arXiv API error: HTTP {}", resp.status());
    }

    let xml = resp.text().await.context("arXiv response read failed")?;
    let mut results = parse_arxiv_response(&xml)?;
    Ok(results.pop())
}

fn parse_arxiv_response(xml: &str) -> Result<Vec<ArxivResult>> {
    let re_entry = Regex::new(r"(?s)<entry>(.*?)</entry>").unwrap();
    let re_title = Regex::new(r"(?s)<title>(.*?)</title>").unwrap();
    let re_pdf_tag = Regex::new(r#"(?i)<link\b[^>]*\btitle="pdf"[^>]*/>"#).unwrap();
    let re_href = Regex::new(r#"\bhref="([^"]+)""#).unwrap();
    let re_id = Regex::new(r"<id>https?://arxiv\.org/abs/([^<\s]+)</id>").unwrap();
    let re_author = Regex::new(r"<author>\s*<name>([^<]+)</name>").unwrap();
    let re_published = Regex::new(r"<published>(\d{4})").unwrap();
    let re_doi = Regex::new(r#"<arxiv:doi[^>]*>([^<]+)</arxiv:doi>"#).unwrap();

    let mut results = vec![];

    for entry_cap in re_entry.captures_iter(xml) {
        let entry = entry_cap.get(1).unwrap().as_str();

        let title = re_title
            .captures(entry)
            .and_then(|c| c.get(1))
            .map(|m| {
                m.as_str()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .replace("&amp;", "&")
                    .replace("&lt;", "<")
                    .replace("&gt;", ">")
                    .replace("&quot;", "\"")
            })
            .unwrap_or_default();

        let pdf_url = re_pdf_tag
            .find(entry)
            .and_then(|tag_match| re_href.captures(tag_match.as_str()))
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().replace("http://arxiv.org", "https://arxiv.org"));

        let arxiv_id = re_id
            .captures(entry)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string())
            .unwrap_or_default();

        let authors: Vec<String> = re_author
            .captures_iter(entry)
            .filter_map(|c| c.get(1).map(|m| m.as_str().trim().to_string()))
            .collect();

        let year = re_published
            .captures(entry)
            .and_then(|c| c.get(1))
            .and_then(|m| m.as_str().parse::<u32>().ok());

        let doi = re_doi
            .captures(entry)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().trim().to_string());

        if let Some(url) = pdf_url {
            if !title.is_empty() {
                results.push(ArxivResult {
                    pdf_url: url,
                    title,
                    arxiv_id,
                    authors,
                    year,
                    doi,
                });
            }
        }
    }

    Ok(results)
}
