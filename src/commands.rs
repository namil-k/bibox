use anyhow::{Context, Result};
use chrono::Local;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::bibtex::{entries_to_bibtex, entry_to_filename};
use crate::config::Config;
use crate::crossref;
use crate::interactive::{interactive_select, SelectItem};
use crate::models::{Entry, EntryType};
use crate::pdf;
use crate::storage::{
    filter_entries, find_by_key, find_by_key_mut, generate_bibtex_key, generate_unique_key,
    load_db, save_db, search_entries,
};
use crate::arxiv;
use crate::unpaywall;

fn db_path_from_config(_config: &Config) -> PathBuf {
    crate::config::db_path()
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
    key_arg: Option<String>,
    title_arg: Option<String>,
    author_arg: Option<String>,
    year_arg: Option<u32>,
    entry_type_arg: Option<String>,
    journal_arg: Option<String>,
    publisher_arg: Option<String>,
    booktitle_arg: Option<String>,
    config: &Config,
) -> Result<()> {
    let db_path = db_path_from_config(config);
    let mut db = load_db(&db_path)?;

    std::fs::create_dir_all(&config.bibox_dir)?;

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
                tags: vec![],
                note: None,
                collections: vec![],
                file_path: None,
                created_at: String::new(),
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

    let collections: Vec<String> = to.map(|c| vec![c]).unwrap_or_default();

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
        tags: vec![],
        note: None,
        collections,
        file_path,
        created_at: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
    };

    println!(
        "{}",
        config
            .msgs
            .added(&entry.bibtex_key, entry.title.as_deref().unwrap_or("?"))
    );
    db.entries.push(entry);
    save_db(&db, &db_path)?;

    Ok(())
}

// ── list ─────────────────────────────────────────────────────────────────────

