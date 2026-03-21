use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod arxiv;
mod bibtex;
mod commands;
mod config;
mod crossref;
mod git;
mod i18n;
mod interactive;
mod models;
mod openlibrary;
mod pdf;
mod storage;
mod tui;
mod unpaywall;

use config::load_config;

#[derive(Parser)]
#[command(
    name = "bibox",
    about = "PDF-based bibliography manager",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a PDF or DOI-only entry
    Add {
        /// PDF file to add
        file: Option<PathBuf>,

        /// DOI to look up (skips PDF if provided alone)
        #[arg(long)]
        doi: Option<String>,

        /// ISBN to look up via Open Library (skips PDF if provided alone)
        #[arg(long)]
        isbn: Option<String>,

        /// Assign to a collection
        #[arg(long)]
        to: Option<String>,

        /// Override auto-generated citation key
        #[arg(long)]
        key: Option<String>,

        /// Override title
        #[arg(long)]
        title: Option<String>,

        /// Override author(s), semicolon-separated  e.g. "Kim, J; Lee, S"
        #[arg(long)]
        author: Option<String>,

        /// Override publication year
        #[arg(long)]
        year: Option<u32>,

        /// Entry type: article, book, inproceedings, misc
        #[arg(long, value_name = "TYPE")]
        r#type: Option<String>,

        /// Journal name
        #[arg(long)]
        journal: Option<String>,

        /// Publisher name
        #[arg(long)]
        publisher: Option<String>,

        /// Booktitle (for inproceedings)
        #[arg(long)]
        booktitle: Option<String>,
    },

    /// List entries
    List {
        /// Collection name (omit to list all collections)
        collection: Option<String>,

        /// Filter by type: article, book, inproceedings, misc
        #[arg(long, value_name = "TYPE")]
        r#type: Option<String>,

        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,

        /// Filter by year
        #[arg(long)]
        year: Option<u32>,

        /// Maximum number of entries to show
        #[arg(long)]
        limit: Option<usize>,
    },

    /// Search entries interactively (copies citekey to clipboard on Enter)
    Search {
        /// Search query
        query: String,

        /// Restrict search to a collection
        #[arg(long)]
        collection: Option<String>,

        /// Search in a specific field: title, author, journal, doi, tag
        #[arg(long)]
        field: Option<String>,
    },

    /// Show full details of an entry
    Show {
        /// Citation key or entry ID
        key: String,
    },

    /// Edit an existing entry
    Edit {
        /// Citation key or entry ID
        key: String,

        /// New title
        #[arg(long)]
        title: Option<String>,

        /// New author(s), semicolon-separated
        #[arg(long)]
        author: Option<String>,

        /// New year
        #[arg(long)]
        year: Option<u32>,

        /// New DOI
        #[arg(long)]
        doi: Option<String>,

        /// New journal
        #[arg(long)]
        journal: Option<String>,

        /// New publisher
        #[arg(long)]
        publisher: Option<String>,

        /// New booktitle
        #[arg(long)]
        booktitle: Option<String>,

        /// New volume
        #[arg(long)]
        volume: Option<String>,

        /// New number/issue
        #[arg(long)]
        number: Option<String>,

        /// New pages  e.g. "1--10"
        #[arg(long)]
        pages: Option<String>,

        /// Note
        #[arg(long)]
        note: Option<String>,

        /// Tags to add, comma-separated
        #[arg(long)]
        tags_add: Option<String>,

        /// Tags to remove, comma-separated
        #[arg(long)]
        tags_remove: Option<String>,
    },

    /// Delete an entry (and its PDF)
    Delete {
        /// Citation key or entry ID
        key: String,

        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// Add entry to one or more collections
    Collect {
        /// Citation key or entry ID
        key: String,

        /// Collection name(s)
        #[arg(required = true)]
        collections: Vec<String>,
    },

    /// Remove entry from a collection
    Uncollect {
        /// Citation key or entry ID
        key: String,

        /// Collection name to remove from
        collection: String,
    },

    /// Fetch/update metadata for an existing entry via DOI or manual input
    Meta {
        /// Citation key or entry ID
        key: String,

        /// DOI to look up from Crossref
        #[arg(long)]
        doi: Option<String>,

        /// Override title
        #[arg(long)]
        title: Option<String>,

        /// Override author(s), semicolon-separated
        #[arg(long)]
        author: Option<String>,

        /// Override year
        #[arg(long)]
        year: Option<u32>,

        /// Override journal
        #[arg(long)]
        journal: Option<String>,

        /// Override publisher
        #[arg(long)]
        publisher: Option<String>,

        /// Override booktitle
        #[arg(long)]
        booktitle: Option<String>,
    },

    /// Import entries from a .bib file
    Import {
        /// Path to .bib file
        file: PathBuf,

        /// Assign all imported entries to a collection
        #[arg(long)]
        to: Option<String>,
    },

    /// Export BibTeX or PDFs
    Out {
        /// Export entries from this collection
        #[arg(long)]
        collection: Option<String>,

        /// Export a single entry by citation key
        #[arg(long)]
        key: Option<String>,

        /// Filter by entry type
        #[arg(long, value_name = "TYPE")]
        r#type: Option<String>,

        /// Filter by tag
        #[arg(long)]
        tag: Option<String>,

        /// Output file path (default: auto-generated filename)
        #[arg(long, short = 'o')]
        output: Option<PathBuf>,

        /// Copy BibTeX to clipboard instead of writing a file
        #[arg(long)]
        clipboard: bool,

        /// Export PDF files instead of BibTeX
        #[arg(long)]
        as_pdf: bool,

        /// Compress --as-pdf output into a ZIP archive
        #[arg(long, requires = "as_pdf")]
        zip: bool,

        /// Output format: bibtex (default), yaml, ris, csv
        #[arg(long, default_value = "bibtex")]
        format: String,
    },

    /// Open the PDF for an entry
    Open {
        /// Citation key or entry ID
        key: String,
    },

    /// Reconcile the bibox directory with the database
    Sync,

    /// Open or create a per-entry note file in $EDITOR
    Note {
        /// Citation key or entry ID
        key: String,
    },

    /// Bulk-update fields across multiple entries
    Modify {
        /// Field=value pairs to set, e.g. year=2024 journal="Nature"
        #[arg(required = true)]
        assignments: Vec<String>,

        /// Filter: collection:<name>, tag:<tag>, type:<type>, year:<year>
        #[arg(long, short = 'f')]
        filter: Option<String>,

        /// Apply to all entries (required if no --filter)
        #[arg(long)]
        all: bool,

        /// Skip confirmation
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// Review entries interactively one by one
    Review {
        /// Filter by collection
        #[arg(long, short = 'c')]
        collection: Option<String>,

        /// Filter: same syntax as modify (collection:<name>, tag:<tag>, type:<type>, year:<year>)
        #[arg(long, short = 'f')]
        filter: Option<String>,

        /// Only show entries without the "reviewed" tag
        #[arg(long)]
        unreviewed: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = load_config()?;

    match cli.command {
        None => {
            tui::run_tui(&config)?;
        }

        Some(Commands::Add {
            file,
            doi,
            isbn,
            to,
            key,
            title,
            author,
            year,
            r#type,
            journal,
            publisher,
            booktitle,
        }) => {
            commands::cmd_add(
                file, to, doi, isbn, key, title, author, year, r#type, journal, publisher, booktitle,
                &config,
            )
            .await?;
        }

        Some(Commands::List {
            collection,
            r#type,
            tag,
            year,
            limit,
        }) => {
            commands::cmd_list(collection, r#type, tag, year, limit, &config)?;
        }

        Some(Commands::Search {
            query,
            collection,
            field,
        }) => {
            commands::cmd_search(query, collection, field, &config)?;
        }

        Some(Commands::Show { key }) => {
            commands::cmd_show(key, &config)?;
        }

        Some(Commands::Edit {
            key,
            title,
            author,
            year,
            doi,
            journal,
            publisher,
            booktitle,
            volume,
            number,
            pages,
            note,
            tags_add,
            tags_remove,
        }) => {
            commands::cmd_edit(
                key,
                title,
                author,
                year,
                doi,
                journal,
                publisher,
                booktitle,
                volume,
                number,
                pages,
                note,
                tags_add,
                tags_remove,
                &config,
            )?;
        }

        Some(Commands::Delete { key, yes }) => {
            commands::cmd_delete(key, yes, &config)?;
        }

        Some(Commands::Collect { key, collections }) => {
            commands::cmd_collect(key, collections, &config)?;
        }

        Some(Commands::Uncollect { key, collection }) => {
            commands::cmd_uncollect(key, collection, &config)?;
        }

        Some(Commands::Meta {
            key,
            doi,
            title,
            author,
            year,
            journal,
            publisher,
            booktitle,
        }) => {
            commands::cmd_meta(
                key, doi, title, author, year, journal, publisher, booktitle, &config,
            )
            .await?;
        }

        Some(Commands::Import { file, to }) => {
            commands::cmd_import(file, to, &config)?;
        }

        Some(Commands::Out {
            collection,
            key,
            r#type,
            tag,
            output,
            clipboard,
            as_pdf,
            zip,
            format,
        }) => {
            commands::cmd_out(
                collection, key, output, clipboard, r#type, tag, as_pdf, zip, format, &config,
            )?;
        }

        Some(Commands::Open { key }) => {
            commands::cmd_open(key, &config)?;
        }

        Some(Commands::Sync) => {
            commands::cmd_sync(&config)?;
        }

        Some(Commands::Note { key }) => {
            commands::cmd_note(key, &config)?;
        }

        Some(Commands::Modify {
            assignments,
            filter,
            all,
            yes,
        }) => {
            commands::cmd_modify(assignments, filter, all, yes, &config)?;
        }

        Some(Commands::Review {
            collection,
            filter,
            unreviewed,
        }) => {
            commands::cmd_review(collection, filter, unreviewed, &config)?;
        }
    }

    Ok(())
}
