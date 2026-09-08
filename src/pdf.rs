use anyhow::Result;
use regex::Regex;
use std::path::Path;
use std::sync::OnceLock;

/// How much of the extracted text counts as the "first page" region.
/// Publishers print the paper's own DOI on page one, while reference lists
/// (full of other papers' DOIs) come last; in a 46-paper sample every own DOI
/// that was in the text at all sat within the first 5,200 characters, except
/// one printed at the very end.
const FIRST_PAGE_CHARS: usize = 8_000;

/// Extract DOI from a PDF file.
///
/// Sources, most reliable first:
/// 1. Publisher metadata in the raw bytes (XMP `prism:doi`, `dc:identifier`,
///    `pdfx:doi`, Info dictionary `/doi`) - always the paper's own DOI.
/// 2. Body text via anydoc (decodes the compressed content streams where
///    modern PDFs keep their text), preferring the first page.
/// 3. A raw-bytes scan, for PDFs anydoc cannot read (scanned pages,
///    malformed files) that still expose a DOI in uncompressed regions.
pub fn extract_doi(path: &Path) -> Result<Option<String>> {
    let bytes = std::fs::read(path)?;
    Ok(extract_doi_from_bytes(&bytes))
}

/// Same as [`extract_doi`], but for an in-memory document.
pub fn extract_doi_from_bytes(bytes: &[u8]) -> Option<String> {
    let raw = String::from_utf8_lossy(bytes);
    if let Some(doi) = find_doi_in_metadata(&raw) {
        return Some(doi);
    }
    if let Ok(text) = anydoc::to_markdown_bytes(bytes, None) {
        if let Some(doi) = find_doi_in_text(&text) {
            return Some(doi);
        }
    }
    best_candidate(&raw, false)
}

/// Find the paper's own DOI in publisher metadata (XMP packet or Info
/// dictionary) present in the raw bytes.
fn find_doi_in_metadata(raw: &str) -> Option<String> {
    metadata_patterns()
        .iter()
        .filter_map(|re| re.captures(raw))
        .find_map(|caps| clean_doi(caps.get(1)?.as_str()))
}

/// Find the paper's own DOI in its extracted body text: the earliest
/// candidate on the first page, else the most explicit one anywhere.
fn find_doi_in_text(text: &str) -> Option<String> {
    let mut end = FIRST_PAGE_CHARS.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    best_candidate(&text[..end], true).or_else(|| best_candidate(text, false))
}

/// Pick one DOI out of all candidates in `text`.
///
/// `by_position` selects the earliest match; otherwise the most explicit
/// pattern wins (explicit `doi:` prefix, then doi.org URL, then bare DOI).
/// Either way a candidate that another candidate extends (`10.1145/3450626`
/// next to `10.1145/3450626.3459831`) is replaced by the longer one: that is
/// a proceedings DOI beside the paper's, or a DOI broken by a line wrap.
fn best_candidate(text: &str, by_position: bool) -> Option<String> {
    let cands = candidates(text);
    let pick = if by_position {
        cands.iter().min_by_key(|c| c.offset)
    } else {
        cands.iter().min_by_key(|c| (c.rank, c.offset))
    }?;
    let longest = cands
        .iter()
        .filter(|c| extends(&c.doi, &pick.doi))
        .max_by_key(|c| c.doi.len())
        .unwrap_or(pick);
    Some(longest.doi.clone())
}

struct Candidate {
    offset: usize,
    rank: usize,
    doi: String,
}

fn candidates(text: &str) -> Vec<Candidate> {
    let mut out = Vec::new();
    for (rank, re) in text_patterns().iter().enumerate() {
        for caps in re.captures_iter(text) {
            if let Some(m) = caps.get(1) {
                if let Some(doi) = clean_doi(m.as_str()) {
                    out.push(Candidate { offset: m.start(), rank, doi });
                }
            }
        }
    }
    out
}

/// `longer` is `shorter` plus a punctuation-separated suffix.
fn extends(longer: &str, shorter: &str) -> bool {
    longer.len() > shorter.len()
        && longer.starts_with(shorter)
        && !longer[shorter.len()..].starts_with(|c: char| c.is_alphanumeric())
}

