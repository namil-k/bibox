use anyhow::{Context, Result};
use chrono::Local;
use crossterm::event::{self as ct_event, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::bibtex::{entries_to_bibtex, entry_to_filename};
use crate::config::Config;
use crate::crossref;
use crate::git;
use crate::interactive::{interactive_select, SelectItem};
use crate::models::{Entry, EntryType};
use crate::pdf;
use crate::storage::{
    filter_entries, find_by_key, find_by_key_mut, generate_bibtex_key, generate_unique_key,
    load_db, save_db, search_entries,
};
use crate::arxiv;
use crate::unpaywall;
use crate::openlibrary;

fn db_path_from_config(config: &Config) -> PathBuf {
    crate::config::resolve_db_path(config)
}

fn prompt_confirm(msg: &str) -> bool {
    print!("{} [y/N] ", msg);
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap_or(0);
    matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
}

fn copy_to_clipboard(text: &str, config: &Config) -> Result<()> {
    let mut ctx = arboard::Clipboard::new()
        .context(config.msgs.clipboard_init_failed())?;
    ctx.set_text(text)
        .context(config.msgs.clipboard_copy_failed())?;
    Ok(())
}

fn print_entry_block(entry: &Entry, config: &Config) {
    let author = entry.author_display();
    let year = entry
        .year
        .map(|y| y.to_string())
        .unwrap_or_else(|| "n.d.".to_string());
    let title = entry.title.as_deref().unwrap_or(config.msgs.no_title());
    let tags = if entry.tags.is_empty() {
        String::new()
    } else {
        config.msgs.tag_inline(&entry.tags.join(", "))
    };
    let collections = if entry.collections.is_empty() {
        String::new()
    } else {
        config.msgs.collection_inline(&entry.collections.join(", "))
    };
    let file = if entry.file_path.is_some() { " [pdf]" } else { "" };

    println!("[{}] {}{}", entry.bibtex_key, title, file);
    println!(
        "{}",
        config.msgs.entry_block_meta(
            &entry.entry_type.to_string(),
            &author,
            &year,
            &tags,
            &collections
        )
    );
}

// ── add ─────────────────────────────────────────────────────────────────────

pub async fn cmd_add(
    file: Option<PathBuf>,
    to: Option<String>,
    doi_arg: Option<String>,
    isbn_arg: Option<String>,
    arxiv_arg: Option<String>,
    url_arg: Option<String>,
    search_arg: Option<String>,
    index_arg: Option<usize>,
    key_arg: Option<String>,
    title_arg: Option<String>,
    author_arg: Option<String>,
    year_arg: Option<u32>,
    entry_type_arg: Option<String>,
    journal_arg: Option<String>,
    publisher_arg: Option<String>,
    booktitle_arg: Option<String>,
    howpublished_arg: Option<String>,
    month_arg: Option<String>,
    json: bool,
    config: &Config,
) -> Result<()> {
    let db_path = db_path_from_config(config);
    let mut db = load_db(&db_path)?;

    std::fs::create_dir_all(&config.bibox_dir)?;

    // ── Mutable copies for resolution ──
    let mut doi_arg = doi_arg;
    let mut title_arg = title_arg;
    let mut author_arg = author_arg;
    let mut year_arg = year_arg;

    // ── --arxiv: fetch metadata directly from arXiv API ──
    if let Some(ref arxiv_id) = arxiv_arg {
        let id = arxiv_id.trim();
        println!("Fetching arXiv entry: {}", id);
        match crate::arxiv::fetch_by_id(id).await? {
            Some(result) => {
                // Use DOI if available, otherwise use arXiv metadata directly
                if let Some(doi) = result.doi {
                    doi_arg = Some(doi);
                } else {
                    // No DOI — use arXiv metadata directly
                    if title_arg.is_none() { title_arg = Some(result.title.clone()); }
                    if author_arg.is_none() && !result.authors.is_empty() {
                        author_arg = Some(result.authors.join("; "));
                    }
                    if year_arg.is_none() { year_arg = result.year; }
                }
                // Try to download PDF
                let tmp = std::env::temp_dir().join("bibox_download.pdf");
                if file.is_none() {
                    println!("Downloading PDF from arXiv...");
                    match crate::unpaywall::download_pdf(&result.pdf_url, &tmp).await {
                        Ok(()) => {
                            // We'll handle the file below in the normal flow
                        }
                        Err(e) => {
                            println!("PDF download failed: {}. Adding without PDF.", e);
                        }
                    }
                }
            }
            None => {
                anyhow::bail!("arXiv entry not found: {}", id);
            }
        }
    }

    // ── --search: Crossref title search → interactive select → DOI ──
    if let Some(ref query) = search_arg {
        println!("{}", config.msgs.searching_crossref_query(query));
        let results = crate::crossref::search_by_title(query, 5).await?;
        if results.is_empty() {
            anyhow::bail!("{}", config.msgs.no_search_results(query));
        }
        if let Some(idx) = index_arg {
            // Non-interactive: auto-select by index
            if idx >= results.len() {
                anyhow::bail!("Index {} out of range (0-{})", idx, results.len() - 1);
            }
            doi_arg = Some(results[idx].doi.clone());
        } else {
            // Interactive select
            let items: Vec<crate::interactive::SelectItem> = results
                .iter()
                .map(|r| crate::interactive::SelectItem {
                    key: r.doi.clone(),
                    display: r.display(60, 20),
                })
                .collect();
            match crate::interactive::interactive_select(&items)? {
                Some(doi) => { doi_arg = Some(doi); }
                None => return Ok(()), // user cancelled
            }
        }
    }

    // ── --url: resolve URL to DOI or metadata ──
    // If resolve fails, the URL is preserved for manual/misc entry
    let mut url_preserved: Option<String> = None;
    if let Some(ref url) = url_arg {
        match crate::url_resolver::resolve_url(url).await {
            Ok(crate::url_resolver::ResolvedUrl::Doi(doi)) => {
                doi_arg = Some(doi);
            }
            Ok(crate::url_resolver::ResolvedUrl::ArxivId(id)) => {
                let arxiv_url = format!("https://arxiv.org/abs/{}", id);
                match crate::url_resolver::fetch_and_parse_meta(&arxiv_url).await {
                    Ok(crate::url_resolver::ResolvedUrl::Doi(doi)) => {
                        doi_arg = Some(doi);
                    }
                    _ => {
                        // Could not resolve arXiv → DOI; preserve URL for misc entry
                        url_preserved = Some(url.clone());
                    }
                }
            }
            Ok(crate::url_resolver::ResolvedUrl::Metadata(meta)) => {
                // Direct metadata — create entry without Crossref
                let title = title_arg.unwrap_or_else(|| meta.title.unwrap_or_else(|| "Untitled".to_string()));
                let authors = if let Some(a) = author_arg {
                    a.split(';').map(|s| s.trim().to_string()).collect()
                } else {
                    meta.authors
                };
                let year = year_arg.or(meta.year);
                let journal = journal_arg.or(meta.journal);

                let base_key = generate_bibtex_key(&authors, year, &title);
                let bibtex_key = key_arg.unwrap_or_else(|| generate_unique_key(&db, &base_key));
                let collection = to.or_else(|| config.default_collection.clone());

                let entry = Entry {
                    id: Uuid::new_v4().to_string(),
                    bibtex_key: bibtex_key.clone(),
                    entry_type: entry_type_arg
                        .as_deref()
                        .and_then(|t| t.parse().ok())
                        .unwrap_or(EntryType::Misc),
                    title: Some(title),
                    author: authors,
                    year,
                    journal,
                    volume: None,
                    number: None,
                    pages: None,
                    publisher: publisher_arg,
                    editor: None,
                    edition: None,
                    isbn: None,
                    booktitle: booktitle_arg,
                    doi: meta.doi,
                    url: Some(meta.url),
                    abstract_text: None,
                    tags: vec![],
                    howpublished: howpublished_arg.clone(),
                    month: month_arg.clone(),
                    note: None,
                    collections: collection.map(|c| vec![c]).unwrap_or_default(),
                    file_path: None,
                    created_at: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                    updated_at: None,
                };

                if json {
                    println!("{}", serde_json::to_string_pretty(&entry)?);
                } else {
                    println!("{}", config.msgs.added(&bibtex_key, entry.title.as_deref().unwrap_or("?")));
                }
                db.entries.push(entry);
                save_db(&db, &db_path)?;
                if config.git {
                    git::auto_commit(&db_path, &format!("bibox: add {}", bibtex_key))?;
                }
                return Ok(());
            }
            Err(_) => {
                // URL resolution failed — preserve URL for manual/misc entry
                url_preserved = Some(url.clone());
            }
        }
    }

    // Duplicate DOI check
    if let Some(ref doi) = doi_arg {
        let doi_norm = doi.trim().to_lowercase();
        if let Some(existing_key) = db.entries.iter().find_map(|e| {
            e.doi.as_ref().filter(|d| d.trim().to_lowercase() == doi_norm).map(|_| e.bibtex_key.clone())
        }) {
            println!("{}", config.msgs.already_exists_with_hint(&existing_key));
            return Ok(());
        }
    }

    // ISBN-only mode (no file, no DOI)
    // Manual entry: --title with no file/DOI/ISBN/search/arxiv → create misc directly
    // Also accepts --url to store as-is (without DOI resolution)
    if file.is_none() && doi_arg.is_none() && isbn_arg.is_none()
        && search_arg.is_none() && arxiv_arg.is_none()
    {
        if let Some(ref title) = title_arg {
            let authors: Vec<String> = author_arg.as_deref()
                .map(|a| a.split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
                .unwrap_or_default();
            let base_key = generate_bibtex_key(&authors, year_arg, title);
            let bibtex_key = key_arg.unwrap_or_else(|| generate_unique_key(&db, &base_key));
            let collections = to
                .or_else(|| config.default_collection.clone())
                .map(|c| vec![c])
                .unwrap_or_default();
            let entry = Entry {
                id: Uuid::new_v4().to_string(),
                bibtex_key: bibtex_key.clone(),
                entry_type: EntryType::Misc,
                title: Some(title.clone()),
                author: authors,
                year: year_arg,
                journal: journal_arg,
                volume: None,
                number: None,
                pages: None,
                publisher: publisher_arg,
                editor: None,
                edition: None,
                isbn: None,
                booktitle: booktitle_arg,
                doi: None,
                url: url_preserved.clone().or_else(|| url_arg.clone()),
                abstract_text: None,
                tags: vec![],
                howpublished: howpublished_arg.clone(),
                month: month_arg.clone(),
                note: None,
                collections,
                file_path: None,
                created_at: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                updated_at: None,
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&entry)?);
            } else {
                println!("{}", config.msgs.added(&bibtex_key, title));
            }
            db.entries.push(entry);
            save_db(&db, &db_path)?;
            if config.git {
                git::auto_commit(&db_path, &format!("bibox: add {}", bibtex_key))?;
            }
            return Ok(());
        }
    }

    if file.is_none() && doi_arg.is_none() {
        if let Some(ref isbn) = isbn_arg {
            println!("Fetching metadata from Open Library for ISBN {}...", isbn);
            let mut entry = openlibrary::fetch_by_isbn(isbn, &db).await?;
            // Apply collection
            entry.collections = to
                .or_else(|| config.default_collection.clone())
                .map(|c| vec![c])
                .unwrap_or_default();
            // Apply overrides
            if let Some(k) = key_arg {
                entry.bibtex_key = generate_unique_key(&db, &k);
            }
            if let Some(t) = title_arg { entry.title = Some(t); }
            if let Some(a) = author_arg {
                entry.author = a.split(';').map(|s| s.trim().to_string()).collect();
            }
            if let Some(y) = year_arg { entry.year = Some(y); }
            if let Some(p) = publisher_arg { entry.publisher = Some(p); }

            if json {
                println!("{}", serde_json::to_string_pretty(&entry)?);
            } else {
                println!("Added: {} — {}", entry.bibtex_key, entry.title.as_deref().unwrap_or("?"));
            }
            let key_clone = entry.bibtex_key.clone();
            db.entries.push(entry);
            save_db(&db, &db_path)?;
            if config.git {
                git::auto_commit(&db_path, &format!("bibox: add {}", key_clone))?;
            }
            return Ok(());
        }
    }

    let mut meta: Option<crossref::Metadata> = None;
    let mut doi_found: Option<String> = doi_arg.clone();
    let mut temp_pdf_path: Option<PathBuf> = file.clone();

    // DOI-only mode (no file)
    if file.is_none() {
        if let Some(doi) = &doi_arg {
            println!("{}", config.msgs.fetching_crossref());
            match crossref::fetch_metadata(doi).await {
                Ok(m) => {
                    println!(
                        "{}",
                        config.msgs.found_title(m.title.as_deref().unwrap_or("?"))
                    );
                    meta = Some(m);
                }
                Err(e) => anyhow::bail!("{}", config.msgs.doi_lookup_failed(&e.to_string())),
            }

            println!("{}", config.msgs.searching_unpaywall());
            match unpaywall::find_open_access(doi).await {
                Ok(Some(oa)) => {
                    println!("{}", config.msgs.oa_found(&oa.source));
                    if prompt_confirm(config.msgs.download_prompt()) {
                        let tmp = std::env::temp_dir().join("bibox_download.pdf");
                        print!("{}", config.msgs.downloading());
                        io::stdout().flush()?;
                        unpaywall::download_pdf(&oa.pdf_url, &tmp).await?;
                        println!("{}", config.msgs.done());
                        temp_pdf_path = Some(tmp);
                    }
                }
                Ok(None) => {
                    println!("{}", config.msgs.no_oa_pdf());
                    // Fallback: search arXiv by title
                    if let Some(ref m) = meta {
                        if let Some(ref title_str) = m.title {
                            temp_pdf_path = try_arxiv_fallback(title_str, config).await;
                        }
                    }
                }
                Err(e) => {
                    println!("{}", config.msgs.unpaywall_failed(&e.to_string()));
                    if let Some(ref m) = meta {
                        if let Some(ref title_str) = m.title {
                            temp_pdf_path = try_arxiv_fallback(title_str, config).await;
                        }
                    }
                }
            }
        } else {
            anyhow::bail!("{}", config.msgs.no_file_or_doi());
        }
    } else {
        // File provided: extract DOI from PDF (unless --doi given)
        if doi_arg.is_none() {
            if let Some(path) = &file {
                print!("{}", config.msgs.extracting_doi());
                io::stdout().flush()?;
                match pdf::extract_doi(path) {
                    Ok(Some(doi)) => {
                        println!("{}", config.msgs.doi_found(&doi));
                        doi_found = Some(doi);
                    }
                    Ok(None) => println!("{}", config.msgs.doi_not_found()),
                    Err(e) => println!("{}", config.msgs.doi_extract_failed(&e.to_string())),
                }
            }
        }

        if let Some(doi) = &doi_found {
            println!("{}", config.msgs.fetching_crossref());
            match crossref::fetch_metadata(doi).await {
                Ok(m) => {
                    println!(
                        "{}",
                        config.msgs.found_title(m.title.as_deref().unwrap_or("?"))
                    );
                    meta = Some(m);
                }
                Err(e) => println!("{}", config.msgs.meta_lookup_failed(&e.to_string())),
            }
        }
    }

    let entry_type = if let Some(et) = &entry_type_arg {
        et.parse::<EntryType>().unwrap_or(EntryType::Misc)
    } else if let Some(m) = &meta {
        m.entry_type.parse::<EntryType>().unwrap_or(EntryType::Article)
    } else {
        EntryType::Article
    };

    let title = title_arg
        .clone()
        .or_else(|| meta.as_ref().and_then(|m| m.title.clone()));

    let authors: Vec<String> = if let Some(a) = author_arg {
        a.split(';').map(|s| s.trim().to_string()).collect()
    } else if let Some(m) = &meta {
        if !m.authors.is_empty() {
            m.authors.clone()
        } else {
            vec![]
        }
    } else {
        vec![]
    };

    let year = year_arg.or_else(|| meta.as_ref().and_then(|m| m.year));
    let journal = journal_arg
        .clone()
        .or_else(|| meta.as_ref().and_then(|m| m.journal.clone()));
    let publisher = publisher_arg
        .clone()
        .or_else(|| meta.as_ref().and_then(|m| m.publisher.clone()));
    let booktitle = booktitle_arg
        .clone()
        .or_else(|| meta.as_ref().and_then(|m| m.booktitle.clone()));
    let doi = doi_found.or_else(|| meta.as_ref().map(|m| m.doi.clone()));

    let base_key = key_arg.unwrap_or_else(|| {
        generate_bibtex_key(&authors, year, title.as_deref().unwrap_or("unknown"))
    });
    let bibtex_key = generate_unique_key(&db, &base_key);

    let file_path = if let Some(src_path) = &temp_pdf_path {
        let filename = if title.is_some() || !authors.is_empty() {
            let tmp_entry = Entry {
                id: String::new(),
                bibtex_key: bibtex_key.clone(),
                entry_type: entry_type.clone(),
                title: title.clone(),
                author: authors.clone(),
                year,
                journal: journal.clone(),
                volume: None,
                number: None,
                pages: None,
                publisher: publisher.clone(),
                editor: None,
                edition: None,
                isbn: None,
                booktitle: booktitle.clone(),
                doi: doi.clone(),
                url: None,
                abstract_text: None,
                tags: vec![],
                howpublished: None,
                month: None,
                note: None,
                collections: vec![],
                file_path: None,
                created_at: String::new(),
                updated_at: None,
            };
            format!("{}.pdf", entry_to_filename(&tmp_entry))
        } else {
            src_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        };

        let dest = config.bibox_dir.join(&filename);
        let dest = if dest.exists() {
            let stem = dest.file_stem().unwrap_or_default().to_string_lossy();
            config.bibox_dir.join(format!("{}_{}.pdf", stem, &bibtex_key))
        } else {
            dest
        };

        std::fs::copy(src_path, &dest).with_context(|| {
            config.msgs.file_copy_failed(
                &src_path.to_string_lossy(),
                &dest.to_string_lossy(),
            )
        })?;

        if src_path != &dest && !src_path.starts_with(&config.bibox_dir) {
            if temp_pdf_path.as_deref()
                != Some(std::env::temp_dir().join("bibox_download.pdf").as_path())
            {
                std::fs::remove_file(src_path).ok();
            }
        }

        println!(
            "{}",
            config
                .msgs
                .file_moved(&dest.file_name().unwrap_or_default().to_string_lossy())
        );

        Some(
            dest.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
        )
    } else {
        None
    };

    let collections: Vec<String> = to
        .or_else(|| config.default_collection.clone())
        .map(|c| vec![c])
        .unwrap_or_default();

    let entry = Entry {
        id: Uuid::new_v4().to_string(),
        bibtex_key: bibtex_key.clone(),
        entry_type,
        title: title.clone(),
        author: authors,
        year,
        journal,
        volume: meta.as_ref().and_then(|m| m.volume.clone()),
        number: meta.as_ref().and_then(|m| m.number.clone()),
        pages: meta.as_ref().and_then(|m| m.pages.clone()),
        publisher,
        editor: None,
        edition: None,
        isbn: None,
        booktitle,
        doi,
        url: meta.as_ref().and_then(|m| m.url.clone()),
        abstract_text: None,
        tags: vec![],
        howpublished: howpublished_arg,
        month: month_arg,
        note: None,
        collections,
        file_path,
        created_at: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        updated_at: None,
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&entry)?);
    } else {
        println!(
            "{}",
            config
                .msgs
                .added(&entry.bibtex_key, entry.title.as_deref().unwrap_or("?"))
        );
    }
    let add_key = entry.bibtex_key.clone();
    db.entries.push(entry);
    save_db(&db, &db_path)?;
    if config.git {
        git::auto_commit(&db_path, &format!("bibox: add {}", add_key))?;
    }

    Ok(())
}

