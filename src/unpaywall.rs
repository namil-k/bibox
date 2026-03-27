use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct UnpaywallResponse {
    best_oa_location: Option<OaLocation>,
    is_oa: bool,
}

#[derive(Debug, Deserialize)]
struct OaLocation {
    url_for_pdf: Option<String>,
    url: Option<String>,
    host_type: Option<String>,
    #[serde(rename = "repository_institution")]
    repository: Option<String>,
}

#[derive(Debug)]
pub struct OaResult {
    pub pdf_url: String,
    pub source: String,
}

/// Find an open-access PDF for a given DOI using the Unpaywall API.
pub async fn find_open_access(doi: &str) -> Result<Option<OaResult>> {
    let client = reqwest::Client::new();
    let url = format!(
        "https://api.unpaywall.org/v2/{}?email=bibox@example.com",
        doi
    );

    let resp = client
        .get(&url)
        .send()
        .await
        .context("Unpaywall API request failed")?;

    if !resp.status().is_success() {
        return Ok(None);
    }

    let data: UnpaywallResponse = match resp.json().await {
        Ok(d) => d,
        Err(_) => return Ok(None),
    };

    if !data.is_oa {
        return Ok(None);
    }

    if let Some(loc) = data.best_oa_location {
        let pdf_url = loc.url_for_pdf.or(loc.url);
        if let Some(url) = pdf_url {
            let source = loc
                .host_type
                .unwrap_or_else(|| loc.repository.unwrap_or_else(|| "Open Access".to_string()));
            return Ok(Some(OaResult {
                pdf_url: url,
                source,
            }));
        }
    }

    Ok(None)
}

/// Download a PDF from a URL and save it to the given path.
pub async fn download_pdf(url: &str, dest: &std::path::Path) -> Result<()> {
    let client = reqwest::Client::builder()
        .user_agent("bibox/0.1 (https://github.com/namil-k/bibox; mailto:bibox@example.com)")
        .build()?;
    let resp = client
        .get(url)
        .send()
        .await
        .context("PDF download request failed")?;

    if !resp.status().is_success() {
        anyhow::bail!("PDF download failed: HTTP {}", resp.status());
    }

    let bytes = resp.bytes().await.context("PDF download failed")?;
    std::fs::write(dest, bytes)?;
    Ok(())
}
