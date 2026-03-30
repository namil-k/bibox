use anyhow::{bail, Result};
use crate::models::Entry;

pub fn find_section(content: &str, section_name: &str) -> Option<(usize, usize)> {
    let target = section_name.trim().to_lowercase();
    let mut i = 0;
    let bytes = content.as_bytes();
    while i < bytes.len() {
        let line_start = i;
        let line_end = content[i..].find('\n').map(|p| i + p).unwrap_or(content.len());
        let line = &content[line_start..line_end];
        if line.starts_with("## ") {
            let heading = line[3..].trim().to_lowercase();
            if heading == target {
                let content_start = if line_end < content.len() { line_end + 1 } else { line_end };
                let content_end = find_next_h2(content, content_start);
                return Some((content_start, content_end));
            }
        }
        if line_end >= content.len() { break; }
        i = line_end + 1;
    }
    None
}

fn find_next_h2(content: &str, from: usize) -> usize {
    let mut i = from;
    while i < content.len() {
        let line_end = content[i..].find('\n').map(|p| i + p).unwrap_or(content.len());
        let line = &content[i..line_end];
        if line.starts_with("## ") {
            return i;
        }
        if line_end >= content.len() { break; }
        i = line_end + 1;
    }
    content.len()
}

pub fn write_section(content: &str, section_name: &str, new_body: &str) -> String {
    if let Some((start, end)) = find_section(content, section_name) {
        let mut result = String::with_capacity(content.len());
        result.push_str(&content[..start]);
        result.push_str(new_body);
        if !new_body.ends_with('\n') { result.push('\n'); }
        result.push('\n');
        result.push_str(&content[end..]);
        result
    } else {
        let mut result = content.to_string();
        if !result.ends_with('\n') { result.push('\n'); }
        result.push_str(&format!("## {}\n", section_name));
        result.push_str(new_body);
        if !new_body.ends_with('\n') { result.push('\n'); }
        result.push('\n');
        result
    }
}

const TEMPLATE_AI_SUMMARY: &str = r#"# {{title}}
citekey: {{citekey}}
doi: {{doi}}
year: {{year}}
author: {{author}}

## Summary

## Key Contributions

## Methodology

## Results

## Limitations

## Related Work

## Notes
"#;

const TEMPLATE_READING_NOTES: &str = r#"# {{title}}
citekey: {{citekey}}

## Main Argument

## Evidence

## Questions

## Quotes

## Connection to My Work
"#;

pub const BUILTIN_NAMES: &[&str] = &["ai-summary", "reading-notes"];

pub fn builtin_template(name: &str) -> Option<&'static str> {
    match name {
        "ai-summary" => Some(TEMPLATE_AI_SUMMARY),
        "reading-notes" => Some(TEMPLATE_READING_NOTES),
        _ => None,
    }
}

/// List all available templates: built-in + custom from templates_dir.
/// Returns (name, is_custom, is_override) tuples.
pub fn list_templates(templates_dir: &std::path::Path) -> Vec<(String, bool, bool)> {
    let mut result: Vec<(String, bool, bool)> = Vec::new();
    let mut custom_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Scan custom dir
    if templates_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(templates_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "md").unwrap_or(false) {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        custom_names.insert(stem.to_string());
                    }
                }
            }
        }
    }

    // Built-ins first
    for &name in BUILTIN_NAMES {
        let overridden = custom_names.contains(name);
        result.push((name.to_string(), false, overridden));
    }

    // Custom templates (excluding overrides already listed)
    let mut custom_only: Vec<String> = custom_names.into_iter()
        .filter(|n| !BUILTIN_NAMES.contains(&n.as_str()))
        .collect();
    custom_only.sort();
    for name in custom_only {
        result.push((name, true, false));
    }

    result
}

pub fn load_template(name: &str, templates_dir: &std::path::Path) -> Result<String> {
    let custom_path = templates_dir.join(format!("{}.md", name));
    if custom_path.exists() {
        return Ok(std::fs::read_to_string(&custom_path)?);
    }
    if let Some(content) = builtin_template(name) {
        return Ok(content.to_string());
    }
    let mut available: Vec<String> = vec!["ai-summary".into(), "reading-notes".into()];
    if templates_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(templates_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "md").unwrap_or(false) {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        if !available.contains(&stem.to_string()) {
                            available.push(stem.to_string());
                        }
                    }
                }
            }
        }
    }
    bail!("Template '{}' not found. Available: {}", name, available.join(", "));
}

