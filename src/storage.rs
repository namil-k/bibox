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

    let merge_log = raw.get("merge_log").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    Ok(Database { entries, merge_log })
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

    let raw = fmt.replace("{author}", &last_name)
        .replace("{year}", &year_str)
        .replace("{title}", &title_word);
    // Sanitize: only allow ASCII alphanumeric, underscore, hyphen
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect()
}

/// A citekey has to survive BibTeX, `\cite{}` and a note file name.
pub fn valid_citekey(key: &str) -> bool {
    !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Move an entry's PDF to its `Author_Year_Title.pdf` name after the metadata
/// changed. Never overwrites: if another file has that name, `<stem>_<key>.pdf`
/// (the import rule); if that is taken too, the file stays where it is.
/// Returns the old and new names when the file moved.
pub fn sync_pdf_name(entry: &mut Entry, pdf_dir: &Path) -> Result<Option<(String, String)>> {
    let Some(old) = entry.file_path.clone() else { return Ok(None) };
    let old_path = pdf_dir.join(&old);
    if !old_path.exists() {
        return Ok(None);
    }
    let stem = crate::bibtex::entry_to_filename(entry);
    for new in [format!("{}.pdf", stem), format!("{}_{}.pdf", stem, entry.bibtex_key)] {
        if new == old {
            return Ok(None);
        }
        let new_path = pdf_dir.join(&new);
        if !new_path.exists() {
            std::fs::rename(&old_path, &new_path)?;
            entry.file_path = Some(new.clone());
            return Ok(Some((old, new)));
        }
    }
    Ok(None)
}

/// Give the entry `old` the citekey `new`. Its note file follows the key; its
/// PDF keeps the name its metadata gives it. Changes nothing and fails if `new`
/// is malformed, belongs to another entry, or already has a note file.
pub fn rename_entry(db: &mut Database, old: &str, new: &str, pdf_dir: &Path, notes_dir: &Path) -> Result<()> {
    if !valid_citekey(new) {
        anyhow::bail!("Invalid citekey \"{}\": use only ASCII letters, digits, '_' and '-'", new);
    }
    if old == new {
        return Ok(());
    }
    if key_exists(db, new) {
        anyhow::bail!("Citekey \"{}\" is already used by another entry", new);
    }
    let old_note = notes_dir.join(format!("{}.md", old));
    let new_note = notes_dir.join(format!("{}.md", new));
    if new_note.exists() {
        anyhow::bail!("A note file already exists at {}; move it away first", new_note.display());
    }
    let entry = find_by_key_mut(db, old).ok_or_else(|| anyhow::anyhow!("Entry not found: {}", old))?;
    if old_note.exists() {
        std::fs::rename(&old_note, &new_note)?;
    }
    entry.bibtex_key = new.to_string();
    sync_pdf_name(entry, pdf_dir)?;
    Ok(())
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

#[cfg(test)]
mod merge_log_tests {
    use super::*;

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("bibox-storage-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("db.json")
    }

    #[test]
    fn an_empty_merge_log_is_not_written_and_a_full_one_survives_a_round_trip() {
        let p = tmp("mlog");
        save_db(&Database::default(), &p).unwrap();
        assert!(!std::fs::read_to_string(&p).unwrap().contains("merge_log"));
        let mut db = Database::default();
        db.merge_log.push(serde_json::json!({"field": "title", "rule": "newer"}));
        save_db(&db, &p).unwrap();
        let back = load_db(&p).unwrap();
        assert_eq!(back.merge_log.len(), 1);
        assert_eq!(back.merge_log[0]["rule"], "newer");
    }
}
