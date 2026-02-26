use anyhow::{Context, Result};
use regex::Regex;

#[derive(Debug)]
pub struct ArxivResult {
    pub pdf_url: String,
    pub title: String,
    pub arxiv_id: String,
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

fn parse_arxiv_response(xml: &str) -> Result<Vec<ArxivResult>> {
    let re_entry = Regex::new(r"(?s)<entry>(.*?)</entry>").unwrap();
    // Title is the first <title> tag inside each entry
    let re_title = Regex::new(r"(?s)<title>(.*?)</title>").unwrap();
    // Match any <link> tag containing title="pdf" (attribute order varies)
    let re_pdf_tag = Regex::new(r#"(?i)<link\b[^>]*\btitle="pdf"[^>]*/>"#).unwrap();
    // Extract href value from a tag string
    let re_href = Regex::new(r#"\bhref="([^"]+)""#).unwrap();
    // arXiv ID: <id>http://arxiv.org/abs/XXXXXXXX</id>
    let re_id = Regex::new(r"<id>https?://arxiv\.org/abs/([^<\s]+)</id>").unwrap();

    let mut results = vec![];

    for entry_cap in re_entry.captures_iter(xml) {
        let entry = entry_cap.get(1).unwrap().as_str();

        let title = re_title
            .captures(entry)
            .and_then(|c| c.get(1))
            .map(|m| {
                // Collapse whitespace and decode basic HTML entities
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

        // Find the <link> tag with title="pdf", then extract its href (order-independent)
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

        if let Some(url) = pdf_url {
            if !title.is_empty() {
                results.push(ArxivResult {
                    pdf_url: url,
                    title,
                    arxiv_id,
                });
            }
        }
    }

    Ok(results)
}
