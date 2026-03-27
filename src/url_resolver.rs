// src/url_resolver.rs
use anyhow::{bail, Context, Result};
use scraper::{Html, Selector};

/// Result of resolving a URL to bibliographic metadata.
pub enum ResolvedUrl {
    /// Resolved to a DOI string
    Doi(String),
    /// Resolved to an arXiv ID
    ArxivId(String),
    /// Extracted metadata directly from HTML meta tags
    Metadata(UrlMetadata),
}

pub struct UrlMetadata {
    pub title: Option<String>,
    pub authors: Vec<String>,
    pub year: Option<u32>,
    pub journal: Option<String>,
    pub doi: Option<String>,
    pub url: String,
}

/// Try to resolve a URL to a DOI, arXiv ID, or direct metadata.
pub async fn resolve_url(url: &str) -> Result<ResolvedUrl> {
    if let Some(resolved) = try_pattern_match(url) {
        return Ok(resolved);
    }
    fetch_and_parse_meta(url).await
}

/// Extract DOI or arXiv ID from known URL patterns.
fn try_pattern_match(url: &str) -> Option<ResolvedUrl> {
    let url_lower = url.to_lowercase();

    // arXiv: arxiv.org/abs/<id> or arxiv.org/pdf/<id>
    if url_lower.contains("arxiv.org/abs/") || url_lower.contains("arxiv.org/pdf/") {
        let id = url
            .rsplit('/')
            .next()
            .map(|s| s.trim_end_matches(".pdf"))
            .map(|s| s.to_string())?;
        if !id.is_empty() {
            return Some(ResolvedUrl::ArxivId(id));
        }
    }

    // doi.org direct link
    if url_lower.contains("doi.org/10.") {
        let doi = extract_doi_from_path(url, "doi.org/")?;
        return Some(ResolvedUrl::Doi(doi));
    }

    // ACM Digital Library
    if url_lower.contains("dl.acm.org/doi/10.") {
        let doi = extract_doi_from_path(url, "dl.acm.org/doi/")?;
        return Some(ResolvedUrl::Doi(doi));
    }

    // Springer
    if url_lower.contains("link.springer.com/article/10.") {
        let doi = extract_doi_from_path(url, "link.springer.com/article/")?;
        return Some(ResolvedUrl::Doi(doi));
    }

    // Nature
    if url_lower.contains("nature.com/articles/") {
        let suffix = url.rsplit("nature.com/articles/").next()?;
        let suffix = suffix.split('?').next().unwrap_or(suffix);
        if !suffix.is_empty() {
            return Some(ResolvedUrl::Doi(format!("10.1038/{}", suffix)));
        }
    }

    // IEEE Xplore — fall through to HTML fallback
    if url_lower.contains("ieeexplore.ieee.org/document/") {
        return None;
    }

    None
}

/// Extract DOI from URL path after a known prefix.
fn extract_doi_from_path(url: &str, prefix: &str) -> Option<String> {
    let idx = url.find(prefix)?;
    let after = &url[idx + prefix.len()..];
    let doi = after.split('?').next().unwrap_or(after);
    let doi = doi.trim_end_matches('/');
    if doi.starts_with("10.") {
        Some(doi.to_string())
    } else {
        None
    }
}