pub fn render_template(template: &str, entry: &Entry) -> String {
    template
        .replace("{{title}}", entry.title.as_deref().unwrap_or(""))
        .replace("{{citekey}}", &entry.bibtex_key)
        .replace("{{doi}}", entry.doi.as_deref().unwrap_or(""))
        .replace("{{year}}", &entry.year.map(|y| y.to_string()).unwrap_or_default())
        .replace("{{author}}", &entry.author.join(", "))
        .replace("{{journal}}", entry.journal.as_deref().unwrap_or(""))
        .replace("{{booktitle}}", entry.booktitle.as_deref().unwrap_or(""))
        .replace("{{publisher}}", entry.publisher.as_deref().unwrap_or(""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Entry, EntryType};

    fn make_test_entry() -> Entry {
        Entry {
            id: "test-id-123".to_string(),
            bibtex_key: "kim2024attention".to_string(),
            entry_type: EntryType::Article,
            title: Some("Attention Is All You Need".to_string()),
            author: vec!["Kim, J".to_string(), "Lee, S".to_string()],
            year: Some(2024),
            journal: Some("Nature".to_string()),
            volume: Some("1".to_string()),
            number: Some("2".to_string()),
            pages: Some("1--10".to_string()),
            publisher: Some("Springer".to_string()),
            editor: None,
            edition: None,
            isbn: None,
            booktitle: Some("NeurIPS 2024".to_string()),
            doi: Some("10.1234/test".to_string()),
            url: None,
            tags: vec![],
            note: None,
            collections: vec![],
            file_path: None,
            created_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn find_section_exists() {
        let content = "## Summary\nThis is the summary.\n## Methods\nSome methods.\n";
        let result = find_section(content, "Summary");
        assert!(result.is_some());
        let (start, end) = result.unwrap();
        let section_body = &content[start..end];
        assert!(section_body.contains("This is the summary."));
        assert!(!section_body.contains("Some methods."));
    }

    #[test]
    fn find_section_case_insensitive() {
        let content = "## Summary\nBody here.\n## Next\n";
        assert!(find_section(content, "summary").is_some());
        assert!(find_section(content, "SUMMARY").is_some());
        assert!(find_section(content, " Summary ").is_some());
    }

    #[test]
    fn find_section_not_found() {
        let content = "## Summary\nBody here.\n";
        assert!(find_section(content, "Nonexistent").is_none());
    }

    #[test]
    fn find_section_at_eof() {
        let content = "## Summary\nBody here.";
        let result = find_section(content, "Summary");
        assert!(result.is_some());
        let (start, end) = result.unwrap();
        assert_eq!(end, content.len());
        let section_body = &content[start..end];
        assert!(section_body.contains("Body here."));
    }

    #[test]
    fn write_section_replace_existing() {
        let content = "## Summary\nOld summary.\n## Methods\nKeep this.\n";
        let result = write_section(content, "Summary", "New summary content.");
        assert!(result.contains("New summary content."));
        assert!(result.contains("## Methods"));
        assert!(result.contains("Keep this."));
        assert!(!result.contains("Old summary."));
    }

    #[test]
    fn write_section_append_new() {
        let content = "## Summary\nExisting.\n";
        let result = write_section(content, "New Section", "Brand new content.");
        assert!(result.contains("## New Section"));
        assert!(result.contains("Brand new content."));
        assert!(result.contains("## Summary"));
        assert!(result.contains("Existing."));
    }

    #[test]
    fn render_template_substitutes_vars() {
        let entry = make_test_entry();
        let template = "# {{title}}\ncitekey: {{citekey}}\ndoi: {{doi}}\nyear: {{year}}\nauthor: {{author}}\njournal: {{journal}}\nbooktitle: {{booktitle}}\npublisher: {{publisher}}\n";
        let rendered = render_template(template, &entry);
        assert!(rendered.contains("Attention Is All You Need"));
        assert!(rendered.contains("kim2024attention"));
        assert!(rendered.contains("10.1234/test"));
        assert!(rendered.contains("2024"));
        assert!(rendered.contains("Kim, J, Lee, S"));
        assert!(rendered.contains("Nature"));
        assert!(rendered.contains("NeurIPS 2024"));
        assert!(rendered.contains("Springer"));
    }

    #[test]
    fn render_template_missing_values_empty() {
        let mut entry = make_test_entry();
        entry.title = None;
        entry.doi = None;
        entry.year = None;
        entry.author = vec![];
        entry.journal = None;
        entry.booktitle = None;
        entry.publisher = None;
        let template = "title={{title}} doi={{doi}} year={{year}} author={{author}}";
        let rendered = render_template(template, &entry);
        assert_eq!(rendered, "title= doi= year= author=");
    }
}
