use anyhow::{Context, Result};
use serde::Deserialize;

use crate::models::{Entry, EntryType};
use crate::storage::{generate_bibtex_key, generate_unique_key};
use crate::models::Database;

#[derive(Debug, Deserialize)]
struct OLBook {
    title: Option<String>,
    authors: Option<Vec<OLAuthorRef>>,
    publish_date: Option<String>,
    publishers: Option<Vec<String>>,
    number_of_pages: Option<u32>,
    isbn_13: Option<Vec<String>>,
    isbn_10: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct OLAuthorRef {
    key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OLAuthor {
    name: Option<String>,
}

async fn resolve_author(client: &reqwest::Client, key: &str) -> Option<String> {
    // key looks like "/authors/OL123A"
    let clean_key = key.trim_start_matches('/');
    let url = format!("https://openlibrary.org/{}.json", clean_key);
    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let author: OLAuthor = resp.json().await.ok()?;
    author.name
}

fn parse_year(publish_date: &str) -> Option<u32> {
    // Try to find a 4-digit year anywhere in the string
    for part in publish_date.split_whitespace() {
        if let Ok(y) = part.trim_matches(',').parse::<u32>() {
            if (1000..=9999).contains(&y) {
                return Some(y);
            }
        }
    }
    // Fallback: parse any 4-digit sequence
    let digits: String = publish_date.chars().collect();
    let re = regex::Regex::new(r"\b(\d{4})\b").ok()?;
    re.captures(&digits)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
}

pub async fn fetch_by_isbn(isbn: &str, db: &Database) -> Result<Entry> {
    let client = reqwest::Client::builder()
        .user_agent("bibox/0.1 (https://github.com/user/bibox)")
        .build()?;

    let url = format!("https://openlibrary.org/isbn/{}.json", isbn);
    let resp = client
        .get(&url)
        .send()
        .await
        .context("Open Library API request failed")?;

    if !resp.status().is_success() {
        anyhow::bail!("ISBN not found: {} (HTTP {})", isbn, resp.status());
    }

    let book: OLBook = resp.json().await.context("Open Library response parse failed")?;

    // Resolve authors
    let mut authors: Vec<String> = vec![];
    if let Some(author_refs) = &book.authors {
        for ar in author_refs {
            if let Some(ref key) = ar.key {
                if let Some(name) = resolve_author(&client, key).await {
                    // Store as "Last, First" if possible; Open Library returns full names
                    authors.push(name);
                }
            }
        }
    }

    let year = book
        .publish_date
        .as_deref()
        .and_then(parse_year);

    let title = book.title.clone();

    let isbn_val = book
        .isbn_13
        .as_ref()
        .and_then(|v| v.first().cloned())
        .or_else(|| book.isbn_10.as_ref().and_then(|v| v.first().cloned()))
        .unwrap_or_else(|| isbn.to_string());

    let publisher = book.publishers.as_ref().and_then(|v| v.first().cloned());

    let base_key = generate_bibtex_key(
        &authors,
        year,
        title.as_deref().unwrap_or("unknown"),
    );
    let bibtex_key = generate_unique_key(db, &base_key);

    let entry = Entry {
        id: uuid::Uuid::new_v4().to_string(),
        bibtex_key,
        entry_type: EntryType::Book,
        title,
        author: authors,
        year,
        journal: None,
        volume: None,
        number: None,
        pages: book.number_of_pages.map(|n| n.to_string()),
        publisher,
        editor: None,
        edition: None,
        isbn: Some(isbn_val),
        booktitle: None,
        doi: None,
        url: None,
        abstract_text: None,
        tags: vec![],
        howpublished: None,
        month: None,
        note: None,
        collections: vec![],
        file_path: None,
        created_at: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        updated_at: None,
    };

    Ok(entry)
}
