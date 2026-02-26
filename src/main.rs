use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod arxiv;
mod bibtex;
mod commands;
mod config;
mod crossref;
mod i18n;
mod interactive;
mod models;
mod pdf;
mod storage;
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
    command: Commands,
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
        /// Filter by collection name
        #[arg(long)]
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
    },

    /// Reconcile the bibox directory with the database
    Sync,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = load_config()?;

    match cli.command {
        Commands::Add {
            file,
            doi,
            to,
            key,
            title,
            author,
            year,
            r#type,
            journal,
            publisher,
            booktitle,
        } => {
            commands::cmd_add(
                file, to, doi, key, title, author, year, r#type, journal, publisher, booktitle,
                &config,
            )
            .await?;
        }

        Commands::List {
            collection,
            r#type,
            tag,
            year,
            limit,
        } => {
            commands::cmd_list(collection, r#type, tag, year, limit, &config)?;
        }

        Commands::Search {
            query,
            collection,
            field,
        } => {
            commands::cmd_search(query, collection, field, &config)?;
        }

        Commands::Show { key } => {
            commands::cmd_show(key, &config)?;
        }

        Commands::Edit {
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
        } => {
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

        Commands::Delete { key, yes } => {
            commands::cmd_delete(key, yes, &config)?;
        }

        Commands::Collect { key, collections } => {
            commands::cmd_collect(key, collections, &config)?;
        }

        Commands::Uncollect { key, collection } => {
            commands::cmd_uncollect(key, collection, &config)?;
        }

        Commands::Meta {
            key,
            doi,
            title,
            author,
            year,
            journal,
            publisher,
            booktitle,
        } => {
            commands::cmd_meta(
                key, doi, title, author, year, journal, publisher, booktitle, &config,
            )
            .await?;
        }

        Commands::Import { file, to } => {
            commands::cmd_import(file, to, &config)?;
        }

        Commands::Out {
            collection,
            key,
            r#type,
            tag,
            output,
            clipboard,
            as_pdf,
            zip,
        } => {
            commands::cmd_out(
                collection, key, output, clipboard, r#type, tag, as_pdf, zip, &config,
            )?;
        }

        Commands::Sync => {
            commands::cmd_sync(&config)?;
        }
    }

    Ok(())
}
