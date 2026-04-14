use crate::models::{Entry, EntryType};

/// Escape LaTeX special characters for BibTeX output.
/// & → \&, % → \%, # → \#
/// Does NOT escape _ (common in DOIs/URLs) or $ (rare in bibliographic data).
fn escape_bibtex(s: &str) -> String {
    s.replace('&', "\\&")
     .replace('%', "\\%")
     .replace('#', "\\#")
}

pub fn entry_to_bibtex(entry: &Entry) -> String {
    let mut lines = Vec::new();

    let type_str = match entry.entry_type {
        EntryType::Article => "article",
        EntryType::Book => "book",
        EntryType::InProceedings => "inproceedings",
        EntryType::Misc => "misc",
    };

    lines.push(format!("@{}{{{},", type_str, entry.bibtex_key));

    if let Some(title) = &entry.title {
        lines.push(format!("  title     = {{{}}},", escape_bibtex(title)));
    }

    if !entry.author.is_empty() {
        let author_str = entry.author.join(" and ");
        lines.push(format!("  author    = {{{}}},", escape_bibtex(&author_str)));
    }

    if let Some(year) = entry.year {
        lines.push(format!("  year      = {{{}}},", year));
    }

    // Type-specific fields
    match entry.entry_type {
        EntryType::Article => {
            if let Some(j) = &entry.journal {
                lines.push(format!("  journal   = {{{}}},", escape_bibtex(j)));
            }
            if let Some(v) = &entry.volume {
                lines.push(format!("  volume    = {{{}}},", v));
            }
            if let Some(n) = &entry.number {
                lines.push(format!("  number    = {{{}}},", n));
            }
            if let Some(p) = &entry.pages {
                lines.push(format!("  pages     = {{{}}},", p));
            }
        }
        EntryType::Book => {
            if let Some(p) = &entry.publisher {
                lines.push(format!("  publisher = {{{}}},", escape_bibtex(p)));
            }
            if let Some(ed) = &entry.editor {
                lines.push(format!("  editor    = {{{}}},", escape_bibtex(ed)));
            }
            if let Some(e) = &entry.edition {
                lines.push(format!("  edition   = {{{}}},", e));
            }
            if let Some(isbn) = &entry.isbn {
                lines.push(format!("  isbn      = {{{}}},", isbn));
            }
        }
        EntryType::InProceedings => {
            if let Some(bt) = &entry.booktitle {
                lines.push(format!("  booktitle = {{{}}},", escape_bibtex(bt)));
            }
            if let Some(p) = &entry.pages {
                lines.push(format!("  pages     = {{{}}},", p));
            }
        }
        EntryType::Misc => {
            if let Some(u) = &entry.url {
                lines.push(format!("  url       = {{{}}},", u));
            }
        }
    }

    if let Some(doi) = &entry.doi {
        lines.push(format!("  doi       = {{{}}},", doi));
    }

    if let Some(note) = &entry.note {
        lines.push(format!("  note      = {{{}}},", escape_bibtex(note)));
    }

    // Remove trailing comma from last field
    if let Some(last) = lines.last_mut() {
        if last.ends_with(',') {
            last.pop();
        }
    }

    lines.push("}".to_string());
    lines.join("\n")
}

pub fn entries_to_bibtex(entries: &[&Entry]) -> String {
    entries
        .iter()
        .map(|e| entry_to_bibtex(e))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Generate a PDF filename from entry metadata.
/// Format: Author_Year_Full Title.pdf or Author_et_al_Year_Full Title.pdf
pub fn entry_to_filename(entry: &Entry) -> String {
    let author_part = match entry.author.len() {
        0 => "Unknown".to_string(),
        1 => entry.author[0]
            .split(',')
            .next()
            .unwrap_or("Unknown")
            .trim()
            .to_string(),
        _ => {
            let last = entry.author[0]
                .split(',')
                .next()
                .unwrap_or("Unknown")
                .trim()
                .to_string();
            format!("{}_et_al", last)
        }
    };

    let year_part = entry
        .year
        .map(|y| y.to_string())
        .unwrap_or_else(|| "0000".to_string());

    let title_part = entry.title.as_deref().unwrap_or("Untitled");

    // Sanitize: remove characters that are problematic in filenames
    let sanitize = |s: &str| -> String {
        s.chars()
            .map(|c| match c {
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
                _ => c,
            })
            .collect()
    };

    format!(
        "{}_{}_{}",
        sanitize(&author_part),
        year_part,
        sanitize(title_part)
    )
}