/// Fetch URL and extract metadata from HTML <meta> tags.
/// pub(crate) so cmd_add can call this directly for ArxivId URLs
/// (avoids re-entering resolve_url which would pattern-match again).
pub(crate) async fn fetch_and_parse_meta(url: &str) -> Result<ResolvedUrl> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()?;

    let resp = client
        .get(url)
        .header(
            "User-Agent",
            "bibox/0.1 (https://github.com/user/bibox; mailto:user@example.com)",
        )
        .send()
        .await
        .context("Failed to fetch URL")?;

    if !resp.status().is_success() {
        bail!("Failed to fetch URL: HTTP {}", resp.status());
    }

    let html = resp.text().await?;
    let document = Html::parse_document(&html);

    // Try citation_doi first
    if let Some(doi) = get_meta_content(&document, "citation_doi") {
        if doi.starts_with("10.") {
            return Ok(ResolvedUrl::Doi(doi));
        }
    }

    // Try to extract metadata from meta tags
    let title = get_meta_content(&document, "citation_title");
    let authors = get_meta_contents(&document, "citation_author");
    let year = get_meta_content(&document, "citation_date")
        .or_else(|| get_meta_content(&document, "citation_publication_date"))
        .and_then(|d| d.split('/').next().and_then(|y| y.parse::<u32>().ok()));
    let journal = get_meta_content(&document, "citation_journal_title");

    if title.is_some() || !authors.is_empty() {
        return Ok(ResolvedUrl::Metadata(UrlMetadata {
            title,
            authors,
            year,
            journal,
            doi: None,
            url: url.to_string(),
        }));
    }

    bail!("Could not extract metadata from URL. Try --doi or --search instead.");
}

fn get_meta_content(doc: &Html, name: &str) -> Option<String> {
    let selector = Selector::parse(&format!("meta[name=\"{}\"]", name)).ok()?;
    doc.select(&selector)
        .next()
        .and_then(|el| el.value().attr("content"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn get_meta_contents(doc: &Html, name: &str) -> Vec<String> {
    let selector = match Selector::parse(&format!("meta[name=\"{}\"]", name)) {
        Ok(s) => s,
        Err(_) => return vec![],
    };
    doc.select(&selector)
        .filter_map(|el| el.value().attr("content"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arxiv_abs() {
        let r = try_pattern_match("https://arxiv.org/abs/2301.12345").unwrap();
        match r {
            ResolvedUrl::ArxivId(id) => assert_eq!(id, "2301.12345"),
            _ => panic!("Expected ArxivId"),
        }
    }

    #[test]
    fn arxiv_pdf() {
        let r = try_pattern_match("https://arxiv.org/pdf/2301.12345.pdf").unwrap();
        match r {
            ResolvedUrl::ArxivId(id) => assert_eq!(id, "2301.12345"),
            _ => panic!("Expected ArxivId"),
        }
    }

    #[test]
    fn doi_org() {
        let r = try_pattern_match("https://doi.org/10.1145/3132747.3132763").unwrap();
        match r {
            ResolvedUrl::Doi(doi) => assert_eq!(doi, "10.1145/3132747.3132763"),
            _ => panic!("Expected Doi"),
        }
    }

    #[test]
    fn acm_dl() {
        let r = try_pattern_match("https://dl.acm.org/doi/10.1145/3132747.3132763").unwrap();
        match r {
            ResolvedUrl::Doi(doi) => assert_eq!(doi, "10.1145/3132747.3132763"),
            _ => panic!("Expected Doi"),
        }
    }

    #[test]
    fn springer() {
        let r = try_pattern_match("https://link.springer.com/article/10.1007/s00607-024-01268-x").unwrap();
        match r {
            ResolvedUrl::Doi(doi) => assert_eq!(doi, "10.1007/s00607-024-01268-x"),
            _ => panic!("Expected Doi"),
        }
    }

    #[test]
    fn nature() {
        let r = try_pattern_match("https://www.nature.com/articles/s41586-024-07487-w").unwrap();
        match r {
            ResolvedUrl::Doi(doi) => assert_eq!(doi, "10.1038/s41586-024-07487-w"),
            _ => panic!("Expected Doi"),
        }
    }

    #[test]
    fn doi_org_with_query_params() {
        let r = try_pattern_match("https://doi.org/10.1145/3132747.3132763?ref=pdf").unwrap();
        match r {
            ResolvedUrl::Doi(doi) => assert_eq!(doi, "10.1145/3132747.3132763"),
            _ => panic!("Expected Doi"),
        }
    }

    #[test]
    fn ieee_falls_through() {
        let r = try_pattern_match("https://ieeexplore.ieee.org/document/8049192");
        assert!(r.is_none());
    }

    #[test]
    fn unknown_url() {
        let r = try_pattern_match("https://example.com/random-page");
        assert!(r.is_none());
    }
}
