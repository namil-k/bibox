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

fn builtin_template(name: &str) -> Option<&'static str> {
    match name {
        "ai-summary" => Some(TEMPLATE_AI_SUMMARY),
        "reading-notes" => Some(TEMPLATE_READING_NOTES),
        _ => None,
    }
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
