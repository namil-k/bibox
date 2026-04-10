use anyhow::Result;
use std::path::Path;

use crate::models::{Database, Entry};

pub fn load_db(db_path: &Path) -> Result<Database> {
    if !db_path.exists() {
        return Ok(Database::default());
    }
    let content = std::fs::read_to_string(db_path)?;

    // Parse entry-by-entry so one bad entry doesn't break the whole DB
    let raw: serde_json::Value = serde_json::from_str(&content)?;
    let entries_val = raw.get("entries").and_then(|v| v.as_array());

    let entries = match entries_val {
        None => vec![],
        Some(arr) => {
            let mut good = Vec::with_capacity(arr.len());
            for (i, val) in arr.iter().enumerate() {
                match serde_json::from_value::<Entry>(val.clone()) {
                    Ok(entry) => good.push(entry),
                    Err(e) => {
                        eprintln!("bibox: skipping malformed entry at index {} ({})", i, e);
                        eprintln!("  Run `bibox doctor` to inspect and repair.");
                    }
                }
            }
            good
        }
    };

    Ok(Database { entries })
}

pub fn save_db(db: &Database, db_path: &Path) -> Result<()> {
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(db)?;
    std::fs::write(db_path, content)?;
    Ok(())
}

pub fn find_by_key<'a>(db: &'a Database, key: &str) -> Option<&'a Entry> {
    db.entries
        .iter()
        .find(|e| e.bibtex_key == key || e.id == key)
}

pub fn find_by_key_mut<'a>(db: &'a mut Database, key: &str) -> Option<&'a mut Entry> {
    db.entries
        .iter_mut()
        .find(|e| e.bibtex_key == key || e.id == key)
}

pub fn key_exists(db: &Database, key: &str) -> bool {
    db.entries.iter().any(|e| e.bibtex_key == key)
}

pub fn generate_unique_key(db: &Database, base_key: &str) -> String {
    if !key_exists(db, base_key) {
        return base_key.to_string();
    }
    for suffix in b'a'..=b'z' {
        let candidate = format!("{}{}", base_key, suffix as char);
        if !key_exists(db, &candidate) {
            return candidate;
        }
    }
    // Fallback: append number
    for i in 2..=99 {
        let candidate = format!("{}{}", base_key, i);
        if !key_exists(db, &candidate) {
            return candidate;
        }
    }
    format!("{}_dup", base_key)
}

/// Generate a unique key against a list of existing keys, excluding one key (the entry being updated).
pub fn generate_unique_key_excluding(existing: &[&str], base_key: &str, exclude: &str) -> String {
    let exists = |k: &str| existing.iter().any(|e| *e == k && *e != exclude);
    if !exists(base_key) {
        return base_key.to_string();
    }
    for suffix in b'a'..=b'z' {
        let candidate = format!("{}{}", base_key, suffix as char);
        if !exists(&candidate) {
            return candidate;
        }
    }
    for i in 2..=99 {
        let candidate = format!("{}{}", base_key, i);
        if !exists(&candidate) {
            return candidate;
        }
    }
    format!("{}_dup", base_key)
}

pub fn generate_bibtex_key(authors: &[String], year: Option<u32>, title: &str) -> String {
    generate_bibtex_key_fmt(authors, year, title, "{author}{year}{title}")
}

pub fn generate_bibtex_key_fmt(authors: &[String], year: Option<u32>, title: &str, fmt: &str) -> String {
    let last_name = authors
        .first()
        .and_then(|a| a.split(',').next())
        .unwrap_or("unknown")
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>();

    let year_str = year
        .map(|y| y.to_string())
        .unwrap_or_else(|| "0000".to_string());

    let skip_words = [
        "a", "an", "the", "of", "in", "on", "at", "to", "for", "with", "and", "or",
    ];
    let title_word = title
        .split_whitespace()
        .find(|w| {
            let lower = w.to_lowercase();
            let clean: String = lower.chars().filter(|c| c.is_alphabetic()).collect();
            !clean.is_empty() && !skip_words.contains(&clean.as_str())
        })
        .unwrap_or("unknown")
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect::<String>();

    fmt.replace("{author}", &last_name)
        .replace("{year}", &year_str)
        .replace("{title}", &title_word)
}

pub fn filter_entries<'a>(
    db: &'a Database,
    collection: Option<&str>,
    entry_type: Option<&str>,
    tag: Option<&str>,
    year: Option<u32>,
) -> Vec<&'a Entry> {
    db.entries
        .iter()
        .filter(|e| {
            if let Some(col) = collection {
                let prefix = format!("{}/", col);
                if !e.collections.iter().any(|c| c == col || c.starts_with(&prefix)) {
                    return false;
                }
            }
            if let Some(et) = entry_type {
                if e.entry_type.to_string() != et.to_lowercase() {
                    return false;
                }
            }
            if let Some(t) = tag {
                if !e.tags.iter().any(|tg| tg == t) {
                    return false;
                }
            }
            if let Some(y) = year {
                if e.year != Some(y) {
                    return false;
                }
            }
            true
        })
        .collect()
}

pub fn search_entries<'a>(
    db: &'a Database,
    query: &str,
    field: Option<&str>,
    collection: Option<&str>,
    case_sensitive: bool,
) -> Vec<&'a Entry> {
    let q = if case_sensitive {
        query.to_string()
    } else {
        query.to_lowercase()
    };

    db.entries
        .iter()
        .filter(|e| {
            if let Some(col) = collection {
                let prefix = format!("{}/", col);
                if !e.collections.iter().any(|c| c == col || c.starts_with(&prefix)) {
                    return false;
                }
            }

            let matches = |s: &str| -> bool {
                if case_sensitive {
                    s.contains(&q)
                } else {
                    s.to_lowercase().contains(&q)
                }
            };

            match field.unwrap_or("all") {
                "title" => e.title.as_deref().map(matches).unwrap_or(false),
                "author" => e.author.iter().any(|a| matches(a)),
                "year" => e.year.map(|y| matches(&y.to_string())).unwrap_or(false),
                "tag" => e.tags.iter().any(|t| matches(t)),
                "key" => matches(&e.bibtex_key),
                _ => {
                    // all fields
                    e.title.as_deref().map(matches).unwrap_or(false)
                        || e.author.iter().any(|a| matches(a))
                        || e.year.map(|y| matches(&y.to_string())).unwrap_or(false)
                        || e.tags.iter().any(|t| matches(t))
                        || matches(&e.bibtex_key)
                }
            }
        })
        .collect()
}