pub fn cmd_list(
    collection: Option<String>,
    entry_type: Option<String>,
    tag: Option<String>,
    year: Option<u32>,
    limit: Option<usize>,
    config: &Config,
) -> Result<()> {
    let db_path = db_path_from_config(config);
    let db = load_db(&db_path)?;

    // No collection + no filters → show collections summary
    if collection.is_none() && entry_type.is_none() && tag.is_none() && year.is_none() {
        return list_collections(&db, config);
    }

    let entries = filter_entries(
        &db,
        collection.as_deref(),
        entry_type.as_deref(),
        tag.as_deref(),
        year,
    );

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

pub fn cmd_show(id_or_key: String, config: &Config) -> Result<()> {
    let db_path = db_path_from_config(config);
    let db = load_db(&db_path)?;

    let entry = find_by_key(&db, &id_or_key)
        .with_context(|| config.msgs.entry_not_found(&id_or_key))?;

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
    if let Some(note) = &entry.note {
        println!("{}: {}", config.msgs.label_note(), note);
    }
    println!("{}: {}", config.msgs.label_created(), entry.created_at);
    println!("{}", sep);

    Ok(())
}

// ── edit ─────────────────────────────────────────────────────────────────────

pub fn cmd_edit(
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
    tags_add: Option<String>,
    tags_remove: Option<String>,
    config: &Config,
) -> Result<()> {
    let db_path = db_path_from_config(config);
    let mut db = load_db(&db_path)?;

    let entry = find_by_key_mut(&mut db, &id_or_key)
        .with_context(|| config.msgs.entry_not_found(&id_or_key))?;

    if let Some(t) = title {
        entry.title = Some(t);
    }
    if let Some(a) = author {
        entry.author = a.split(';').map(|s| s.trim().to_string()).collect();
    }
    if let Some(y) = year {
        entry.year = Some(y);
    }
    if let Some(d) = doi {
        entry.doi = Some(d);
    }
    if let Some(j) = journal {
        entry.journal = Some(j);
    }
    if let Some(p) = publisher {
        entry.publisher = Some(p);
    }
    if let Some(bt) = booktitle {
        entry.booktitle = Some(bt);
    }
    if let Some(v) = volume {
        entry.volume = Some(v);
    }
    if let Some(n) = number {
        entry.number = Some(n);
    }
    if let Some(pg) = pages {
        entry.pages = Some(pg);
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

    // Rename file if metadata changed
    if entry.file_path.is_some() {
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
    save_db(&db, &db_path)?;
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

    db.entries
        .retain(|e| e.bibtex_key != entry.bibtex_key && e.id != entry.id);
    save_db(&db, &db_path)?;
    println!("{}", config.msgs.deleted(&entry.bibtex_key));

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
    let key = entry.bibtex_key.clone();
    save_db(&db, &db_path)?;
    println!("{}", config.msgs.uncollected(&key, &collection));

    Ok(())
}

// ── meta ─────────────────────────────────────────────────────────────────────

pub async fn cmd_meta(
    id_or_key: String,
    doi: Option<String>,
    title: Option<String>,
    author: Option<String>,
    year: Option<u32>,
    journal: Option<String>,
    publisher: Option<String>,
    booktitle: Option<String>,
    config: &Config,
) -> Result<()> {
    let db_path = db_path_from_config(config);
    let mut db = load_db(&db_path)?;

    if let Some(ref doi_str) = doi {
        println!("{}", config.msgs.fetching_crossref());
        match crossref::fetch_metadata(doi_str).await {
            Ok(meta) => {
                let entry = find_by_key_mut(&mut db, &id_or_key)
                    .with_context(|| config.msgs.entry_not_found(&id_or_key))?;

                entry.title = title.or(meta.title);
                if let Some(a) = author {
                    entry.author = a.split(';').map(|s| s.trim().to_string()).collect();
                } else if !meta.authors.is_empty() {
                    entry.author = meta.authors;
                }
                entry.year = year.or(meta.year);
                entry.journal = journal.or(meta.journal);
                entry.publisher = publisher.or(meta.publisher);
                entry.booktitle = booktitle.or(meta.booktitle);
                entry.doi = Some(doi_str.clone());
                entry.volume = meta.volume;
                entry.number = meta.number;
                entry.pages = meta.pages;
                entry.url = meta.url;

                let key = entry.bibtex_key.clone();
                println!("{}", config.msgs.meta_updated(&key));
            }
            Err(e) => anyhow::bail!("{}", config.msgs.doi_lookup_failed(&e.to_string())),
        }
    } else {
        let entry = find_by_key_mut(&mut db, &id_or_key)
            .with_context(|| config.msgs.entry_not_found(&id_or_key))?;

        if let Some(t) = title {
            entry.title = Some(t);
        }
        if let Some(a) = author {
            entry.author = a.split(';').map(|s| s.trim().to_string()).collect();
        }
        if let Some(y) = year {
            entry.year = Some(y);
        }
        if let Some(j) = journal {
            entry.journal = Some(j);
        }
        if let Some(p) = publisher {
            entry.publisher = Some(p);
        }
        if let Some(bt) = booktitle {
            entry.booktitle = Some(bt);
        }

        let key = entry.bibtex_key.clone();
        println!("{}", config.msgs.meta_manual_updated(&key));
    }

    save_db(&db, &db_path)?;
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
            tags: vec![],
            note: raw.note,
            collections,
            file_path: None,
            created_at: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        };

        db.entries.push(entry);
        added += 1;
    }

    save_db(&db, &db_path)?;

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

pub fn cmd_out(
    collection: Option<String>,
    key: Option<String>,
    output: Option<PathBuf>,
    clipboard: bool,
    entry_type: Option<String>,
    tag: Option<String>,
    as_pdf: bool,
    zip: bool,
    config: &Config,
) -> Result<()> {
    let db_path = db_path_from_config(config);
    let db = load_db(&db_path)?;

    let timestamp = Local::now().format("%Y%m%d_%H%M%S").to_string();

    if as_pdf {
        let entries = if let Some(ref k) = key {
            vec![find_by_key(&db, k)
                .with_context(|| config.msgs.entry_not_found(k))?]
        } else {
            filter_entries(
                &db,
                collection.as_deref(),
                entry_type.as_deref(),
                tag.as_deref(),
                None,
            )
        };

        let col_name = collection.as_deref().unwrap_or("bibox");
        let folder_name = format!("{}_{}", col_name, &timestamp[..8]);

        let dest_parent = output.unwrap_or_else(|| {
            dirs::download_dir()
                .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")))
        });
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

        if zip {
            let zip_path = dest_parent.join(format!("{}.zip", folder_name));
            create_zip(&dest_dir, &zip_path)?;
            std::fs::remove_dir_all(&dest_dir)?;
            println!(
                "{}",
                config
                    .msgs
                    .zip_created(&zip_path.to_string_lossy(), copied)
            );
        } else {
            println!(
                "{}",
                config
                    .msgs
                    .folder_created(&dest_dir.to_string_lossy(), copied)
            );
        }

        return Ok(());
    }

    // BibTeX export
    let entries: Vec<&Entry> = if let Some(ref k) = key {
        vec![find_by_key(&db, k)
            .with_context(|| config.msgs.entry_not_found(k))?]
    } else if collection.is_some() || entry_type.is_some() || tag.is_some() {
        filter_entries(
            &db,
            collection.as_deref(),
            entry_type.as_deref(),
            tag.as_deref(),
            None,
        )
    } else {
        db.entries.iter().collect()
    };

    let bibtex = entries_to_bibtex(&entries);

    if clipboard {
        copy_to_clipboard(&bibtex, config)?;
        println!("{}", config.msgs.clipboard_copied_entries(entries.len()));
        return Ok(());
    }

    let col_name = key
        .as_deref()
        .or(collection.as_deref())
        .unwrap_or("references");
    let filename = format!("{}_{}.bib", col_name, timestamp);
    let out_path = output.unwrap_or_else(|| PathBuf::from(&filename));
    std::fs::write(&out_path, &bibtex)?;
    println!(
        "{}",
        config
            .msgs
            .bibtex_saved(&out_path.to_string_lossy(), entries.len())
    );

    Ok(())
}

// ── sync ─────────────────────────────────────────────────────────────────────

pub fn cmd_sync(config: &Config) -> Result<()> {
    let db_path = db_path_from_config(config);
    let mut db = load_db(&db_path)?;

    std::fs::create_dir_all(&config.bibox_dir)?;

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
    let missing: Vec<String> = db_files
        .iter()
        .filter(|fp| !actual_files.contains(fp))
        .cloned()
        .collect();

    for fp in &missing {
        if prompt_confirm(&config.msgs.sync_file_missing(fp)) {
            db.entries.retain(|e| e.file_path.as_deref() != Some(fp));
            println!("{}", config.msgs.sync_removed(fp));
        }
    }

    // Files on disk but not in DB
    let untracked: Vec<String> = actual_files
        .iter()
        .filter(|fp| !db_files.contains(fp))
        .cloned()
        .collect();

    for fp in &untracked {
        println!("{}", config.msgs.sync_new_file(fp));
        if prompt_confirm(config.msgs.add_to_db_prompt()) {
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
                tags: vec![],
                note: Some(config.msgs.sync_added_note().to_string()),
                collections: vec![],
                file_path: Some(fp.clone()),
                created_at: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
            };

            println!("{}", config.msgs.sync_entry_added(&key));
            db.entries.push(entry);
        }
    }

    save_db(&db, &db_path)?;
    println!("{}", config.msgs.sync_complete());

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
}

fn parse_bibtex(content: &str) -> Vec<RawBibEntry> {
    let mut entries = vec![];
    let re_entry = regex::Regex::new(r"@(\w+)\s*\{\s*([^,\s]*)\s*,").unwrap();
    let re_field = regex::Regex::new(r"\b(\w+)\s*=\s*\{([^}]*)\}").unwrap();

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
        };

        for fcap in re_field.captures_iter(body) {
            let field = fcap.get(1).unwrap().as_str().to_lowercase();
            let value = fcap.get(2).unwrap().as_str().trim().to_string();
            match field.as_str() {
                "title" => raw.title = Some(value),
                "author" => raw.author = Some(value),
                "year" => raw.year = value.parse().ok(),
                "journal" => raw.journal = Some(value),
                "volume" => raw.volume = Some(value),
                "number" => raw.number = Some(value),
                "pages" => raw.pages = Some(value),
                "publisher" => raw.publisher = Some(value),
                "editor" => raw.editor = Some(value),
                "edition" => raw.edition = Some(value),
                "isbn" => raw.isbn = Some(value),
                "booktitle" => raw.booktitle = Some(value),
                "doi" => raw.doi = Some(value),
                "url" => raw.url = Some(value),
                "note" => raw.note = Some(value),
                _ => {}
            }
        }

        entries.push(raw);
        pos = body_start + body.len();
    }

    entries
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