// ── list ─────────────────────────────────────────────────────────────────────

pub fn cmd_list(
    collection: Option<String>,
    entry_type: Option<String>,
    tag: Option<String>,
    year: Option<u32>,
    limit: Option<usize>,
    json: bool,
    config: &Config,
) -> Result<()> {
    let db_path = db_path_from_config(config);
    let db = load_db(&db_path)?;

    // No collection + no filters → show collections summary
    if collection.is_none() && entry_type.is_none() && tag.is_none() && year.is_none() {
        if json {
            let mut collections: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
            for entry in &db.entries {
                for col in &entry.collections {
                    *collections.entry(col.clone()).or_insert(0) += 1;
                }
            }
            let output = serde_json::json!({
                "total": db.entries.len(),
                "collections": collections,
            });
            println!("{}", serde_json::to_string_pretty(&output)?);
            return Ok(());
        }
        return list_collections(&db, config);
    }

    let entries = filter_entries(
        &db,
        collection.as_deref(),
        entry_type.as_deref(),
        tag.as_deref(),
        year,
    );

    if json {
        let output: Vec<&Entry> = entries.iter().take(limit.unwrap_or(usize::MAX)).cloned().collect();
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    let page_size = limit.unwrap_or(config.default_page_size);
    let total = entries.len();

    if total == 0 {
        println!("{}", config.msgs.no_entries());
        return Ok(());
    }

    for entry in entries.iter().take(page_size) {
        print_entry_block(entry, config);
        println!();
    }

    if total > page_size {
        println!("{}", config.msgs.showing_of(page_size, total));
    } else {
        println!("{}", config.msgs.total(total));
    }

    Ok(())
}

fn list_collections(db: &crate::models::Database, config: &Config) -> Result<()> {
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    let mut uncollected = 0usize;

    for entry in &db.entries {
        if entry.collections.is_empty() {
            uncollected += 1;
        } else {
            for col in &entry.collections {
                *counts.entry(col.clone()).or_insert(0) += 1;
            }
        }
    }

    let total = db.entries.len();

    println!("{}", config.msgs.collections_header());
    println!();
    println!("{:<28} {}", "(all)", config.msgs.entry_count(total));
    for (name, count) in &counts {
        println!("{:<28} {}", name, config.msgs.entry_count(*count));
    }
    if uncollected > 0 {
        println!("{:<28} {}", "(uncollected)", config.msgs.entry_count(uncollected));
    }

    Ok(())
}

// ── search ───────────────────────────────────────────────────────────────────

pub fn cmd_search(
    query: String,
    collection: Option<String>,
    field: Option<String>,
    json: bool,
    config: &Config,
) -> Result<()> {
    let db_path = db_path_from_config(config);
    let db = load_db(&db_path)?;

    let entries = search_entries(
        &db,
        &query,
        field.as_deref(),
        collection.as_deref(),
        config.search_case_sensitive,
    );

    if json {
        let output: Vec<&Entry> = entries;
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    if entries.is_empty() {
        println!("{}", config.msgs.no_results(&query));
        return Ok(());
    }

    let items: Vec<SelectItem> = entries
        .iter()
        .map(|e| {
            let year = e
                .year
                .map(|y| y.to_string())
                .unwrap_or_else(|| "n.d.".to_string());
            let title = e.title.as_deref().unwrap_or(config.msgs.no_title());
            SelectItem {
                key: e.bibtex_key.clone(),
                display: format!(
                    "{:<20} {:<45} {}",
                    e.bibtex_key,
                    if title.len() > 45 {
                        format!("{}...", &title[..42])
                    } else {
                        title.to_string()
                    },
                    year
                ),
            }
        })
        .collect();

    if let Some(key) = interactive_select(&items)? {
        copy_to_clipboard(&key, config)?;
        println!("{}", config.msgs.copied_to_clipboard(&key));
    }

    Ok(())
}

// ── show ─────────────────────────────────────────────────────────────────────

pub fn cmd_show(id_or_key: String, json: bool, config: &Config) -> Result<()> {
    let db_path = db_path_from_config(config);
    let db = load_db(&db_path)?;

    let entry = find_by_key(&db, &id_or_key)
        .with_context(|| config.msgs.entry_not_found(&id_or_key))?;

    if json {
        println!("{}", serde_json::to_string_pretty(entry)?);
        return Ok(());
    }

    let sep = "─────────────────────────────────────────";
    println!("{}", sep);
    println!("{}: {}", config.msgs.label_key(), entry.bibtex_key);
    println!("{}: {}", config.msgs.label_id(), entry.id);
    println!("{}: {}", config.msgs.label_type(), entry.entry_type);
    if let Some(t) = &entry.title {
        println!("{}: {}", config.msgs.label_title(), t);
    }
    if !entry.author.is_empty() {
        println!("{}: {}", config.msgs.label_author(), entry.author.join("; "));
    }
    if let Some(y) = entry.year {
        println!("{}: {}", config.msgs.label_year(), y);
    }
    if let Some(j) = &entry.journal {
        println!("{}: {}", config.msgs.label_journal(), j);
    }
    if let Some(p) = &entry.publisher {
        println!("{}: {}", config.msgs.label_publisher(), p);
    }
    if let Some(bt) = &entry.booktitle {
        println!("{}: {}", config.msgs.label_booktitle(), bt);
    }
    if let Some(v) = &entry.volume {
        println!("{}: {}", config.msgs.label_volume(), v);
    }
    if let Some(n) = &entry.number {
        println!("{}: {}", config.msgs.label_number(), n);
    }
    if let Some(pg) = &entry.pages {
        println!("{}: {}", config.msgs.label_pages(), pg);
    }
    if let Some(d) = &entry.doi {
        println!("{}: {}", config.msgs.label_doi(), d);
    }
    if !entry.tags.is_empty() {
        println!("{}: {}", config.msgs.label_tags(), entry.tags.join(", "));
    }
    if !entry.collections.is_empty() {
        println!(
            "{}: {}",
            config.msgs.label_collections(),
            entry.collections.join(", ")
        );
    }
    if let Some(fp) = &entry.file_path {
        println!("{}: {}", config.msgs.label_file(), fp);
    }
    if let Some(hp) = &entry.howpublished {
        println!("{}: {}", config.msgs.label_howpublished(), hp);
    }
    if let Some(m) = &entry.month {
        println!("{}: {}", config.msgs.label_month(), m);
    }
    if let Some(note) = &entry.note {
        println!("{}: {}", config.msgs.label_note(), note);
    }
    println!("{}: {}", config.msgs.label_created(), entry.created_at);
    println!("{}", sep);

    Ok(())
}

// ── edit ─────────────────────────────────────────────────────────────────────

pub async fn cmd_edit(
    id_or_key: String,
    title: Option<String>,
    author: Option<String>,
    year: Option<u32>,
    doi: Option<String>,
    journal: Option<String>,
    publisher: Option<String>,
    booktitle: Option<String>,
    volume: Option<String>,
    number: Option<String>,
    pages: Option<String>,
    note: Option<String>,
    howpublished: Option<String>,
    month: Option<String>,
    tags_add: Option<String>,
    tags_remove: Option<String>,
    attach_pdf: Option<std::path::PathBuf>,
    config: &Config,
) -> Result<()> {
    let db_path = db_path_from_config(config);
    let mut db = load_db(&db_path)?;

    // If --doi provided, fetch from Crossref first (preserve-on-None)
    if let Some(ref doi_str) = doi {
        println!("{}", config.msgs.fetching_crossref());
        match crossref::fetch_metadata(doi_str).await {
            Ok(meta) => {
                let entry = find_by_key_mut(&mut db, &id_or_key)
                    .with_context(|| config.msgs.entry_not_found(&id_or_key))?;

                // CLI flags override Crossref; Crossref overrides existing; existing preserved if both None
                entry.title = title.or(meta.title).or(entry.title.take());
                if let Some(a) = author {
                    entry.author = a.split(';').map(|s| s.trim().to_string()).collect();
                } else if !meta.authors.is_empty() {
                    entry.author = meta.authors;
                }
                entry.year = year.or(meta.year).or(entry.year.take());
                entry.journal = journal.or(meta.journal).or(entry.journal.take());
                entry.publisher = publisher.or(meta.publisher).or(entry.publisher.take());
                entry.booktitle = booktitle.or(meta.booktitle).or(entry.booktitle.take());
                entry.doi = Some(doi_str.clone());
                entry.volume = volume.or(meta.volume).or(entry.volume.take());
                entry.number = number.or(meta.number).or(entry.number.take());
                entry.pages = pages.or(meta.pages).or(entry.pages.take());
                entry.url = meta.url.or(entry.url.take());

                if let Some(hp) = howpublished {
                    entry.howpublished = Some(hp);
                }
                if let Some(m) = month {
                    entry.month = Some(m);
                }
                if let Some(n) = note {
                    entry.note = Some(n);
                }
                if let Some(ta) = tags_add {
                    for tag in ta.split(',').map(|s| s.trim().to_string()) {
                        if !entry.tags.contains(&tag) {
                            entry.tags.push(tag);
                        }
                    }
                }
                if let Some(tr) = tags_remove {
                    let remove: Vec<&str> = tr.split(',').map(|s| s.trim()).collect();
                    entry.tags.retain(|t| !remove.contains(&t.as_str()));
                }
            }
            Err(e) => anyhow::bail!("{}", config.msgs.doi_lookup_failed(&e.to_string())),
        }
    } else {
        // No --doi: plain manual edit (original behavior)
        let entry = find_by_key_mut(&mut db, &id_or_key)
            .with_context(|| config.msgs.entry_not_found(&id_or_key))?;

        if let Some(t) = title { entry.title = Some(t); }
        if let Some(a) = author { entry.author = a.split(';').map(|s| s.trim().to_string()).collect(); }
        if let Some(y) = year { entry.year = Some(y); }
        if let Some(d) = doi { entry.doi = Some(d); }
        if let Some(j) = journal { entry.journal = Some(j); }
        if let Some(p) = publisher { entry.publisher = Some(p); }
        if let Some(bt) = booktitle { entry.booktitle = Some(bt); }
        if let Some(v) = volume { entry.volume = Some(v); }
        if let Some(n) = number { entry.number = Some(n); }
        if let Some(pg) = pages { entry.pages = Some(pg); }
        if let Some(hp) = howpublished { entry.howpublished = Some(hp); }
        if let Some(m) = month { entry.month = Some(m); }
        if let Some(n) = note { entry.note = Some(n); }
        if let Some(ta) = tags_add {
            for tag in ta.split(',').map(|s| s.trim().to_string()) {
                if !entry.tags.contains(&tag) { entry.tags.push(tag); }
            }
        }
        if let Some(tr) = tags_remove {
            let remove: Vec<&str> = tr.split(',').map(|s| s.trim()).collect();
            entry.tags.retain(|t| !remove.contains(&t.as_str()));
        }
    }

    // Rename file if metadata changed + attach PDF if requested
    let entry = find_by_key_mut(&mut db, &id_or_key)
        .with_context(|| config.msgs.entry_not_found(&id_or_key))?;

    if let Some(src) = attach_pdf {
        let filename = format!("{}.pdf", entry_to_filename(entry));
        std::fs::create_dir_all(&config.bibox_dir)?;
        let dest = config.bibox_dir.join(&filename);
        std::fs::copy(&src, &dest)
            .with_context(|| format!("Failed to copy PDF from {}", src.display()))?;
        entry.file_path = Some(filename.clone());
        println!("PDF attached: {}", dest.display());
    } else if entry.file_path.is_some() {
        let new_filename = format!("{}.pdf", entry_to_filename(entry));
        let old_fp = entry.file_path.as_ref().unwrap().clone();
        if old_fp != new_filename {
            let old_path = config.bibox_dir.join(&old_fp);
            let new_path = config.bibox_dir.join(&new_filename);
            if old_path.exists() {
                std::fs::rename(&old_path, &new_path).with_context(|| {
                    config.msgs.file_rename_failed(&old_path.to_string_lossy())
                })?;
                entry.file_path = Some(new_filename.clone());
                println!("{}", config.msgs.file_renamed(&old_fp, &new_filename));
            }
        }
    }

    let key = entry.bibtex_key.clone();
    entry.updated_at = Some(Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
    save_db(&db, &db_path)?;
    if config.git {
        git::auto_commit(&db_path, &format!("bibox: edit {}", key))?;
    }
    println!("{}", config.msgs.updated(&key));

    Ok(())
}

// ── delete ───────────────────────────────────────────────────────────────────

pub fn cmd_delete(id_or_key: String, force: bool, config: &Config) -> Result<()> {
    let db_path = db_path_from_config(config);
    let mut db = load_db(&db_path)?;

    let entry = find_by_key(&db, &id_or_key)
        .with_context(|| config.msgs.entry_not_found(&id_or_key))?
        .clone();

    if !force {
        let title = entry.title.as_deref().unwrap_or(config.msgs.no_title());
        if !prompt_confirm(&config.msgs.delete_prompt(&entry.bibtex_key, title)) {
            println!("{}", config.msgs.cancelled());
            return Ok(());
        }
    }

    if let Some(fp) = &entry.file_path {
        let path = config.bibox_dir.join(fp);
        if path.exists() {
            std::fs::remove_file(&path)?;
            println!("{}", config.msgs.file_deleted(fp));
        }
    }

    let del_key = entry.bibtex_key.clone();
    db.entries
        .retain(|e| e.bibtex_key != entry.bibtex_key && e.id != entry.id);
    save_db(&db, &db_path)?;
    if config.git {
        git::auto_commit(&db_path, &format!("bibox: delete {}", del_key))?;
    }
    println!("{}", config.msgs.deleted(&del_key));

    Ok(())
}

// ── collect / uncollect ───────────────────────────────────────────────────────

pub fn cmd_collect(id_or_key: String, collections: Vec<String>, config: &Config) -> Result<()> {
    let db_path = db_path_from_config(config);
    let mut db = load_db(&db_path)?;

    let entry = find_by_key_mut(&mut db, &id_or_key)
        .with_context(|| config.msgs.entry_not_found(&id_or_key))?;

    let mut added = vec![];
    let mut skipped = vec![];

    for col in collections {
        if entry.collections.contains(&col) {
            skipped.push(col);
        } else {
            entry.collections.push(col.clone());
            added.push(col);
        }
    }

    let key = entry.bibtex_key.clone();
    if !added.is_empty() {
        entry.updated_at = Some(Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
    }
    save_db(&db, &db_path)?;

    if !added.is_empty() {
        println!("{}", config.msgs.collect_added(&key, &added.join(", ")));
    }
    if !skipped.is_empty() {
        println!("{}", config.msgs.collect_skipped(&key, &skipped.join(", ")));
    }

    Ok(())
}

pub fn cmd_uncollect(id_or_key: String, collection: String, config: &Config) -> Result<()> {
    let db_path = db_path_from_config(config);
    let mut db = load_db(&db_path)?;

    let entry = find_by_key_mut(&mut db, &id_or_key)
        .with_context(|| config.msgs.entry_not_found(&id_or_key))?;

    if !entry.collections.contains(&collection) {
        println!(
            "{}",
            config.msgs.not_in_collection(&id_or_key, &collection)
        );
        return Ok(());
    }

    entry.collections.retain(|c| c != &collection);
    entry.updated_at = Some(Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
    let key = entry.bibtex_key.clone();
    save_db(&db, &db_path)?;
    println!("{}", config.msgs.uncollected(&key, &collection));

    Ok(())
}

// ── import ───────────────────────────────────────────────────────────────────

pub fn cmd_import(file: PathBuf, to: Option<String>, config: &Config) -> Result<()> {
    let db_path = db_path_from_config(config);
    let mut db = load_db(&db_path)?;

    let content = std::fs::read_to_string(&file)
        .with_context(|| config.msgs.file_read_failed(&file.to_string_lossy()))?;

    let mut added = 0;
    let mut merged: Vec<String> = vec![];
    let mut skipped: Vec<String> = vec![];

    let entries = parse_bibtex(&content);

    for mut raw in entries {
        let entry_type: EntryType = raw.entry_type.parse().unwrap_or(EntryType::Misc);

        let authors: Vec<String> = raw
            .author
            .as_deref()
            .unwrap_or("")
            .split(" and ")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let has_required = match entry_type {
            EntryType::Article => {
                raw.title.is_some() && !authors.is_empty() && raw.year.is_some()
            }
            EntryType::Book => {
                raw.title.is_some() && !authors.is_empty() && raw.year.is_some()
            }
            EntryType::InProceedings => {
                raw.title.is_some() && !authors.is_empty() && raw.year.is_some()
            }
            EntryType::Misc => true,
        };

        if !has_required {
            skipped.push(format!(
                "{} {}",
                raw.key
                    .unwrap_or_else(|| config.msgs.no_key().to_string()),
                config.msgs.no_required_fields()
            ));
            continue;
        }

        // Duplicate DOI check — merge missing fields instead of skipping
        if let Some(ref doi) = raw.doi {
            let doi_norm = doi.trim().to_lowercase();
            if let Some(idx) = db.entries.iter().position(|e| {
                e.doi.as_ref()
                    .map(|d| d.trim().to_lowercase() == doi_norm)
                    .unwrap_or(false)
            }) {
                let existing_key = db.entries[idx].bibtex_key.clone();
                let mut n = 0usize;
                let e = &mut db.entries[idx];
                if e.title.is_none() { if let Some(v) = raw.title.take() { e.title = Some(v); n += 1; } }
                if e.author.is_empty() && !authors.is_empty() { e.author = authors.clone(); n += 1; }
                if e.year.is_none() { if let Some(v) = raw.year.take() { e.year = Some(v); n += 1; } }
                if e.journal.is_none() { if let Some(v) = raw.journal.take() { e.journal = Some(v); n += 1; } }
                if e.volume.is_none() { if let Some(v) = raw.volume.take() { e.volume = Some(v); n += 1; } }
                if e.number.is_none() { if let Some(v) = raw.number.take() { e.number = Some(v); n += 1; } }
                if e.pages.is_none() { if let Some(v) = raw.pages.take() { e.pages = Some(v); n += 1; } }
                if e.publisher.is_none() { if let Some(v) = raw.publisher.take() { e.publisher = Some(v); n += 1; } }
                if e.editor.is_none() { if let Some(v) = raw.editor.take() { e.editor = Some(v); n += 1; } }
                if e.edition.is_none() { if let Some(v) = raw.edition.take() { e.edition = Some(v); n += 1; } }
                if e.isbn.is_none() { if let Some(v) = raw.isbn.take() { e.isbn = Some(v); n += 1; } }
                if e.booktitle.is_none() { if let Some(v) = raw.booktitle.take() { e.booktitle = Some(v); n += 1; } }
                if e.url.is_none() { if let Some(v) = raw.url.take() { e.url = Some(v); n += 1; } }
                if e.note.is_none() { if let Some(v) = raw.note.take() { e.note = Some(v); n += 1; } }
                if e.howpublished.is_none() { if let Some(v) = raw.howpublished.take() { e.howpublished = Some(v); n += 1; } }
                if e.month.is_none() { if let Some(v) = raw.month.take() { e.month = Some(v); n += 1; } }
                if n > 0 {
                    merged.push(config.msgs.merged_fields(&existing_key, n));
                } else {
                    skipped.push(config.msgs.already_exists(&existing_key));
                }
                continue;
            }
        }

        let base_key = raw.key.unwrap_or_else(|| {
            generate_bibtex_key(
                &authors,
                raw.year,
                raw.title.as_deref().unwrap_or("unknown"),
            )
        });
        let bibtex_key = generate_unique_key(&db, &base_key);
        let collections: Vec<String> = to.clone().map(|c| vec![c]).unwrap_or_default();

        let entry = Entry {
            id: Uuid::new_v4().to_string(),
            bibtex_key,
            entry_type,
            title: raw.title,
            author: authors,
            year: raw.year,
            journal: raw.journal,
            volume: raw.volume,
            number: raw.number,
            pages: raw.pages,
            publisher: raw.publisher,
            editor: raw.editor,
            edition: raw.edition,
            isbn: raw.isbn,
            booktitle: raw.booktitle,
            doi: raw.doi,
            url: raw.url,
            abstract_text: raw.abstract_text,
            tags: raw.keywords
                .map(|k| k.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
                .unwrap_or_default(),
            howpublished: raw.howpublished,
            month: raw.month,
            note: raw.note,
            collections,
            file_path: None,
            created_at: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            updated_at: None,
        };

        db.entries.push(entry);
        added += 1;
    }

    save_db(&db, &db_path)?;
    if config.git {
        git::auto_commit(&db_path, &format!("bibox: import {} entries", added))?;
    }

    println!("{}", config.msgs.import_complete(added));
    if !merged.is_empty() {
        for m in &merged {
            println!("  ~ {}", m);
        }
    }
    if !skipped.is_empty() {
        println!("{}", config.msgs.skipped_header(skipped.len()));
        for s in &skipped {
            println!("  - {}", s);
        }
    }

    Ok(())
}

// ── out ──────────────────────────────────────────────────────────────────────

pub fn cmd_export(
    keys: Vec<String>,
    collection: Option<String>,
    output: Option<PathBuf>,
    clipboard: bool,
    entry_type: Option<String>,
    tag: Option<String>,
    as_pdf: bool,
    include_pdf: bool,
    zip: bool,
    format: String,
    notes_only: bool,
    config: &Config,
) -> Result<()> {
    let db_path = db_path_from_config(config);
    let db = load_db(&db_path)?;

    let timestamp = Local::now().format("%Y%m%d_%H%M%S").to_string();

    // ── Resolve entries ──
    let entries: Vec<&Entry> = if !keys.is_empty() {
        keys.iter()
            .map(|k| find_by_key(&db, k).with_context(|| config.msgs.entry_not_found(k)))
            .collect::<Result<Vec<_>>>()?
    } else if collection.is_some() || entry_type.is_some() || tag.is_some() {
        filter_entries(&db, collection.as_deref(), entry_type.as_deref(), tag.as_deref(), None)
    } else {
        db.entries.iter().collect()
    };

    let col_name = if !keys.is_empty() {
        if keys.len() == 1 { keys[0].as_str() } else { "selected" }
    } else {
        collection.as_deref().unwrap_or("references")
    };

    // ── PDF-only export (--as-pdf) ──
    if as_pdf {
        return export_pdfs(&entries, col_name, &timestamp, output, zip, config);
    }

    // ── Notes-only export (--notes-only) ──
    if notes_only {
        let dest_dir = output.clone().unwrap_or_else(|| config.export_dir.clone());
        std::fs::create_dir_all(&dest_dir)?;
        let mut copied = 0;
        let mut missing = 0;
        for entry in &entries {
            let note_path = config.notes_dir.join(format!("{}.md", entry.bibtex_key));
            if note_path.exists() {
                let dest = dest_dir.join(format!("{}.md", entry.bibtex_key));
                std::fs::copy(&note_path, &dest)?;
                copied += 1;
            } else {
                missing += 1;
            }
        }
        let abs_dir = dest_dir.canonicalize().unwrap_or(dest_dir);
        println!(
            "Exported {} note(s) to {} ({} entries had no note)",
            copied,
            abs_dir.display(),
            missing
        );
        return Ok(());
    }

    let fmt = format.to_lowercase();

    // Clipboard only supported for BibTeX
    if clipboard {
        if fmt != "bibtex" {
            println!("Clipboard only supported for BibTeX format");
            return Ok(());
        }
        let bibtex = entries_to_bibtex(&entries);
        copy_to_clipboard(&bibtex, config)?;
        println!("{}", config.msgs.clipboard_copied_entries(entries.len()));
        return Ok(());
    }

    match fmt.as_str() {
        "bibtex" => {
            let bibtex = entries_to_bibtex(&entries);
            if let Some(ref path) = output {
                std::fs::write(path, &bibtex)?;
                eprintln!("{}", config.msgs.bibtex_saved(&path.to_string_lossy(), entries.len()));
            } else {
                print!("{}", bibtex);
                eprintln!("Exported {} entries as BibTeX to stdout", entries.len());
            }
        }
        "yaml" => {
            let yaml = entries_to_yaml(&entries);
            if let Some(ref path) = output {
                std::fs::write(path, &yaml)?;
                eprintln!("Exported {} entries to {}", entries.len(), path.display());
            } else {
                print!("{}", yaml);
                eprintln!("Exported {} entries as YAML to stdout", entries.len());
            }
        }
        "ris" => {
            let ris = entries_to_ris(&entries);
            if let Some(ref path) = output {
                std::fs::write(path, &ris)?;
                eprintln!("Exported {} entries to {}", entries.len(), path.display());
            } else {
                print!("{}", ris);
                eprintln!("Exported {} entries as RIS to stdout", entries.len());
            }
        }
        "csv" => {
            let csv = entries_to_csv(&entries);
            if let Some(path) = output.as_ref() {
                std::fs::write(path, &csv)?;
                println!("Exported {} entries to {}", entries.len(), path.display());
            } else {
                print!("{}", csv);
            }
        }
        other => {
            anyhow::bail!(
                "Unknown format '{}'. Supported formats: bibtex, yaml, ris, csv",
                other
            );
        }
    }

    // ── --include-pdf: also copy PDFs alongside the bibliography file ──
    if include_pdf {
        let dest_parent = output.as_ref()
            .and_then(|p| p.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| config.export_dir.clone());
        let folder_name = format!("{}_pdfs_{}", col_name, &timestamp[..8]);
        let dest_dir = dest_parent.join(&folder_name);
        std::fs::create_dir_all(&dest_dir)?;

        let mut copied = 0;
        for entry in &entries {
            if let Some(fp) = &entry.file_path {
                let src = config.bibox_dir.join(fp);
                if src.exists() {
                    let dst = dest_dir.join(fp);
                    std::fs::copy(&src, &dst)?;
                    copied += 1;
                }
            }
        }
        println!("{}", config.msgs.folder_created(&dest_dir.to_string_lossy(), copied));
    }

    Ok(())
}

fn export_pdfs(
    entries: &[&Entry],
    col_name: &str,
    timestamp: &str,
    output: Option<PathBuf>,
    zip: bool,
    config: &Config,
) -> Result<()> {
    let folder_name = format!("{}_{}", col_name, &timestamp[..8]);
    let dest_parent = output.unwrap_or_else(|| config.export_dir.clone());
    let dest_dir = dest_parent.join(&folder_name);
    std::fs::create_dir_all(&dest_dir)?;

    let mut copied = 0;
    for entry in entries {
        if let Some(fp) = &entry.file_path {
            let src = config.bibox_dir.join(fp);
            if src.exists() {
                let dst = dest_dir.join(fp);
                std::fs::copy(&src, &dst)?;
                copied += 1;
            }
        }
    }

    if zip {
        let zip_path = dest_parent.join(format!("{}.zip", folder_name));
        create_zip(&dest_dir, &zip_path)?;
        std::fs::remove_dir_all(&dest_dir)?;
        println!("{}", config.msgs.zip_created(&zip_path.to_string_lossy(), copied));
    } else {
        println!("{}", config.msgs.folder_created(&dest_dir.to_string_lossy(), copied));
    }
    Ok(())
}

// ── export helpers ────────────────────────────────────────────────────────────

fn entries_to_yaml(entries: &[&Entry]) -> String {
    let mut out = String::new();
    for entry in entries {
        out.push_str("---\n");
        out.push_str(&format!("key: {}\n", yaml_scalar(&entry.bibtex_key)));
        out.push_str(&format!("type: {}\n", yaml_scalar(&entry.entry_type.to_string())));
        if let Some(ref t) = entry.title {
            out.push_str(&format!("title: {}\n", yaml_scalar(t)));
        }
        if !entry.author.is_empty() {
            out.push_str("authors:\n");
            for a in &entry.author {
                out.push_str(&format!("  - {}\n", yaml_scalar(a)));
            }
        }
        if let Some(y) = entry.year {
            out.push_str(&format!("year: {}\n", y));
        }
        if let Some(ref j) = entry.journal {
            out.push_str(&format!("journal: {}\n", yaml_scalar(j)));
        }
        if let Some(ref v) = entry.volume {
            out.push_str(&format!("volume: {}\n", yaml_scalar(v)));
        }
        if let Some(ref n) = entry.number {
            out.push_str(&format!("number: {}\n", yaml_scalar(n)));
        }
        if let Some(ref p) = entry.pages {
            out.push_str(&format!("pages: {}\n", yaml_scalar(p)));
        }
        if let Some(ref p) = entry.publisher {
            out.push_str(&format!("publisher: {}\n", yaml_scalar(p)));
        }
        if let Some(ref bt) = entry.booktitle {
            out.push_str(&format!("booktitle: {}\n", yaml_scalar(bt)));
        }
        if let Some(ref d) = entry.doi {
            out.push_str(&format!("doi: {}\n", yaml_scalar(d)));
        }
        if let Some(ref u) = entry.url {
            out.push_str(&format!("url: {}\n", yaml_scalar(u)));
        }
        if let Some(ref hp) = entry.howpublished {
            out.push_str(&format!("howpublished: {}\n", yaml_scalar(hp)));
        }
        if let Some(ref m) = entry.month {
            out.push_str(&format!("month: {}\n", yaml_scalar(m)));
        }
        if let Some(ref isbn) = entry.isbn {
            out.push_str(&format!("isbn: {}\n", yaml_scalar(isbn)));
        }
        if let Some(ref ed) = entry.editor {
            out.push_str(&format!("editor: {}\n", yaml_scalar(ed)));
        }
        if let Some(ref ed) = entry.edition {
            out.push_str(&format!("edition: {}\n", yaml_scalar(ed)));
        }
        if let Some(ref note) = entry.note {
            out.push_str(&format!("note: {}\n", yaml_scalar(note)));
        }
        if !entry.tags.is_empty() {
            out.push_str("tags:\n");
            for tag in &entry.tags {
                out.push_str(&format!("  - {}\n", yaml_scalar(tag)));
            }
        }
        if !entry.collections.is_empty() {
            out.push_str("collections:\n");
            for col in &entry.collections {
                out.push_str(&format!("  - {}\n", yaml_scalar(col)));
            }
        }
    }
    out
}

/// Wrap a string in single-quotes if it contains special YAML characters,
/// or return as a plain scalar. Escapes internal single-quotes.
fn yaml_scalar(s: &str) -> String {
    let needs_quoting = s.contains(':')
        || s.contains('#')
        || s.contains('"')
        || s.contains('\'')
        || s.contains('\n')
        || s.starts_with('{')
        || s.starts_with('[')
        || s.starts_with('&')
        || s.starts_with('*')
        || s.starts_with('!')
        || s.starts_with('|')
        || s.starts_with('>')
        || s.is_empty();
    if needs_quoting {
        // Use double-quote style; escape backslash and double-quote
        let escaped = s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n");
        format!("\"{}\"", escaped)
    } else {
        s.to_string()
    }
}

fn entries_to_ris(entries: &[&Entry]) -> String {
    let mut out = String::new();
    for entry in entries {
        // TY field
        let ty = match entry.entry_type {
            crate::models::EntryType::Article => "JOUR",
            crate::models::EntryType::Book => "BOOK",
            crate::models::EntryType::InProceedings => "CONF",
            crate::models::EntryType::Misc => "GEN",
        };
        out.push_str(&format!("TY  - {}\n", ty));
        out.push_str(&format!("ID  - {}\n", entry.bibtex_key));
        if let Some(ref t) = entry.title {
            out.push_str(&format!("TI  - {}\n", t));
        }
        for author in &entry.author {
            out.push_str(&format!("AU  - {}\n", author.trim()));
        }
        if let Some(y) = entry.year {
            out.push_str(&format!("PY  - {}\n", y));
        }
        if let Some(ref j) = entry.journal {
            out.push_str(&format!("JO  - {}\n", j));
        }
        if let Some(ref v) = entry.volume {
            out.push_str(&format!("VL  - {}\n", v));
        }
        if let Some(ref n) = entry.number {
            out.push_str(&format!("IS  - {}\n", n));
        }
        if let Some(ref p) = entry.pages {
            // Split pages into SP/EP if a dash is present
            if let Some(idx) = p.find('-') {
                let sp = p[..idx].trim();
                // skip over multiple dashes (e.g. em-dash --)
                let rest = p[idx..].trim_start_matches('-').trim();
                if !sp.is_empty() {
                    out.push_str(&format!("SP  - {}\n", sp));
                }
                if !rest.is_empty() {
                    out.push_str(&format!("EP  - {}\n", rest));
                }
            } else {
                out.push_str(&format!("SP  - {}\n", p));
            }
        }
        if let Some(ref d) = entry.doi {
            out.push_str(&format!("DO  - {}\n", d));
        }
        if let Some(ref isbn) = entry.isbn {
            out.push_str(&format!("SN  - {}\n", isbn));
        }
        if let Some(ref pub_) = entry.publisher {
            out.push_str(&format!("PB  - {}\n", pub_));
        }
        if let Some(ref u) = entry.url {
            out.push_str(&format!("UR  - {}\n", u));
        }
        if let Some(ref m) = entry.month {
            out.push_str(&format!("DA  - {}\n", m));
        }
        if let Some(ref note) = entry.note {
            out.push_str(&format!("N1  - {}\n", note));
        }
        out.push_str("ER  - \n\n");
    }
    out
}

fn entries_to_csv(entries: &[&Entry]) -> String {
    let mut out = String::new();
    out.push_str("key,type,title,authors,year,month,journal,doi,howpublished,tags,collections\n");
    for entry in entries {
        let title = entry.title.as_deref().unwrap_or("");
        let authors = entry.author.join("; ");
        let year = entry
            .year
            .map(|y| y.to_string())
            .unwrap_or_default();
        let month = entry.month.as_deref().unwrap_or("");
        let journal = entry.journal.as_deref().unwrap_or("");
        let doi = entry.doi.as_deref().unwrap_or("");
        let howpublished = entry.howpublished.as_deref().unwrap_or("");
        let tags = entry.tags.join("; ");
        let collections = entry.collections.join("; ");

        out.push_str(&csv_field(&entry.bibtex_key));
        out.push(',');
        out.push_str(&csv_field(&entry.entry_type.to_string()));
        out.push(',');
        out.push_str(&csv_field(title));
        out.push(',');
        out.push_str(&csv_field(&authors));
        out.push(',');
        out.push_str(&csv_field(&year));
        out.push(',');
        out.push_str(&csv_field(month));
        out.push(',');
        out.push_str(&csv_field(journal));
        out.push(',');
        out.push_str(&csv_field(doi));
        out.push(',');
        out.push_str(&csv_field(howpublished));
        out.push(',');
        out.push_str(&csv_field(&tags));
        out.push(',');
        out.push_str(&csv_field(&collections));
        out.push('\n');
    }
    out
}

/// Wrap a CSV field in double-quotes if it contains commas, quotes, or newlines.
/// Internal double-quotes are escaped as `""`.
fn csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

// ── open ─────────────────────────────────────────────────────────────────────

pub fn cmd_open(id_or_key: String, config: &Config) -> Result<()> {
    let db_path = db_path_from_config(config);
    let db = load_db(&db_path)?;

    let entry = find_by_key(&db, &id_or_key)
        .with_context(|| config.msgs.entry_not_found(&id_or_key))?;

    let fp = entry
        .file_path
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("No PDF file associated with '{}'", id_or_key))?;

    let full_path = config.bibox_dir.join(fp);
    if !full_path.exists() {
        anyhow::bail!("PDF file not found: {}", full_path.display());
    }

    let path_str = full_path.to_string_lossy().to_string();

    if let Some(viewer) = &config.pdf_viewer {
        std::process::Command::new(viewer)
            .arg(&path_str)
            .spawn()
            .with_context(|| format!("Failed to launch viewer '{}'", viewer))?;
    } else {
        #[cfg(target_os = "macos")]
        std::process::Command::new("open")
            .arg(&path_str)
            .spawn()
            .context("Failed to run 'open'")?;

        #[cfg(not(target_os = "macos"))]
        std::process::Command::new("xdg-open")
            .arg(&path_str)
            .spawn()
            .context("Failed to run 'xdg-open'")?;
    }

    println!("Opening: {}", full_path.display());
    Ok(())
}

// ── sync ─────────────────────────────────────────────────────────────────────

pub fn cmd_init(path: PathBuf, migrate: bool, json: bool, config: &Config) -> Result<()> {
    let home = if path.to_string_lossy().starts_with("~/") {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(&path.to_string_lossy()[2..])
    } else {
        std::fs::canonicalize(&path).unwrap_or(path.clone())
    };

    // Create directory structure
    std::fs::create_dir_all(home.join("pdfs"))?;
    std::fs::create_dir_all(home.join("notes"))?;

    // Create db.json if not exists
    let db_file = home.join("db.json");
    if !db_file.exists() {
        std::fs::write(&db_file, "{\"entries\":[]}")?;
    }

    // Migrate existing data if requested
    if migrate {
        let old_db_path = crate::config::db_path();
        if old_db_path.exists() && !db_file.exists() {
            println!("Migrating database...");
            std::fs::copy(&old_db_path, &db_file)?;
        } else if old_db_path.exists() {
            // Merge: load old, load new, combine
            let old_db = load_db(&old_db_path)?;
            let mut new_db = load_db(&db_file)?;
            let existing_keys: std::collections::HashSet<String> =
                new_db.entries.iter().map(|e| e.bibtex_key.clone()).collect();
            for entry in old_db.entries {
                if !existing_keys.contains(&entry.bibtex_key) {
                    new_db.entries.push(entry);
                }
            }
            save_db(&new_db, &db_file)?;
            println!("Merged {} entries into new home.", new_db.entries.len());
        }

        // Copy PDFs
        if config.bibox_dir.exists() {
            let dest_pdfs = home.join("pdfs");
            let mut copied = 0;
            for entry in std::fs::read_dir(&config.bibox_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().map(|e| e == "pdf").unwrap_or(false) {
                    let dest = dest_pdfs.join(entry.file_name());
                    if !dest.exists() {
                        std::fs::copy(&path, &dest)?;
                        copied += 1;
                    }
                }
            }
            if copied > 0 { println!("Copied {} PDFs.", copied); }
        }

        // Copy notes
        if config.notes_dir.exists() {
            let dest_notes = home.join("notes");
            let mut copied = 0;
            for entry in std::fs::read_dir(&config.notes_dir)? {
                let entry = entry?;
                let dest = dest_notes.join(entry.file_name());
                if !dest.exists() {
                    std::fs::copy(entry.path(), &dest)?;
                    copied += 1;
                }
            }
            if copied > 0 { println!("Copied {} notes.", copied); }
        }
    }

    // Update config.toml with home path
    let mut new_config = crate::config::load_config()?;
    new_config.home = Some(home.clone());
    crate::config::save_config(&new_config)?;

    if json {
        let result = serde_json::json!({
            "home": home.to_string_lossy(),
            "db": home.join("db.json").to_string_lossy().to_string(),
            "pdfs": home.join("pdfs").to_string_lossy().to_string(),
            "notes": home.join("notes").to_string_lossy().to_string(),
            "status": "created",
            "migrated": migrate,
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("Initialized bibox home: {}", home.display());
        println!("  pdfs/    — PDF files");
        println!("  notes/   — Markdown notes");
        println!("  db.json  — Database");
        println!();
        println!("To sync with GitHub:");
        println!("  cd {} && git init && git add . && git commit -m 'init bibox'", home.display());
    }

    Ok(())
}

pub fn cmd_sync(yes: bool, json: bool, config: &Config) -> Result<()> {
    let db_path = db_path_from_config(config);
    let mut db = load_db(&db_path)?;

    std::fs::create_dir_all(&config.bibox_dir)?;

    // Entries whose stored filename doesn't match the canonical name (metadata drift)
    let mut renamed_count = 0usize;
    for entry in db.entries.iter_mut() {
        if let Some(ref fp) = entry.file_path.clone() {
            let canonical = format!("{}.pdf", crate::bibtex::entry_to_filename(entry));
            if *fp != canonical {
                let old_path = config.bibox_dir.join(fp);
                let new_path = config.bibox_dir.join(&canonical);
                if old_path.exists() && !new_path.exists() {
                    std::fs::rename(&old_path, &new_path).ok();
                    entry.file_path = Some(canonical.clone());
                    println!("{}", config.msgs.file_renamed(fp, &canonical));
                    renamed_count += 1;
                }
            }
        }
    }
    if renamed_count > 0 {
        println!("Renamed {} file(s) to match current metadata.", renamed_count);
    }

    // Re-compute actual/db file lists after renames
    let actual_files: Vec<String> = std::fs::read_dir(&config.bibox_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path().is_file()
                && e.path()
                    .extension()
                    .map(|x| x == "pdf")
                    .unwrap_or(false)
        })
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();

    let db_files: Vec<String> = db
        .entries
        .iter()
        .filter_map(|e| e.file_path.clone())
        .collect();

    // Files in DB but not on disk
    let mut missing: Vec<String> = db_files
        .iter()
        .filter(|fp| !actual_files.contains(fp))
        .cloned()
        .collect();

    // Files on disk but not in DB
    let mut untracked: Vec<String> = actual_files
        .iter()
        .filter(|fp| !db_files.contains(fp))
        .cloned()
        .collect();

    // ── Detect externally renamed files via DOI matching ──
    // For each missing entry that has a DOI, scan untracked files for a DOI match.
    let missing_with_doi: Vec<(String, String)> = db.entries.iter()
        .filter(|e| e.file_path.as_ref().map_or(false, |fp| missing.contains(fp)))
        .filter_map(|e| {
            e.doi.as_ref().map(|doi| (e.file_path.as_ref().unwrap().clone(), doi.to_lowercase()))
        })
        .collect();

    let mut matched_old: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut matched_new: std::collections::HashSet<String> = std::collections::HashSet::new();

    if !missing_with_doi.is_empty() {
        for new_fp in &untracked {
            let full_path = config.bibox_dir.join(new_fp);
            if let Ok(Some(file_doi)) = pdf::extract_doi(&full_path) {
                let file_doi_lc = file_doi.to_lowercase();
                for (old_fp, entry_doi) in &missing_with_doi {
                    if file_doi_lc == *entry_doi {
                        if let Some(entry) = db.entries.iter_mut()
                            .find(|e| e.file_path.as_deref() == Some(old_fp.as_str()))
                        {
                            println!("Detected rename: {} → {}", old_fp, new_fp);
                            entry.file_path = Some(new_fp.clone());
                            matched_old.insert(old_fp.clone());
                            matched_new.insert(new_fp.clone());
                        }
                        break;
                    }
                }
            }
        }
    }

    // Remove matched entries from missing/untracked so they aren't double-processed
    missing.retain(|fp| !matched_old.contains(fp));
    untracked.retain(|fp| !matched_new.contains(fp));

    if !matched_old.is_empty() {
        println!("Matched {} externally renamed file(s) by DOI.", matched_old.len());
    }

    for fp in &missing {
        if yes || prompt_confirm(&config.msgs.sync_file_missing(fp)) {
            db.entries.retain(|e| e.file_path.as_deref() != Some(fp));
            println!("{}", config.msgs.sync_removed(fp));
        }
    }

    for fp in &untracked {
        println!("{}", config.msgs.sync_new_file(fp));
        if yes || prompt_confirm(config.msgs.add_to_db_prompt()) {
            let full_path = config.bibox_dir.join(fp);
            let doi = pdf::extract_doi(&full_path).ok().flatten();

            let key = generate_unique_key(
                &db,
                &fp.trim_end_matches(".pdf")
                    .chars()
                    .filter(|c| c.is_alphanumeric() || *c == '_')
                    .collect::<String>()
                    .to_lowercase(),
            );

            let entry = Entry {
                id: Uuid::new_v4().to_string(),
                bibtex_key: key.clone(),
                entry_type: EntryType::Misc,
                title: Some(fp.trim_end_matches(".pdf").to_string()),
                author: vec![],
                year: None,
                journal: None,
                volume: None,
                number: None,
                pages: None,
                publisher: None,
                editor: None,
                edition: None,
                isbn: None,
                booktitle: None,
                doi,
                url: None,
                abstract_text: None,
                tags: vec![],
                howpublished: None,
                month: None,
                note: Some(config.msgs.sync_added_note().to_string()),
                collections: vec![],
                file_path: Some(fp.clone()),
                created_at: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                updated_at: None,
            };

            println!("{}", config.msgs.sync_entry_added(&key));
            db.entries.push(entry);
        }
    }

    save_db(&db, &db_path)?;
    if json {
        let result = serde_json::json!({
            "status": "complete",
            "renamed": renamed_count,
            "removed": missing.len(),
            "added": untracked.len(),
            "total_entries": db.entries.len(),
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("{}", config.msgs.sync_complete());
    }

    Ok(())
}

// ── note ─────────────────────────────────────────────────────────────────────

pub fn cmd_note(
    id_or_key: String,
    stdin: bool,
    from: Option<PathBuf>,
    section: Option<String>,
    template: Option<String>,
    show: bool,
    path: bool,
    force: bool,
    json: bool,
    config: &Config,
) -> Result<()> {
    let db_path = db_path_from_config(config);
    let mut db = load_db(&db_path)?;

    let entry = find_by_key(&db, &id_or_key)
        .with_context(|| config.msgs.entry_not_found(&id_or_key))?
        .clone();

    let notes_dir = &config.notes_dir;
    std::fs::create_dir_all(notes_dir)?;
    let note_path = notes_dir.join(format!("{}.md", entry.bibtex_key));

    // ── Validate flag combinations ──
    if section.is_some() && !stdin && from.is_none() {
        anyhow::bail!("{}", config.msgs.section_requires_source());
    }

    // ── Read-only modes ──
    if path {
        if json {
            let result = serde_json::json!({
                "path": note_path.to_string_lossy(),
                "exists": note_path.exists(),
                "citekey": entry.bibtex_key,
            });
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            println!("{}", note_path.display());
        }
        return Ok(());
    }

    if show {
        if !note_path.exists() {
            anyhow::bail!("{}", config.msgs.note_not_found(&id_or_key));
        }
        let content = std::fs::read_to_string(&note_path)?;
        if json {
            let result = serde_json::json!({
                "citekey": entry.bibtex_key,
                "path": note_path.to_string_lossy(),
                "content": content,
            });
            println!("{}", serde_json::to_string_pretty(&result)?);
        } else {
            print!("{}", content);
        }
        return Ok(());
    }

    // ── Order of operations: 1) Template  2) Content write  3) Editor ──

    // Step 1: Template
    if let Some(ref tmpl_name) = template {
        if note_path.exists() && !force {
            anyhow::bail!("{}", config.msgs.note_already_exists());
        }
        let tmpl = crate::notes::load_template(tmpl_name, &config.templates_dir)?;
        let rendered = crate::notes::render_template(&tmpl, &entry);
        std::fs::write(&note_path, &rendered)?;
        println!("{}", config.msgs.note_template_applied(tmpl_name, &note_path.display().to_string()));
    }

    // Step 2: Content write (--stdin or --from)
    let content_source: Option<String> = if stdin {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        Some(buf)
    } else if let Some(ref file) = from {
        Some(std::fs::read_to_string(file).with_context(|| format!("Cannot read: {}", file.display()))?)
    } else {
        None
    };

    if let Some(new_content) = content_source {
        // Ensure the note file exists (create minimal header if no template was applied)
        if !note_path.exists() {
            let header = format!(
                "# {}\ncitekey: {}\n\n",
                entry.title.as_deref().unwrap_or("Untitled"),
                entry.bibtex_key
            );
            std::fs::write(&note_path, &header)?;
        }

        let existing = std::fs::read_to_string(&note_path)?;

        if let Some(ref sec) = section {
            let updated = crate::notes::write_section(&existing, sec, &new_content);
            std::fs::write(&note_path, &updated)?;
            println!("{}", config.msgs.note_written_section(sec, &note_path.display().to_string()));
        } else {
            // Append to end
            let mut result = existing;
            if !result.ends_with('\n') {
                result.push('\n');
            }
            result.push_str(&new_content);
            if !new_content.ends_with('\n') {
                result.push('\n');
            }
            std::fs::write(&note_path, &result)?;
            println!("{}", config.msgs.note_appended(&note_path.display().to_string()));
        }
        if let Some(e) = find_by_key_mut(&mut db, &entry.bibtex_key) {
            e.updated_at = Some(Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
            let _ = save_db(&db, &db_path);
        }
        return Ok(());
    }

    // Step 3: No --stdin/--from — open editor (original behavior)
    if !note_path.exists() {
        let header = format!(
            "# {}\ncitekey: {}\n",
            entry.title.as_deref().unwrap_or("Untitled"),
            entry.bibtex_key
        );
        std::fs::write(&note_path, &header)?;
    }

    let editor = std::env::var("EDITOR").unwrap_or_else(|_| {
        if which_exists("nano") { "nano".to_string() } else { "vi".to_string() }
    });

    std::process::Command::new(&editor)
        .arg(&note_path)
        .status()
        .with_context(|| format!("Failed to launch editor '{}'", editor))?;

    if let Some(e) = find_by_key_mut(&mut db, &entry.bibtex_key) {
        e.updated_at = Some(Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
        let _ = save_db(&db, &db_path);
    }
    println!("{}", config.msgs.note_saved(&note_path.display().to_string()));
    Ok(())
}

fn which_exists(name: &str) -> bool {
    std::process::Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// ── modify ───────────────────────────────────────────────────────────────────

pub fn cmd_modify(
    assignments: Vec<String>,
    filter: Option<String>,
    all: bool,
    yes: bool,
    config: &Config,
) -> Result<()> {
    // 1. Require --filter or --all
    if filter.is_none() && !all {
        anyhow::bail!("Specify --filter or --all");
    }

    // 2. Parse assignments
    struct Assignment {
        field: String,
        value: String,
    }
    let mut parsed_assignments: Vec<Assignment> = Vec::new();
    for a in &assignments {
        let (field, value) = a
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("Invalid assignment '{}': expected field=value", a))?;
        let field = field.trim().to_string();
        let value = value.trim().to_string();
        // Validate field name
        match field.as_str() {
            "year" | "journal" | "publisher" | "booktitle" | "volume" | "number" | "pages"
            | "note" | "howpublished" | "month" | "tags_add" | "tags_remove" => {}
            other => anyhow::bail!("Unknown field '{}'", other),
        }
        parsed_assignments.push(Assignment { field, value });
    }

    // 3. Load DB
    let db_path = db_path_from_config(config);
    let mut db = load_db(&db_path)?;

    // 4. Parse filter and collect matching entry indices
    let matching_indices: Vec<usize> = if let Some(ref filter_str) = filter {
        // Parse comma-separated filter conditions
        struct FilterCond {
            kind: String,
            value: String,
        }
        let conditions: Vec<FilterCond> = filter_str
            .split(',')
            .map(|part| {
                let part = part.trim();
                if let Some((k, v)) = part.split_once(':') {
                    FilterCond {
                        kind: k.trim().to_string(),
                        value: v.trim().to_string(),
                    }
                } else {
                    FilterCond {
                        kind: String::new(),
                        value: part.to_string(),
                    }
                }
            })
            .collect();

        db.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                conditions.iter().all(|cond| match cond.kind.as_str() {
                    "collection" => entry.collections.contains(&cond.value),
                    "tag" => entry.tags.contains(&cond.value),
                    "type" => entry.entry_type.to_string() == cond.value,
                    "year" => {
                        if let Ok(y) = cond.value.parse::<u32>() {
                            entry.year == Some(y)
                        } else {
                            false
                        }
                    }
                    _ => false,
                })
            })
            .map(|(i, _)| i)
            .collect()
    } else {
        // --all
        (0..db.entries.len()).collect()
    };

    if matching_indices.is_empty() {
        println!("No entries matched the filter.");
        return Ok(());
    }

    // 5. Show what will change
    println!("Matching entries ({}):", matching_indices.len());
    for &i in &matching_indices {
        println!("  - {}", db.entries[i].bibtex_key);
    }
    println!("Changes to apply:");
    for a in &parsed_assignments {
        println!("  {} = {}", a.field, a.value);
    }

    // 6. Confirm
    if !yes {
        if !prompt_confirm(&format!("Apply to {} entries?", matching_indices.len())) {
            println!("Aborted.");
            return Ok(());
        }
    }

    // 7. Apply assignments
    let mut modified = 0usize;
    for &i in &matching_indices {
        let entry = &mut db.entries[i];
        for a in &parsed_assignments {
            match a.field.as_str() {
                "year" => {
                    entry.year = Some(a.value.parse::<u32>().with_context(|| {
                        format!("Invalid year value: '{}'", a.value)
                    })?);
                }
                "journal" => entry.journal = Some(a.value.clone()),
                "publisher" => entry.publisher = Some(a.value.clone()),
                "booktitle" => entry.booktitle = Some(a.value.clone()),
                "volume" => entry.volume = Some(a.value.clone()),
                "number" => entry.number = Some(a.value.clone()),
                "pages" => entry.pages = Some(a.value.clone()),
                "note" => entry.note = Some(a.value.clone()),
                "howpublished" => entry.howpublished = Some(a.value.clone()),
                "month" => entry.month = Some(a.value.clone()),
                "tags_add" => {
                    for tag in a.value.split(',') {
                        let tag = tag.trim().to_string();
                        if !tag.is_empty() && !entry.tags.contains(&tag) {
                            entry.tags.push(tag);
                        }
                    }
                }
                "tags_remove" => {
                    let to_remove: Vec<String> = a
                        .value
                        .split(',')
                        .map(|t| t.trim().to_string())
                        .collect();
                    entry.tags.retain(|t| !to_remove.contains(t));
                }
                _ => {}
            }
        }
        entry.updated_at = Some(Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
        modified += 1;
    }

    // 8. Save and report
    save_db(&db, &db_path)?;
    println!("Modified {} entries.", modified);

    Ok(())
}

// ── review ───────────────────────────────────────────────────────────────────

pub fn cmd_review(
    collection: Option<String>,
    filter: Option<String>,
    unreviewed: bool,
    config: &Config,
) -> Result<()> {
    let db_path = db_path_from_config(config);
    let mut db = load_db(&db_path)?;

    // Collect matching indices
    let indices: Vec<usize> = {
        // Start with all
        let mut idxs: Vec<usize> = (0..db.entries.len()).collect();

        // Filter by collection
        if let Some(ref col) = collection {
            idxs.retain(|&i| db.entries[i].collections.contains(col));
        }

        // Filter by generic filter string (same logic as cmd_modify)
        if let Some(ref filter_str) = filter {
            struct FilterCond {
                kind: String,
                value: String,
            }
            let conditions: Vec<FilterCond> = filter_str
                .split(',')
                .map(|part| {
                    let part = part.trim();
                    if let Some((k, v)) = part.split_once(':') {
                        FilterCond {
                            kind: k.trim().to_string(),
                            value: v.trim().to_string(),
                        }
                    } else {
                        FilterCond {
                            kind: String::new(),
                            value: part.to_string(),
                        }
                    }
                })
                .collect();

            idxs.retain(|&i| {
                let entry = &db.entries[i];
                conditions.iter().all(|cond| match cond.kind.as_str() {
                    "collection" => entry.collections.contains(&cond.value),
                    "tag" => entry.tags.contains(&cond.value),
                    "type" => entry.entry_type.to_string() == cond.value,
                    "year" => {
                        if let Ok(y) = cond.value.parse::<u32>() {
                            entry.year == Some(y)
                        } else {
                            false
                        }
                    }
                    _ => false,
                })
            });
        }

        // Filter unreviewed
        if unreviewed {
            idxs.retain(|&i| !db.entries[i].tags.contains(&"reviewed".to_string()));
        }

        // Sort by year desc, then bibtex_key asc
        idxs.sort_by(|&a, &b| {
            let ea = &db.entries[a];
            let eb = &db.entries[b];
            let year_cmp = eb.year.unwrap_or(0).cmp(&ea.year.unwrap_or(0));
            if year_cmp != std::cmp::Ordering::Equal {
                year_cmp
            } else {
                ea.bibtex_key.cmp(&eb.bibtex_key)
            }
        });

        idxs
    };

    let total = indices.len();
    if total == 0 {
        println!("No entries to review.");
        return Ok(());
    }

    let mut reviewed_count = 0usize;
    let sep = "─────────────────────────────────────────────";

    // We iterate through entries with a cursor (supports prev)
    let mut cursor: usize = 0;

    loop {
        if cursor >= total {
            break;
        }

        let db_idx = indices[cursor];
        let entry = &db.entries[db_idx];

        // Display entry
        println!("\n{}", sep);
        println!("[{}/{}] {}", cursor + 1, total, entry.bibtex_key);
        println!("{}", sep);
        println!(
            "{:<10}: {}",
            "Title",
            entry.title.as_deref().unwrap_or("(none)")
        );
        let authors_str = if entry.author.is_empty() {
            "(none)".to_string()
        } else {
            entry.author.join("; ")
        };
        println!("{:<10}: {}", "Authors", authors_str);
        println!(
            "{:<10}: {}",
            "Year",
            entry
                .year
                .map(|y| y.to_string())
                .unwrap_or_else(|| "(none)".to_string())
        );
        println!("{:<10}: {}", "Type", entry.entry_type);

        // Journal / booktitle / publisher
        if let Some(j) = &entry.journal {
            println!("{:<10}: {}", "Journal", j);
        } else if let Some(bt) = &entry.booktitle {
            println!("{:<10}: {}", "Booktitle", bt);
        } else if let Some(p) = &entry.publisher {
            println!("{:<10}: {}", "Publisher", p);
        }

        let tags_str = if entry.tags.is_empty() {
            "(none)".to_string()
        } else {
            entry.tags.join(", ")
        };
        println!("{:<10}: {}", "Tags", tags_str);
        println!(
            "{:<10}: {}",
            "Note",
            entry.note.as_deref().unwrap_or("(none)")
        );

        // PDF info
        if let Some(fp) = &entry.file_path {
            let full_path = config.bibox_dir.join(fp);
            let status = if full_path.exists() { "[exists]" } else { "[missing]" };
            println!("{:<10}: {}  {}", "PDF", full_path.display(), status);
        } else {
            println!("{:<10}: (none)", "PDF");
        }
        println!("{}", sep);
        println!("Actions: [n]ext  [p]rev  [o]pen PDF  [t]ag  [e]dit note  [s]kip  [q]uit");
        print!("> ");
        io::stdout().flush()?;

        // Raw keypress input
        enable_raw_mode()?;
        let key = loop {
            match ct_event::read() {
                Ok(Event::Key(k)) => {
                    // Ignore modifier-only events
                    if k.modifiers == KeyModifiers::NONE || k.modifiers == KeyModifiers::SHIFT {
                        break k.code;
                    }
                }
                Ok(_) => continue,
                Err(e) => {
                    disable_raw_mode()?;
                    return Err(e.into());
                }
            }
        };
        disable_raw_mode()?;
        println!(); // newline after the keypress echo

        match key {
            // next / Enter → mark reviewed, move forward
            KeyCode::Char('n') | KeyCode::Enter => {
                let entry = &mut db.entries[db_idx];
                if !entry.tags.contains(&"reviewed".to_string()) {
                    entry.tags.push("reviewed".to_string());
                    entry.updated_at = Some(Local::now().format("%Y-%m-%d %H:%M:%S").to_string());
                    reviewed_count += 1;
                }
                save_db(&db, &db_path)?;
                println!("Reviewed: {}/{}", reviewed_count, total);
                cursor += 1;
            }

            // prev
            KeyCode::Char('p') => {
                if cursor > 0 {
                    cursor -= 1;
                } else {
                    println!("Already at the first entry.");
                }
            }

            // open PDF
            KeyCode::Char('o') => {
                let fp_opt = db.entries[db_idx].file_path.clone();
                if let Some(fp) = fp_opt {
                    let full_path = config.bibox_dir.join(&fp);
                    if full_path.exists() {
                        let path_str = full_path.to_string_lossy().to_string();
                        if let Some(viewer) = &config.pdf_viewer {
                            let _ = std::process::Command::new(viewer)
                                .arg(&path_str)
                                .spawn();
                        } else {
                            #[cfg(target_os = "macos")]
                            let _ = std::process::Command::new("open")
                                .arg(&path_str)
                                .spawn();
                            #[cfg(not(target_os = "macos"))]
                            let _ = std::process::Command::new("xdg-open")
                                .arg(&path_str)
                                .spawn();
                        }
                        println!("Opening PDF...");
                    } else {
                        println!("PDF file not found: {}", full_path.display());
                    }
                } else {
                    println!("No PDF associated with this entry.");
                }
                // Stay on same entry (don't advance cursor)
            }

            // tag
            KeyCode::Char('t') => {
                print!("Tags to add (comma-separated): ");
                io::stdout().flush()?;
                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                let new_tags: Vec<String> = input
                    .trim()
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                if !new_tags.is_empty() {
                    let entry = &mut db.entries[db_idx];
                    for tag in &new_tags {
                        if !entry.tags.contains(tag) {
                            entry.tags.push(tag.clone());
                        }
                    }
                    save_db(&db, &db_path)?;
                    println!("Tags added: {}", new_tags.join(", "));
                }
            }

            // edit note
            KeyCode::Char('e') => {
                let key_str = db.entries[db_idx].bibtex_key.clone();
                let title_str = db.entries[db_idx].title.clone();
                let notes_dir = &config.notes_dir;
                std::fs::create_dir_all(notes_dir)?;
                let note_path = notes_dir.join(format!("{}.md", key_str));
                if !note_path.exists() {
                    let header = format!(
                        "# {}\n\ncitekey: {}\n",
                        title_str.as_deref().unwrap_or("Untitled"),
                        key_str
                    );
                    std::fs::write(&note_path, &header)?;
                }
                let editor = std::env::var("EDITOR").unwrap_or_else(|_| {
                    if which_exists("nano") {
                        "nano".to_string()
                    } else {
                        "vi".to_string()
                    }
                });
                let _ = std::process::Command::new(&editor)
                    .arg(&note_path)
                    .status();
                println!("Note saved: {}", note_path.display());
            }

            // skip
            KeyCode::Char('s') => {
                println!("Skipped.");
                cursor += 1;
            }

            // quit
            KeyCode::Char('q') | KeyCode::Esc => {
                println!("Quitting review session.");
                break;
            }

            _ => {
                println!("Unknown action. Use n/p/o/t/e/s/q.");
            }
        }
    }

    println!("\nSession complete. Reviewed {} of {} entries.", reviewed_count, total);
    Ok(())
}

// ── arxiv fallback ────────────────────────────────────────────────────────────

async fn try_arxiv_fallback(title: &str, config: &Config) -> Option<PathBuf> {
    println!("{}", config.msgs.searching_arxiv());

    let results = match arxiv::search_by_title(title, 5).await {
        Ok(r) => r,
        Err(e) => {
            println!("{}", config.msgs.arxiv_failed(&e.to_string()));
            return None;
        }
    };

    if results.is_empty() {
        println!("{}", config.msgs.no_arxiv_results());
        return None;
    }

    println!("{}", config.msgs.arxiv_found(results.len()));

    let items: Vec<SelectItem> = results
        .iter()
        .map(|r| SelectItem {
            key: r.pdf_url.clone(),
            display: format!("[{}] {}", r.arxiv_id, r.title),
        })
        .collect();

    let chosen_url = match interactive_select(&items) {
        Ok(Some(url)) => url,
        _ => return None,
    };

    if !prompt_confirm(config.msgs.download_prompt()) {
        return None;
    }

    let tmp = std::env::temp_dir().join("bibox_download.pdf");
    print!("{}", config.msgs.downloading());
    let _ = std::io::stdout().flush();

    match unpaywall::download_pdf(&chosen_url, &tmp).await {
        Ok(()) => {
            println!("{}", config.msgs.done());
            Some(tmp)
        }
        Err(e) => {
            println!(" failed: {}", e);
            None
        }
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

struct RawBibEntry {
    key: Option<String>,
    entry_type: String,
    title: Option<String>,
    author: Option<String>,
    year: Option<u32>,
    journal: Option<String>,
    volume: Option<String>,
    number: Option<String>,
    pages: Option<String>,
    publisher: Option<String>,
    editor: Option<String>,
    edition: Option<String>,
    isbn: Option<String>,
    booktitle: Option<String>,
    doi: Option<String>,
    url: Option<String>,
    note: Option<String>,
    howpublished: Option<String>,
    month: Option<String>,
    abstract_text: Option<String>,
    keywords: Option<String>,
}

fn parse_bibtex(content: &str) -> Vec<RawBibEntry> {
    let mut entries = vec![];
    let re_entry = regex::Regex::new(r"@(\w+)\s*\{\s*([^,\s]*)\s*,").unwrap();

    let mut pos = 0;
    while let Some(cap) = re_entry.find_at(content, pos) {
        let entry_type = re_entry
            .captures(cap.as_str())
            .unwrap()
            .get(1)
            .unwrap()
            .as_str()
            .to_lowercase();

        if entry_type == "comment" || entry_type == "string" || entry_type == "preamble" {
            pos = cap.end();
            continue;
        }

        let key = re_entry
            .captures(cap.as_str())
            .unwrap()
            .get(2)
            .map(|m| m.as_str().to_string())
            .filter(|s| !s.is_empty());

        let _entry_start = cap.start();
        let body_start = cap.end();
        let body = find_entry_body(content, body_start);

        let mut raw = RawBibEntry {
            key,
            entry_type,
            title: None,
            author: None,
            year: None,
            journal: None,
            volume: None,
            number: None,
            pages: None,
            publisher: None,
            editor: None,
            edition: None,
            isbn: None,
            booktitle: None,
            doi: None,
            url: None,
            note: None,
            howpublished: None,
            month: None,
            abstract_text: None,
            keywords: None,
        };

        let fields = parse_bib_fields(body);
        for (field, value) in fields {
            match field.as_str() {
                "title"     => raw.title     = Some(decode_latex(&value)),
                "author"    => raw.author    = Some(decode_latex(&value)),
                "year"      => raw.year      = value.parse().ok(),
                "journal"   => raw.journal   = Some(decode_latex(&value)),
                "volume"    => raw.volume    = Some(value),
                "number"    => raw.number    = Some(value),
                "pages"     => raw.pages     = Some(value),
                "publisher" => raw.publisher = Some(decode_latex(&value)),
                "editor"    => raw.editor    = Some(decode_latex(&value)),
                "edition"   => raw.edition   = Some(value),
                "isbn"      => raw.isbn      = Some(value),
                "booktitle" => raw.booktitle = Some(decode_latex(&value)),
                "doi"       => raw.doi       = Some(value),
                "url"       => raw.url       = Some(value),
                "note"      => raw.note      = Some(value),
                "howpublished" => raw.howpublished = Some(value),
                "month"        => raw.month = Some(value),
                "abstract"  => raw.abstract_text = Some(decode_latex(&value)),
                "keywords"  => raw.keywords  = Some(value),
                _ => {}
            }
        }

        entries.push(raw);
        pos = body_start + body.len();
    }

    entries
}

/// Strip LaTeX-style curly braces used for case preservation: {Word} → Word.
/// Handles nested braces. Does not strip quotes or other LaTeX commands.
fn strip_bibtex_braces(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c != '{' && c != '}' {
            out.push(c);
        }
    }
    // Collapse multiple spaces that may result from removing braces
    let mut result = String::with_capacity(out.len());
    let mut prev_space = false;
    for c in out.chars() {
        if c == ' ' {
            if !prev_space { result.push(c); }
            prev_space = true;
        } else {
            result.push(c);
            prev_space = false;
        }
    }
    result.trim().to_string()
}

/// Decode common LaTeX escape sequences to Unicode.
/// Handles accented characters (\'{e} → é), special letters (\l → ł), etc.
fn decode_latex(s: &str) -> String {
    // First pass: named commands like {\l}, {\o}, {\aa}, {\ss}, etc.
    let named: &[(&str, &str)] = &[
        // Polish / Scandinavian / German special letters
        ("{\\l}", "ł"), ("{\\L}", "Ł"),
        ("\\l ", "ł"), ("\\l{}", "ł"),
        ("{\\o}", "ø"), ("{\\O}", "Ø"),
        ("\\o ", "ø"), ("\\o{}", "ø"),
        ("{\\aa}", "å"), ("{\\AA}", "Å"),
        ("{\\ae}", "æ"), ("{\\AE}", "Æ"),
        ("{\\oe}", "œ"), ("{\\OE}", "Œ"),
        ("{\\ss}", "ß"),
        ("\\ss ", "ß"), ("\\ss{}", "ß"),
        // Common standalone
        ("\\l", "ł"), ("\\L", "Ł"),
        ("\\o", "ø"), ("\\O", "Ø"),
        ("\\i", "ı"), ("\\j", "ȷ"),
        // LaTeX special characters
        ("\\&", "&"), ("\\%", "%"), ("\\$", "$"), ("\\#", "#"),
        ("\\{", "{"), ("\\}", "}"), ("\\_", "_"), ("\\~{}", "~"),
    ];

    let mut out = s.to_string();
    for (pat, repl) in named {
        out = out.replace(pat, repl);
    }

    // Second pass: accent commands - \'X, \"X, \^X, \`X, \~X, \=X, \.X, \v{X}, \c{X}, \u{X}, \H{X}
    // Handle both {\'{e}} and \'{e} and \'e forms
    let accent_map: &[(&str, &[(char, char)])] = &[
        ("'", &[('a','á'),('e','é'),('i','í'),('o','ó'),('u','ú'),('y','ý'),
                ('A','Á'),('E','É'),('I','Í'),('O','Ó'),('U','Ú'),('Y','Ý'),
                ('c','ć'),('n','ń'),('s','ś'),('z','ź'),('C','Ć'),('N','Ń'),('S','Ś'),('Z','Ź')]),
        ("`", &[('a','à'),('e','è'),('i','ì'),('o','ò'),('u','ù'),
                ('A','À'),('E','È'),('I','Ì'),('O','Ò'),('U','Ù')]),
        ("\"",&[('a','ä'),('e','ë'),('i','ï'),('o','ö'),('u','ü'),('y','ÿ'),
                ('A','Ä'),('E','Ë'),('I','Ï'),('O','Ö'),('U','Ü'),('Y','Ÿ')]),
        ("^", &[('a','â'),('e','ê'),('i','î'),('o','ô'),('u','û'),
                ('A','Â'),('E','Ê'),('I','Î'),('O','Ô'),('U','Û')]),
        ("~", &[('a','ã'),('n','ñ'),('o','õ'),('A','Ã'),('N','Ñ'),('O','Õ')]),
        ("v", &[('c','č'),('s','š'),('z','ž'),('r','ř'),('n','ň'),('e','ě'),('d','ď'),('t','ť'),
                ('C','Č'),('S','Š'),('Z','Ž'),('R','Ř'),('N','Ň'),('E','Ě'),('D','Ď'),('T','Ť')]),
        ("c", &[('c','ç'),('s','ş'),('C','Ç'),('S','Ş')]),
        ("u", &[('a','ă'),('g','ğ'),('A','Ă'),('G','Ğ')]),
        (".", &[('z','ż'),('Z','Ż')]),
        ("H", &[('o','ő'),('u','ű'),('O','Ő'),('U','Ű')]),
        ("=", &[('a','ā'),('e','ē'),('i','ī'),('o','ō'),('u','ū'),
                ('A','Ā'),('E','Ē'),('I','Ī'),('O','Ō'),('U','Ū')]),
    ];

    for (cmd, mappings) in accent_map {
        for &(from_ch, to_ch) in *mappings {
            // {\'{e}} or {\'e}
            let pat1 = format!("{{\\{}{{{}}}}}", cmd, from_ch);
            let pat2 = format!("{{\\{}{}}}", cmd, from_ch);
            // \'{e} or \'e
            let pat3 = format!("\\{}{{{}}}", cmd, from_ch);
            let pat4 = format!("\\{}{}", cmd, from_ch);
            let repl = to_ch.to_string();
            out = out.replace(&pat1, &repl);
            out = out.replace(&pat2, &repl);
            out = out.replace(&pat3, &repl);
            out = out.replace(&pat4, &repl);
        }
    }

    // Strip any remaining braces
    strip_bibtex_braces(&out)
}

/// Returns true if text contains LaTeX escape sequences.
/// Detects: \l, \'e, \"o (backslash + letter) and \& (backslash + special char).
fn has_latex_escapes(s: &str) -> bool {
    let bytes = s.as_bytes();
    for i in 0..bytes.len().saturating_sub(1) {
        if bytes[i] == b'\\' {
            let next = bytes[i + 1];
            if next.is_ascii_alphabetic() || next == b'&' || next == b'\''
                || next == b'"' || next == b'^' || next == b'`' || next == b'~'
            {
                return true;
            }
        }
    }
    false
}

/// Extract BibTeX field values from an entry body using depth-tracked brace parsing.
/// Handles nested braces correctly (e.g. `title = {{Acronym}: Full Title}`).
fn parse_bib_fields(body: &str) -> Vec<(String, String)> {
    let re_field_start = regex::Regex::new(r"\b(\w+)\s*=\s*\{").unwrap();
    let bytes = body.as_bytes();
    let mut fields = vec![];

    for cap in re_field_start.captures_iter(body) {
        let field_name = cap.get(1).unwrap().as_str().to_lowercase();
        let value_start = cap.get(0).unwrap().end(); // byte offset right after the opening `{`

        let mut depth = 1i32;
        let mut i = value_start;
        while i < bytes.len() {
            match bytes[i] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        let value = body[value_start..i].trim().to_string();
                        fields.push((field_name, value));
                        break;
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }

    fields
}

fn find_entry_body(content: &str, start: usize) -> &str {
    let bytes = content.as_bytes();
    let mut depth = 1i32;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &content[start..i];
                }
            }
            _ => {}
        }
        i += 1;
    }
    &content[start..]
}

fn create_zip(src_dir: &Path, zip_path: &Path) -> Result<()> {
    use std::fs::File;
    use std::io::Read;

    let file = File::create(zip_path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    for entry in std::fs::read_dir(src_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            let name = path.file_name().unwrap().to_string_lossy();
            zip.start_file(name.as_ref(), options)?;
            let mut f = File::open(&path)?;
            let mut buf = vec![];
            f.read_to_end(&mut buf)?;
            use std::io::Write;
            zip.write_all(&buf)?;
        }
    }

    zip.finish()?;
    Ok(())
}

// ── Template commands ───────────────────────────────────────────────────────

pub fn cmd_template_list(json: bool, config: &Config) -> Result<()> {
    let templates = crate::notes::list_templates(&config.templates_dir);
    if json {
        let items: Vec<serde_json::Value> = templates.iter().map(|(name, is_custom, is_override)| {
            serde_json::json!({
                "name": name,
                "custom": is_custom,
                "overridden": is_override,
            })
        }).collect();
        let result = serde_json::json!({
            "templates": items,
            "templates_dir": config.templates_dir.to_string_lossy(),
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        if templates.is_empty() {
            println!("No templates available.");
            return Ok(());
        }
        for (name, is_custom, is_override) in &templates {
            let tag = if *is_override {
                " (built-in, overridden by custom)"
            } else if *is_custom {
                " (custom)"
            } else {
                " (built-in)"
            };
            println!("  {}{}", name, tag);
        }
        println!("\nTemplates dir: {}", config.templates_dir.display());
    }
    Ok(())
}

pub fn cmd_template_show(name: &str, json: bool, config: &Config) -> Result<()> {
    let content = crate::notes::load_template(name, &config.templates_dir)?;
    if json {
        let result = serde_json::json!({
            "name": name,
            "content": content,
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        print!("{}", content);
    }
    Ok(())
}

pub fn cmd_template_create(name: &str, stdin: bool, config: &Config) -> Result<()> {
    std::fs::create_dir_all(&config.templates_dir)?;
    let path = config.templates_dir.join(format!("{}.md", name));
    if path.exists() {
        anyhow::bail!("Template '{}' already exists at {}. Use `bibox template edit {}` instead.", name, path.display(), name);
    }

    let content = if stdin {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        buf
    } else {
        // Open editor with empty file
        std::fs::write(&path, "")?;
        let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
        std::process::Command::new(&editor)
            .arg(&path)
            .status()
            .with_context(|| format!("Failed to launch editor '{}'", editor))?;
        std::fs::read_to_string(&path)?
    };

    if content.trim().is_empty() {
        let _ = std::fs::remove_file(&path);
        anyhow::bail!("Empty template — not saved.");
    }

    if !stdin {
        // Already written by editor
    } else {
        std::fs::write(&path, &content)?;
    }
    println!("Template '{}' created at {}", name, path.display());
    Ok(())
}

pub fn cmd_template_edit(name: &str, config: &Config) -> Result<()> {
    std::fs::create_dir_all(&config.templates_dir)?;
    let path = config.templates_dir.join(format!("{}.md", name));

    // If it's a built-in and no custom override exists, export it first
    if !path.exists() {
        if let Some(content) = crate::notes::builtin_template(name) {
            std::fs::write(&path, content)?;
            println!("Exported built-in '{}' to {} for editing.", name, path.display());
        } else {
            anyhow::bail!("Template '{}' not found.", name);
        }
    }

    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    std::process::Command::new(&editor)
        .arg(&path)
        .status()
        .with_context(|| format!("Failed to launch editor '{}'", editor))?;
    println!("Template '{}' saved.", name);
    Ok(())
}

pub fn cmd_template_delete(name: &str, config: &Config) -> Result<()> {
    if crate::notes::BUILTIN_NAMES.contains(&name) {
        let path = config.templates_dir.join(format!("{}.md", name));
        if path.exists() {
            std::fs::remove_file(&path)?;
            println!("Custom override for '{}' deleted. Built-in version restored.", name);
        } else {
            anyhow::bail!("Cannot delete built-in template '{}'. It has no custom override.", name);
        }
    } else {
        let path = config.templates_dir.join(format!("{}.md", name));
        if !path.exists() {
            anyhow::bail!("Template '{}' not found.", name);
        }
        std::fs::remove_file(&path)?;
        println!("Template '{}' deleted.", name);
    }
    Ok(())
}

pub fn cmd_template_export(name: &str, config: &Config) -> Result<()> {
    let content = match crate::notes::builtin_template(name) {
        Some(c) => c,
        None => anyhow::bail!("'{}' is not a built-in template. Available: {}", name, crate::notes::BUILTIN_NAMES.join(", ")),
    };
    std::fs::create_dir_all(&config.templates_dir)?;
    let path = config.templates_dir.join(format!("{}.md", name));
    if path.exists() {
        anyhow::bail!("Custom '{}' already exists at {}. Edit it directly with `bibox template edit {}`.", name, path.display(), name);
    }
    std::fs::write(&path, content)?;
    println!("Exported '{}' to {}", name, path.display());
    Ok(())
}

// ── agent-guide ─────────────────────────────────────────────────────────────

const AGENT_GUIDE: &str = r##"# bibox — AI Agent Guide

bibox is a terminal-based bibliography manager. All commands support `--json` for machine-readable output.

## Setup

```bash
# Install on Linux server (no Rust needed)
curl -L https://github.com/namil-k/bibox/releases/latest/download/bibox-x86_64-unknown-linux-musl -o ~/.local/bin/bibox
chmod +x ~/.local/bin/bibox

# Initialize a portable home (db, pdfs, notes in one folder)
bibox init <path> --json   # e.g. ~/bibox, ~/papers, etc.

# Show all resolved paths (home, db, pdfs, notes, config)
bibox config --json

# Connect to GitHub for sync
cd <bibox-home> && git init && git remote add origin <url> && git push -u origin master
```

## Adding Papers

```bash
# By PDF (auto-extracts DOI, fetches metadata)
bibox add paper.pdf --json

# By DOI
bibox add --doi 10.1145/3290605.3300907 --json

# By arXiv ID
bibox add --arxiv 2301.12345 --json

# By ISBN
bibox add --isbn 978-0-13-468599-1 --json

# By URL (academic paper pages — extracts DOI automatically)
bibox add --url https://arxiv.org/abs/2301.12345 --json

# Search by title (non-interactive: --index selects 0-based result)
bibox add --search "attention is all you need" --index 0 --json

# Add to a collection on import
bibox add --doi 10.xxx --to ml --json

# Web page / product / non-paper reference (misc entry)
bibox add --title "Varjo Aero" --url https://varjo.com/products/aero/ --author "Varjo" --year 2024 --json

# Online video (IEEE-style misc with howpublished + month)
bibox add --title "Bangkok VR Tour" --author "VR Gorilla" --year 2024 --month jan \
  --howpublished '[Online Video]. Available: \url{https://youtube.com/watch?v=xxx}' --json

# Minimal misc entry (title only)
bibox add --title "Some Web Resource" --json
```

## Querying

```bash
# List all collections with counts
bibox list --json

# List entries in a collection
bibox list ml --json

# Search by keyword
bibox search "transformer" --json
bibox search "kim" --field author --json

# Show single entry metadata
bibox show kim2025rust --json
```

## Notes (AI-agent-friendly Markdown notes)

Notes are stored as `<citekey>.md` files. Section-level updates allow incremental writing.

```bash
# Initialize note from template
bibox note <key> --template ai-summary

# Write to a specific section (replaces if exists, appends if new)
echo "content" | bibox note <key> --stdin --section "Summary"
echo "content" | bibox note <key> --stdin --section "Key Contributions"
echo "content" | bibox note <key> --stdin --section "Methodology"

# Append without targeting a section
echo "extra notes" | bibox note <key> --stdin

# Read note content
bibox note <key> --show --json

# Get note file path
bibox note <key> --path --json
```

### Available template variables
`{{title}}`, `{{citekey}}`, `{{doi}}`, `{{year}}`, `{{author}}`, `{{journal}}`, `{{booktitle}}`, `{{publisher}}`

## Templates

```bash
# List available templates
bibox template list --json

# Show template content
bibox template show ai-summary --json

# Create custom template
echo "# {{title}}
citekey: {{citekey}}
## Review Score
## Strengths
## Weaknesses
" | bibox template create my-review --stdin

# Delete custom template
bibox template delete my-review
```

## Editing Metadata

```bash
# Edit specific fields
bibox edit <key> --title "New Title" --year 2025 --journal "Nature"

# Set howpublished and month (for @misc / IEEE-style entries)
bibox edit <key> --howpublished '[Online Video]. Available: \url{https://...}' --month jan

# Re-fetch metadata from Crossref by DOI (preserves existing values)
bibox edit <key> --doi 10.1234/new

# Add/remove tags
bibox edit <key> --tags-add "transformer,nlp"
bibox edit <key> --tags-remove "draft"

# Bulk-update multiple entries
bibox modify year=2025 --filter "collection:ml" --yes
bibox modify journal="Nature" --filter "tag:review" --yes
bibox modify howpublished="[Online]. Available: \url{...}" --filter "tag:web" --yes
```

## Attaching PDFs

```bash
# Attach a local PDF to an existing entry (copies to bibox pdf dir, renames to citekey.pdf)
bibox edit <key> --attach-pdf /path/to/paper.pdf
```

Useful when automatic fetch fails (403, paywalled). Download the PDF manually in a browser, then attach it.

## Deleting Entries

```bash
# Delete with confirmation
bibox delete <key>

# Non-interactive delete (for agents)
bibox delete <key> -y
```

## Collections & Tags

```bash
# Add to collection
bibox collect <key> ml systems

# Add to sub-collection (path notation, unlimited depth)
bibox collect <key> digest/2026-04

# Remove from collection
bibox uncollect <key> ml

# List a collection and all sub-collections (prefix match)
bibox list digest --json
```

## Importing

```bash
# Import from BibTeX file
bibox import refs.bib

# Import into a collection
bibox import refs.bib --to ml
```

## Export

Without `-o`, BibTeX/YAML/RIS/CSV content goes to stdout (status messages to stderr).

```bash
# Export as BibTeX to stdout (pipe or redirect)
bibox export --collection ml > refs.bib

# Export specific entries to file
bibox export <key1> <key2> -o refs.bib

# Export collection as RIS
bibox export --collection cs --format ris -o cs.ris

# Export with PDFs
bibox export --collection ml --include-pdf --zip

# Export all note .md files to a folder
bibox export --notes-only -o ~/notes

# Export notes for a specific collection
bibox export --collection ml --notes-only -o ~/ml-notes
```

## Sync

```bash
# Non-interactive sync (for agents)
bibox sync --yes --json
```

## Typical AI Agent Workflow

```bash
# 1. Search and add a paper
bibox add --search "attention is all you need" --index 0 --to ml --json

# 2. Get the citekey from JSON output, then create note
bibox note vaswani2017attention --template ai-summary

# 3. Read the paper and fill sections
echo "The paper proposes..." | bibox note vaswani2017attention --stdin --section "Summary"
echo "1. Multi-head attention..." | bibox note vaswani2017attention --stdin --section "Key Contributions"
echo "Encoder-decoder with..." | bibox note vaswani2017attention --stdin --section "Methodology"
echo "BLEU 28.4 on EN-DE..." | bibox note vaswani2017attention --stdin --section "Results"
echo "Quadratic complexity..." | bibox note vaswani2017attention --stdin --section "Limitations"

# 4. Verify
bibox note vaswani2017attention --show

# 5. Push to git
# Push to git (use the home path from `bibox init`)
cd <bibox-home> && git add . && git commit -m "add vaswani2017attention" && git push
```

## Key Flags for Agents

| Flag | Purpose |
|------|---------|
| `--json` | Machine-readable JSON output (available on most commands) |
| `--index N` | Auto-select Nth search result (0-based, with `--search`) |
| `--stdin` | Read content from stdin (notes, templates) |
| `--section "Name"` | Target a specific `## Heading` in a note |
| `--yes` / `-y` | Skip confirmation prompts |
| `--template <name>` | Initialize note from template |
| `--howpublished` | Set howpublished field (for `add`, `edit`, `modify`; used in IEEE @misc) |
| `--month` | Set month field (for `add`, `edit`, `modify`; BibTeX macros like `jan` exported without braces) |

## Diagnostics

```bash
# Check for issues
bibox doctor --json

# Auto-repair fixable issues
bibox doctor --fix
```

Issue types:
- `malformed_entry` - DB entry that cannot be parsed (manual fix required)
- `bad_citekey` - Citekey contains non-ASCII or special chars [fixable: sanitizes to ASCII]
- `duplicate_key` - Two entries share the same citekey
- `missing_pdf` - file_path is set but the PDF file is gone [fixable: clears reference]
- `orphaned_pdf` - PDF on disk not linked to any entry [fixable: deletes file]
- `missing_title` - Entry has no title
- `dirty_title` - BibTeX braces {} leaked into text fields [fixable: strips braces]
- `latex_escape` - LaTeX escapes (\l, \&, \' etc.) in text/author [fixable: decodes to Unicode]
- `orphaned_note` - Note file with no matching entry
- `invalid_json` - db.json is not valid JSON (e.g. git merge conflict markers)

## Tips

- Run `bibox config --json` to get all paths (home, db, pdfs, notes, config.toml location).
- The home path is the git-syncable directory. Get it from `bibox config --json` → `home` field.
- All `--json` output goes to stdout. Errors go to stderr.
- When a command fails, the exit code is non-zero.
- If db.json is corrupted or has git merge conflicts, run `bibox doctor` for a diagnosis.
"##;

pub fn cmd_config(json: bool, config: &Config) -> Result<()> {
    let config_path = crate::config::config_path();
    let db_path = crate::config::resolve_db_path(config);

    if json {
        let result = serde_json::json!({
            "config_path": config_path.to_string_lossy(),
            "home": config.home.as_ref().map(|h| h.to_string_lossy().to_string()),
            "db_path": db_path.to_string_lossy(),
            "bibox_dir": config.bibox_dir.to_string_lossy(),
            "notes_dir": config.notes_dir.to_string_lossy(),
            "templates_dir": config.templates_dir.to_string_lossy(),
            "language": config.language,
            "git": config.git,
            "line_numbers": format!("{:?}", config.line_numbers).to_lowercase(),
            "panel_ratio": config.panel_ratio,
            "bib_export_dir": config.bib_export_dir.to_string_lossy(),
            "export_dir": config.export_dir.to_string_lossy(),
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("Config:       {}", config_path.display());
        println!("Home:         {}", config.home.as_ref().map(|h| h.to_string_lossy().to_string()).unwrap_or_else(|| "(not set)".into()));
        println!("Database:     {}", db_path.display());
        println!("PDFs:         {}", config.bibox_dir.display());
        println!("Notes:        {}", config.notes_dir.display());
        println!("Templates:    {}", config.templates_dir.display());
        println!("Language:     {}", config.language);
        println!("Git:          {}", config.git);
        println!("Line numbers: {:?}", config.line_numbers);
        println!("Panel ratio:  {:?}", config.panel_ratio);
        println!("Bib export:   {}", config.bib_export_dir.display());
        println!("Export:       {}", config.export_dir.display());
    }
    Ok(())
}

pub fn cmd_update(check_only: bool) -> Result<()> {
    const CURRENT: &str = env!("CARGO_PKG_VERSION");

    // Fetch latest version from crates.io
    let client = reqwest::blocking::Client::builder()
        .user_agent(format!("bibox/{} (version-check)", CURRENT))
        .build()?;

    let resp: serde_json::Value = client
        .get("https://crates.io/api/v1/crates/bibox")
        .send()?
        .json()?;

    let latest = resp["crate"]["newest_version"]
        .as_str()
        .unwrap_or("unknown");

    if latest == CURRENT {
        println!("bibox {} is already up to date.", CURRENT);
        return Ok(());
    }

    println!("Current: {}  →  Latest: {}", CURRENT, latest);

    if check_only {
        println!("Run `bibox update` to install the latest version.");
        return Ok(());
    }

    println!("Installing bibox {}...", latest);

    let target = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", _)         => Some("x86_64-unknown-linux-musl"),
        ("macos", "aarch64") => Some("aarch64-apple-darwin"),
        ("macos", "x86_64")  => Some("x86_64-apple-darwin"),
        _                    => None,
    };

    let success = if let Some(target) = target {
        let bin_url = format!(
            "https://github.com/namil-k/bibox/releases/download/v{}/bibox-{}",
            latest, target
        );
        let current_exe = std::env::current_exe()
            .unwrap_or_else(|_| std::path::PathBuf::from("/usr/local/bin/bibox"));
        let tmp = std::env::temp_dir().join("bibox_update");
        println!("Downloading from GitHub releases...");
        let status = std::process::Command::new("curl")
            .args(["-fsSL", &bin_url, "-o", &tmp.to_string_lossy()])
            .status()?;
        if status.success() {
            let _ = std::process::Command::new("chmod")
                .args(["+x", &tmp.to_string_lossy()])
                .status();
            // Replace current binary (mv works even on running executables on Linux)
            let mv = std::process::Command::new("mv")
                .args([&tmp.to_string_lossy().to_string(), &current_exe.to_string_lossy().to_string()])
                .status();
            if mv.map(|s| s.success()).unwrap_or(false) {
                true
            } else {
                // mv failed (permission?) - try with sudo
                println!("Permission denied. Trying with sudo...");
                std::process::Command::new("sudo")
                    .args(["mv", &tmp.to_string_lossy(), &current_exe.to_string_lossy()])
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
            }
        } else {
            false
        }
    } else {
        // Unknown platform: fall back to cargo install
        std::process::Command::new("cargo")
            .args(["install", "bibox", "--force"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };

    if success {
        println!("bibox {} installed successfully.", latest);
    } else {
        anyhow::bail!("update failed");
    }

    Ok(())
}

pub fn cmd_doctor(fix: bool, json: bool, config: &Config) -> Result<()> {
    use std::collections::HashSet;

    let db_path = crate::config::resolve_db_path(config);
    let bibox_dir = &config.bibox_dir;
    let notes_dir = &config.notes_dir;

    // ── Issue types ──────────────────────────────────────────────────────────

    #[derive(serde::Serialize)]
    struct Issue {
        kind: String,
        key: Option<String>,
        detail: String,
        fixable: bool,
    }

    let mut issues: Vec<Issue> = Vec::new();

    // ── 1. Raw DB parse - find malformed entries ─────────────────────────────
    let raw_entries: Vec<serde_json::Value> = if db_path.exists() {
        let content = std::fs::read_to_string(&db_path)?;
        match serde_json::from_str::<serde_json::Value>(&content) {
            Err(e) => {
                // The whole file is invalid JSON - report and stop entry-level checks
                issues.push(Issue {
                    kind: "invalid_json".into(),
                    key: None,
                    detail: format!("db.json is not valid JSON at line {} col {}: {}",
                        e.line(), e.column(), e),
                    fixable: false,
                });
                if json {
                    let result = serde_json::json!({
                        "total_entries": 0,
                        "issues": issues,
                        "issue_count": issues.len(),
                    });
                    println!("{}", serde_json::to_string_pretty(&result)?);
                } else {
                    println!("bibox doctor  (db: {})", db_path.display());
                    println!();
                    println!("CRITICAL: db.json is not valid JSON.");
                    println!("  at line {} col {}: {}", e.line(), e.column(), e);
                    println!();
                    println!("Manual repair required. Open the file in a text editor:");
                    println!("  {}", db_path.display());
                }
                return Ok(());
            }
            Ok(raw) => raw.get("entries")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default(),
        }
    } else {
        vec![]
    };

    let mut good_entries: Vec<crate::models::Entry> = Vec::new();
    let mut malformed_indices: Vec<usize> = Vec::new();
    for (i, val) in raw_entries.iter().enumerate() {
        match serde_json::from_value::<crate::models::Entry>(val.clone()) {
            Ok(e) => good_entries.push(e),
            Err(e) => {
                malformed_indices.push(i);
                let key_hint = val.get("bibtex_key").and_then(|v| v.as_str()).unwrap_or("?");
                issues.push(Issue {
                    kind: "malformed_entry".into(),
                    key: Some(key_hint.to_string()),
                    detail: format!("Entry at index {} cannot be parsed: {}", i, e),
                    fixable: false,
                });
            }
        }
    }

    // ── 2. Invalid citekey characters (non-ASCII, special chars) ───────────
    let mut bad_key_keys: Vec<String> = Vec::new();
    for e in &good_entries {
        let has_bad = e.bibtex_key.chars().any(|c| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'));
        if has_bad {
            bad_key_keys.push(e.bibtex_key.clone());
            issues.push(Issue {
                kind: "bad_citekey".into(),
                key: Some(e.bibtex_key.clone()),
                detail: format!("Citekey contains non-ASCII or special chars"),
                fixable: true,
            });
        }
    }

    // ── 3. Duplicate citekeys ────────────────────────────────────────────────
    let mut seen_keys: HashSet<String> = HashSet::new();
    let mut dup_keys: HashSet<String> = HashSet::new();
    for e in &good_entries {
        if !seen_keys.insert(e.bibtex_key.clone()) {
            dup_keys.insert(e.bibtex_key.clone());
        }
    }
    for key in &dup_keys {
        issues.push(Issue {
            kind: "duplicate_key".into(),
            key: Some(key.clone()),
            detail: format!("Duplicate citekey '{}' - use `bibox edit` to rename one", key),
            fixable: false,
        });
    }

    // ── 4. Missing file (file_path set but not on disk) ──────────────────────
    let mut missing_pdf_keys: Vec<String> = Vec::new();
    for e in &good_entries {
        if let Some(fp) = &e.file_path {
            let full = bibox_dir.join(fp);
            if !full.exists() {
                missing_pdf_keys.push(e.bibtex_key.clone());
                issues.push(Issue {
                    kind: "missing_pdf".into(),
                    key: Some(e.bibtex_key.clone()),
                    detail: format!("file_path '{}' is registered but file does not exist", fp),
                    fixable: true,
                });
            }
        }
    }

    // ── 4. Orphaned PDFs (files in bibox_dir not referenced by any entry) ────
    let registered_files: HashSet<String> = good_entries.iter()
        .filter_map(|e| e.file_path.clone())
        .collect();

    let mut orphaned_pdfs: Vec<std::path::PathBuf> = Vec::new();
    if bibox_dir.exists() {
        for entry in std::fs::read_dir(bibox_dir)?.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("pdf") {
                let fname = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                if !registered_files.contains(&fname) {
                    orphaned_pdfs.push(path.clone());
                    issues.push(Issue {
                        kind: "orphaned_pdf".into(),
                        key: None,
                        detail: format!("PDF '{}' is not linked to any entry", fname),
                        fixable: true,
                    });
                }
            }
        }
    }

    // ── 5. Missing title ─────────────────────────────────────────────────────
    for e in &good_entries {
        if e.title.as_deref().unwrap_or("").trim().is_empty() {
            issues.push(Issue {
                kind: "missing_title".into(),
                key: Some(e.bibtex_key.clone()),
                detail: "Entry has no title".into(),
                fixable: false,
            });
        }
    }

    // ── 6. Dirty text fields (BibTeX brace leakage: { or } in title/journal/etc.) ──
    let mut dirty_title_keys: Vec<String> = Vec::new();
    for e in &good_entries {
        let fields = [
            ("title",     e.title.as_deref()),
            ("journal",   e.journal.as_deref()),
            ("booktitle", e.booktitle.as_deref()),
            ("publisher", e.publisher.as_deref()),
            ("editor",    e.editor.as_deref()),
        ];
        let dirty_fields: Vec<&str> = fields.iter()
            .filter(|(_, v)| v.map(|s| s.contains('{') || s.contains('}')).unwrap_or(false))
            .map(|(f, _)| *f)
            .collect();
        if !dirty_fields.is_empty() {
            dirty_title_keys.push(e.bibtex_key.clone());
            issues.push(Issue {
                kind: "dirty_title".into(),
                key: Some(e.bibtex_key.clone()),
                detail: format!(
                    "BibTeX braces {{}} found in field(s): {} — run `--fix` to strip them",
                    dirty_fields.join(", ")
                ),
                fixable: true,
            });
        }
    }

    // ── 7. LaTeX escapes in text fields (\l, \'{e}, etc.) ─────────────────────
    let mut latex_keys: Vec<String> = Vec::new();
    for e in &good_entries {
        let mut affected: Vec<&str> = Vec::new();
        // Check all text fields + author names
        let fields = [
            ("title",     e.title.as_deref()),
            ("journal",   e.journal.as_deref()),
            ("booktitle", e.booktitle.as_deref()),
            ("publisher", e.publisher.as_deref()),
            ("editor",    e.editor.as_deref()),
        ];
        for (name, val) in &fields {
            if val.map(|s| has_latex_escapes(s)).unwrap_or(false) {
                affected.push(name);
            }
        }
        if e.author.iter().any(|a| has_latex_escapes(a)) {
            affected.push("author");
        }
        if !affected.is_empty() {
            latex_keys.push(e.bibtex_key.clone());
            issues.push(Issue {
                kind: "latex_escape".into(),
                key: Some(e.bibtex_key.clone()),
                detail: format!("LaTeX escapes in: {}", affected.join(", ")),
                fixable: true,
            });
        }
    }

    // ── 8. Orphaned notes (notes with no matching entry) ─────────────────────
    let entry_keys: HashSet<String> = good_entries.iter().map(|e| e.bibtex_key.clone()).collect();
    if notes_dir.exists() {
        for f in std::fs::read_dir(notes_dir)?.flatten() {
            let path = f.path();
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
                if !entry_keys.contains(&stem) {
                    issues.push(Issue {
                        kind: "orphaned_note".into(),
                        key: Some(stem.clone()),
                        detail: format!("Note '{}.md' has no matching DB entry", stem),
                        fixable: false,
                    });
                }
            }
        }
    }

    // ── Stats for header ──────────────────────────────────────────────────────
    let total_entries = good_entries.len() + malformed_indices.len();
    let pdf_count = if bibox_dir.exists() {
        std::fs::read_dir(bibox_dir).map(|rd| {
            rd.flatten().filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("pdf")).count()
        }).unwrap_or(0)
    } else { 0 };
    let note_count = if notes_dir.exists() {
        std::fs::read_dir(notes_dir).map(|rd| {
            rd.flatten().filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("md")).count()
        }).unwrap_or(0)
    } else { 0 };

    // ── Output (JSON) ────────────────────────────────────────────────────────
    if json {
        let result = serde_json::json!({
            "total_entries": total_entries,
            "pdf_count": pdf_count,
            "note_count": note_count,
            "issues": issues,
            "issue_count": issues.len(),
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }

    // ── Output (human) ──────────────────────────────────────────────────────
    let bar = "═".repeat(52);
    println!();
    println!("  bibox doctor {}", bar);
    println!();
    println!("    Database   {}", db_path.display());
    println!("    Entries    {}    PDFs  {}    Notes  {}", total_entries, pdf_count, note_count);
    println!();

    // Checks summary
    let checks: &[(&str, &str, &str)] = &[
        ("malformed_entry", "All entries parseable",         "malformed entries"),
        ("bad_citekey",     "All citekeys are clean ASCII",  "citekeys with bad chars"),
        ("duplicate_key",   "No duplicate citekeys",         "duplicate citekeys"),
        ("missing_pdf",     "All registered PDFs on disk",   "missing PDF files"),
        ("orphaned_pdf",    "No orphaned PDFs",              "orphaned PDFs"),
        ("missing_title",   "All entries have titles",       "entries without title"),
        ("dirty_title",     "No BibTeX braces in text",      "entries with {} in text"),
        ("latex_escape",    "No LaTeX escapes in text",      "entries with LaTeX escapes"),
        ("orphaned_note",   "No orphaned notes",             "orphaned notes"),
    ];

    println!("  --- Checks {}", "-".repeat(44));
    println!();
    for (kind, ok_msg, fail_msg) in checks {
        let count = issues.iter().filter(|i| i.kind == *kind).count();
        if count == 0 {
            println!("    OK    {}", ok_msg);
        } else {
            let fixable = issues.iter().any(|i| i.kind == *kind && i.fixable);
            let tag = if fixable { "FIX " } else { "WARN" };
            println!("    {}  {} ({})", tag, fail_msg, count);
        }
    }
    println!();

    if issues.is_empty() {
        println!("    All clear. No issues found.");
        println!();
        return Ok(());
    }

    // Detail sections for each issue kind that has problems
    let kinds = ["malformed_entry", "bad_citekey", "duplicate_key", "missing_pdf", "orphaned_pdf",
                  "missing_title", "dirty_title", "latex_escape", "orphaned_note"];
    for kind in &kinds {
        let group: Vec<&Issue> = issues.iter().filter(|i| i.kind == *kind).collect();
        if group.is_empty() { continue; }
        let (label, fix_hint) = match *kind {
            "malformed_entry" => ("Malformed entries",       "Manual repair needed"),
            "bad_citekey"     => ("Bad citekey chars",        "--fix sanitizes to ASCII"),
            "duplicate_key"   => ("Duplicate citekeys",      "Use `bibox edit` to rename"),
            "missing_pdf"     => ("Missing PDFs",            "--fix clears file_path"),
            "orphaned_pdf"    => ("Orphaned PDFs",           "--fix deletes files"),
            "missing_title"   => ("No title",                "Use `bibox edit --title`"),
            "dirty_title"     => ("BibTeX braces in text",   "--fix strips braces"),
            "latex_escape"    => ("LaTeX escapes in text",   "--fix decodes to Unicode"),
            "orphaned_note"   => ("Orphaned notes",          "Delete manually or re-add entry"),
            _                 => (kind.as_ref(), ""),
        };
        let fixable = group.iter().any(|i| i.fixable);
        let tag = if fixable { "FIX " } else { "WARN" };
        println!("  {} {} ({}) {}", tag, label, group.len(),
            if !fix_hint.is_empty() { format!("  {}", fix_hint) } else { String::new() });
        for issue in &group {
            let key_str = issue.key.as_deref().unwrap_or("-");
            // Compact detail: just show key + relevant info
            let short_detail = if *kind == "dirty_title" {
                // Extract field names from detail
                issue.detail.split("field(s): ")
                    .nth(1)
                    .and_then(|s| s.split(" —").next())
                    .unwrap_or(&issue.detail)
                    .to_string()
            } else if *kind == "latex_escape" {
                issue.detail.split("in: ")
                    .nth(1)
                    .unwrap_or(&issue.detail)
                    .to_string()
            } else if *kind == "orphaned_note" {
                format!("{}.md", key_str)
            } else {
                issue.detail.clone()
            };
            println!("        {}  {}", key_str, short_detail);
        }
        println!();
    }

    // Summary footer
    let fixable_count = issues.iter().filter(|i| i.fixable).count();
    let manual_count = issues.iter().filter(|i| !i.fixable).count();
    println!("  {}", "=".repeat(56));
    let mut parts = vec![];
    if fixable_count > 0 { parts.push(format!("{} fixable", fixable_count)); }
    if manual_count > 0  { parts.push(format!("{} manual", manual_count)); }
    print!("    {}", parts.join("  |  "));
    if fixable_count > 0 && !fix {
        print!("  |  Run `bibox doctor --fix` to repair");
    }
    println!();
    println!();

    // ── Fix ──────────────────────────────────────────────────────────────────
    if fix && fixable_count > 0 {
        println!("  --- Fixing {}", "-".repeat(43));
        println!();

        let mut fixed = 0;
        let mut db = crate::storage::load_db(&db_path)?;

        // Sanitize bad citekeys (two-pass to avoid borrow conflict)
        if !bad_key_keys.is_empty() {
            // Pass 1: compute new keys
            let renames: Vec<(String, String)> = bad_key_keys.iter().map(|old_key| {
                let clean: String = old_key.chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                    .collect();
                let new_key = crate::storage::generate_unique_key(&db, &clean);
                (old_key.clone(), new_key)
            }).collect();
            // Pass 2: apply
            for (old_key, new_key) in &renames {
                if let Some(e) = db.entries.iter_mut().find(|e| &e.bibtex_key == old_key) {
                    if let Some(ref fp) = e.file_path {
                        let old_path = bibox_dir.join(fp);
                        let new_fp = format!("{}.pdf", new_key);
                        let new_path = bibox_dir.join(&new_fp);
                        if old_path.exists() { let _ = std::fs::rename(&old_path, &new_path); }
                        e.file_path = Some(new_fp);
                    }
                    let old_note = notes_dir.join(format!("{}.md", old_key));
                    if old_note.exists() {
                        let new_note = notes_dir.join(format!("{}.md", new_key));
                        let _ = std::fs::rename(&old_note, &new_note);
                    }
                    println!("    OK    {} -> {}", old_key, new_key);
                    e.bibtex_key = new_key.clone();
                    fixed += 1;
                }
            }
        }

        // Clear missing file_path references
        if !missing_pdf_keys.is_empty() {
            for e in db.entries.iter_mut() {
                if missing_pdf_keys.contains(&e.bibtex_key) {
                    e.file_path = None;
                    fixed += 1;
                }
            }
            println!("    OK    Cleared {} missing file_path ref(s)", missing_pdf_keys.len());
        }

        // Strip BibTeX braces from dirty text fields
        if !dirty_title_keys.is_empty() {
            let mut field_count = 0;
            for e in db.entries.iter_mut() {
                if dirty_title_keys.contains(&e.bibtex_key) {
                    for field in [&mut e.title, &mut e.journal, &mut e.booktitle,
                                  &mut e.publisher, &mut e.editor] {
                        if let Some(v) = field.as_ref() {
                            if v.contains('{') || v.contains('}') {
                                *field = Some(strip_bibtex_braces(v));
                                field_count += 1;
                            }
                        }
                    }
                }
            }
            fixed += dirty_title_keys.len();
            println!("    OK    Stripped braces from {} entries ({} fields)", dirty_title_keys.len(), field_count);
        }

        // Decode LaTeX escapes
        if !latex_keys.is_empty() {
            let mut field_count = 0;
            for e in db.entries.iter_mut() {
                if latex_keys.contains(&e.bibtex_key) {
                    for field in [&mut e.title, &mut e.journal, &mut e.booktitle,
                                  &mut e.publisher, &mut e.editor] {
                        if let Some(v) = field.as_ref() {
                            if has_latex_escapes(v) {
                                *field = Some(decode_latex(v));
                                field_count += 1;
                            }
                        }
                    }
                    let mut author_changed = false;
                    for a in e.author.iter_mut() {
                        if has_latex_escapes(a) {
                            *a = decode_latex(a);
                            author_changed = true;
                        }
                    }
                    if author_changed { field_count += 1; }
                }
            }
            fixed += latex_keys.len();
            println!("    OK    Decoded LaTeX in {} entries ({} fields)", latex_keys.len(), field_count);
        }

        // Save DB once for all entry-level fixes
        if fixed > 0 {
            crate::storage::save_db(&db, &db_path)?;
        }

        // Delete orphaned PDFs (file system, not DB)
        if !orphaned_pdfs.is_empty() {
            for path in &orphaned_pdfs {
                if let Err(e) = std::fs::remove_file(path) {
                    eprintln!("    FAIL  Could not delete {}: {}", path.display(), e);
                } else {
                    fixed += 1;
                }
            }
            println!("    OK    Deleted {} orphaned PDF(s)", orphaned_pdfs.len());
        }

        println!();
        println!("  {}", "=".repeat(56));
        println!("    {} issue(s) fixed.", fixed);
        println!();
    }

    Ok(())
}

pub fn cmd_agent_guide(json: bool) -> Result<()> {
    if json {
        let result = serde_json::json!({
            "guide": AGENT_GUIDE,
            "version": env!("CARGO_PKG_VERSION"),
        });
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        print!("{}", AGENT_GUIDE);
    }
    Ok(())
}