/// Strip trailing punctuation and Markdown emphasis that the patterns may
/// have swallowed; reject anything that no longer looks like a DOI.
fn clean_doi(s: &str) -> Option<String> {
    let doi = s.trim_end_matches(['.', ',', ';', ')', ']', '>', '*']);
    if doi.starts_with("10.") && doi.contains('/') {
        Some(doi.to_string())
    } else {
        None
    }
}

/// Body-text patterns in priority order. DOI format: 10.XXXX/anything
fn text_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        compile(&[
            // Explicit doi: prefix
            r#"(?i)doi[:\s]+['"]?(10\.\d{4,}/[^\s\x00-\x1f<>\)\]"']+)"#,
            // https://doi.org/...
            r#"https?://(?:dx\.)?doi\.org/(10\.\d{4,}/[^\s\x00-\x1f<>\)\]"']+)"#,
            // Bare DOI (less reliable, try last)
            r#"\b(10\.\d{4,}/[^\s\x00-\x1f<>\)\]"',;]+)"#,
        ])
    })
}

/// Publisher metadata patterns, matched against the raw bytes.
fn metadata_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        compile(&[
            r#"(?i)<prism:doi[^>]*>\s*(10\.\d{4,}/[^<\s]+)"#,
            r#"(?i)<pdfx:doi[^>]*>\s*(10\.\d{4,}/[^<\s]+)"#,
            r#"(?i)<dc:identifier[^>]*>\s*(?:doi:|https?://(?:dx\.)?doi\.org/)?\s*(10\.\d{4,}/[^<\s]+)"#,
            // Info dictionary entry, e.g. `/doi (10.1016/j.isci.2020.101397)`
            r#"(?i)/doi\s*\(\s*(10\.\d{4,}/[^)\s]+)"#,
        ])
    })
}

