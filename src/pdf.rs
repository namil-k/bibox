use anyhow::Result;
use regex::Regex;
use std::path::Path;

/// Extract DOI from a PDF file.
/// Strategy: read raw bytes and search for DOI patterns.
/// Many modern PDFs embed DOIs as plain ASCII in the content stream.
pub fn extract_doi(path: &Path) -> Result<Option<String>> {
    let bytes = std::fs::read(path)?;
    let content = String::from_utf8_lossy(&bytes);

    // Common DOI patterns
    // DOI format: 10.XXXX/anything
    let patterns = [
        // Explicit doi: prefix
        r#"(?i)doi[:\s]+(['"]?)(10\.\d{4,}/[^\s\x00-\x1f\)\]>"']+)"#,
        // https://doi.org/...
        r#"https?://(?:dx\.)?doi\.org/(10\.\d{4,}/[^\s\x00-\x1f\)\]>"']+)"#,
        // Bare DOI (less reliable, try last)
        r#"\b(10\.\d{4,}/[^\s\x00-\x1f\)\]>"',;]+)"#,
    ];

    for pattern in &patterns {
        let re = Regex::new(pattern)?;
        if let Some(caps) = re.captures(&content) {
            // Get the DOI part (last capture group)
            let doi = caps
                .get(caps.len() - 1)
                .map(|m| m.as_str())
                .unwrap_or("")
                .trim_end_matches(['.', ',', ';', ')', ']', '>'])
                .to_string();

            if !doi.is_empty() && doi.starts_with("10.") {
                return Ok(Some(doi));
            }
        }
    }

    Ok(None)
}