fn compile(patterns: &[&str]) -> Vec<Regex> {
    patterns.iter().map(|p| Regex::new(p).expect("valid DOI regex")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Build a minimal single-page PDF whose content stream holds `text`.
    /// With `compress`, the stream is FlateDecode-encoded so the DOI is not
    /// visible in the raw bytes (the common case for real-world PDFs).
    fn minimal_pdf(text: &str, compress: bool) -> Vec<u8> {
        build_pdf(text, compress, None, None)
    }

    /// Fixture builder. `comment` writes a `%` comment line after the header
    /// (ignored by parsers, so only a raw-bytes scan can see it). `xmp` embeds
    /// an XMP metadata stream referenced from the catalog, like publisher PDFs.
    fn build_pdf(text: &str, compress: bool, comment: Option<&str>, xmp: Option<&str>) -> Vec<u8> {
        let content = format!("BT /F1 12 Tf 72 720 Td ({}) Tj ET", text).into_bytes();
        let stream: Vec<u8> = if compress {
            let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
            enc.write_all(&content).unwrap();
            let data = enc.finish().unwrap();
            let mut s = format!("<< /Length {} /Filter /FlateDecode >>\nstream\n", data.len()).into_bytes();
            s.extend_from_slice(&data);
            s.extend_from_slice(b"\nendstream");
            s
        } else {
            let mut s = format!("<< /Length {} >>\nstream\n", content.len()).into_bytes();
            s.extend_from_slice(&content);
            s.extend_from_slice(b"\nendstream");
            s
        };
        let catalog = match xmp {
            Some(_) => b"<< /Type /Catalog /Pages 2 0 R /Metadata 6 0 R >>".to_vec(),
            None => b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        };
        let mut objs: Vec<Vec<u8>> = vec![
            catalog,
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_vec(),
            stream,
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        ];
        if let Some(x) = xmp {
            let body = format!(
                "<?xpacket begin=\"\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"><rdf:Description rdf:about=\"\" xmlns:prism=\"http://prismstandard.org/namespaces/basic/2.0/\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:pdfx=\"http://ns.adobe.com/pdfx/1.3/\">{}</rdf:Description></rdf:RDF></x:xmpmeta>\n<?xpacket end=\"w\"?>",
                x
            );
            let mut s = format!("<< /Type /Metadata /Subtype /XML /Length {} >>\nstream\n", body.len()).into_bytes();
            s.extend_from_slice(body.as_bytes());
            s.extend_from_slice(b"\nendstream");
            objs.push(s);
        }
        let mut pdf = b"%PDF-1.4\n".to_vec();
        if let Some(c) = comment {
            pdf.extend_from_slice(format!("% {}\n", c).as_bytes());
        }
        let mut offsets = Vec::new();
        for (i, o) in objs.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            pdf.extend_from_slice(o);
            pdf.extend_from_slice(b"\nendobj\n");
        }
        let xref = pdf.len();
        pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
        for off in offsets {
            pdf.extend_from_slice(format!("{:010} 00000 n \n", off).as_bytes());
        }
        pdf.extend_from_slice(
            format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n", objs.len() + 1, xref).as_bytes(),
        );
        pdf
    }

    fn with_temp_pdf<T>(bytes: &[u8], f: impl FnOnce(&Path) -> T) -> T {
        let path = std::env::temp_dir().join(format!(
            "bibox_pdf_test_{}_{:?}.pdf",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&path, bytes).unwrap();
        let out = f(&path);
        let _ = std::fs::remove_file(&path);
        out
    }

    // ---- end-to-end on PDF bytes ----

    #[test]
    fn extract_doi_reads_compressed_content_stream() {
        let pdf = minimal_pdf("Example paper. doi:10.1234/zenodo.5555 for testing", true);
        assert!(!pdf.windows(7).any(|w| w == b"10.1234"), "fixture must hide the DOI in raw bytes");
        let doi = with_temp_pdf(&pdf, |p| extract_doi(p).unwrap());
        assert_eq!(doi.as_deref(), Some("10.1234/zenodo.5555"));
    }

    #[test]
    fn extract_doi_reads_plain_content_stream() {
        let pdf = minimal_pdf("Example paper. https://doi.org/10.5555/plain.999 in stream", false);
        let doi = with_temp_pdf(&pdf, |p| extract_doi(p).unwrap());
        assert_eq!(doi.as_deref(), Some("10.5555/plain.999"));
    }

    #[test]
    fn extract_doi_from_bytes_prefers_publisher_metadata_over_body_text() {
        // Body text cites another paper first; the XMP packet carries the paper's own DOI.
        let pdf = build_pdf(
            "Extends doi:10.1111/cited.work with new data.",
            true,
            None,
            Some("<prism:doi>10.2222/own.paper</prism:doi>"),
        );
        assert_eq!(extract_doi_from_bytes(&pdf).as_deref(), Some("10.2222/own.paper"));
    }

    #[test]
    fn extract_doi_from_bytes_falls_back_to_raw_scan_when_text_has_no_doi() {
        let pdf = build_pdf("No identifier in the body text.", true, Some("doi:10.7777/raw.only"), None);
        assert_eq!(extract_doi_from_bytes(&pdf).as_deref(), Some("10.7777/raw.only"));
    }

    #[test]
    fn extract_doi_from_bytes_falls_back_to_raw_scan_for_non_pdf_input() {
        let bytes = b"not a document at all, but mentions doi:10.9999/not.pdf somewhere";
        assert_eq!(extract_doi_from_bytes(bytes).as_deref(), Some("10.9999/not.pdf"));
    }

    // ---- publisher metadata (XMP / Info dictionary) ----

    #[test]
    fn find_doi_in_metadata_reads_each_publisher_field() {
        assert_eq!(find_doi_in_metadata("<prism:doi>10.1109/vr55154.2023.00036</prism:doi>").as_deref(), Some("10.1109/vr55154.2023.00036"));
        assert_eq!(find_doi_in_metadata("<pdfx:doi>10.1016/j.vrih.2025.08.003</pdfx:doi>").as_deref(), Some("10.1016/j.vrih.2025.08.003"));
        assert_eq!(find_doi_in_metadata("<dc:identifier>doi:10.1038/s44287-024-00139-1</dc:identifier>").as_deref(), Some("10.1038/s44287-024-00139-1"));
        assert_eq!(find_doi_in_metadata("<dc:identifier>https://doi.org/10.1016/j.isci.2020.101397</dc:identifier>").as_deref(), Some("10.1016/j.isci.2020.101397"));
        assert_eq!(find_doi_in_metadata("<< /Title (X) /doi (10.1038/s41377-020-0341-9) /Producer (Y) >>").as_deref(), Some("10.1038/s41377-020-0341-9"));
    }

    #[test]
    fn find_doi_in_metadata_ignores_non_doi_identifiers_and_body_text() {
        assert_eq!(find_doi_in_metadata("<dc:identifier>urn:issn:0042-6989</dc:identifier>"), None);
        assert_eq!(find_doi_in_metadata("plain text mentioning doi:10.1234/not.metadata"), None);
    }

    // ---- body text ----

    #[test]
    fn find_doi_in_text_prefers_first_page_over_reference_list() {
        // The paper's own DOI sits on page one; the reference list far below
        // uses the more explicit `doi:` form, which must not win.
        let text = format!(
            "Journal 41 (2001) 23-36. https://doi.org/10.1000/own.paper\n\nIntroduction...{}\nReferences\n[1] Smith. doi:10.2000/cited.paper",
            "x".repeat(12_000)
        );
        assert_eq!(find_doi_in_text(&text).as_deref(), Some("10.1000/own.paper"));
    }

    #[test]
    fn find_doi_in_text_takes_earliest_candidate_on_first_page() {
        let text = "Cited: 10.1000/bare.first ... DOI: 10.2000/explicit.later";
        assert_eq!(find_doi_in_text(text).as_deref(), Some("10.1000/bare.first"));
    }

    #[test]
    fn find_doi_in_text_falls_back_to_whole_text_when_first_page_has_none() {
        let text = format!("No identifier up front.{}\nCite this article as: doi:10.1186/2042-6410-3-20", "x".repeat(12_000));
        assert_eq!(find_doi_in_text(&text).as_deref(), Some("10.1186/2042-6410-3-20"));
    }

    #[test]
    fn find_doi_in_text_prefers_extended_doi_over_its_prefix() {
        // ACM prints the proceedings DOI in the citation block and the paper's
        // longer DOI in the footer; a line break can also truncate the first one.
        let text = "ACM Trans. Graph. https://doi.org/10.1145/3450626 ... $15.00 https://doi.org/10.1145/3450626.3459831";
        assert_eq!(find_doi_in_text(text).as_deref(), Some("10.1145/3450626.3459831"));
    }

    #[test]
    fn find_doi_in_text_strips_markdown_emphasis_markers() {
        let text = "**[https://doi.org/10.1038/s44287-024-00139-1**](https://doi.org/10.1038/s44287-024-00139-1**)";
        assert_eq!(find_doi_in_text(text).as_deref(), Some("10.1038/s44287-024-00139-1"));
    }

    #[test]
    fn find_doi_in_text_stops_at_html_tags() {
        let text = "DOI: <u>10.1080/00140139.2018.1502817</u>";
        assert_eq!(find_doi_in_text(text).as_deref(), Some("10.1080/00140139.2018.1502817"));
    }

    #[test]
    fn find_doi_in_text_reads_markdown_link_to_doi_org() {
        let text = "See [https://doi.org/10.5555/x.1](https://doi.org/10.5555/x.1) for details.";
        assert_eq!(find_doi_in_text(text).as_deref(), Some("10.5555/x.1"));
    }

    #[test]
    fn find_doi_in_text_strips_trailing_punctuation() {
        assert_eq!(find_doi_in_text("doi:10.1234/abc.def.").as_deref(), Some("10.1234/abc.def"));
        assert_eq!(find_doi_in_text("(doi:10.1234/abc)").as_deref(), Some("10.1234/abc"));
    }

    #[test]
    fn find_doi_in_text_keeps_underscores_and_asterisks() {
        assert_eq!(find_doi_in_text("doi:10.1234/ab_cd*ef").as_deref(), Some("10.1234/ab_cd*ef"));
    }

    #[test]
    fn find_doi_in_text_returns_none_without_doi() {
        assert_eq!(find_doi_in_text("Version 10.4 of the software, released 2019/2020."), None);
        assert_eq!(find_doi_in_text(""), None);
    }
}
