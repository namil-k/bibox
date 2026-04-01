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
mod notes;
mod openlibrary;
mod pdf;
mod storage;
mod tui;
mod unpaywall;
mod url_resolver;

use config::load_config;

#[derive(Parser)]
#[command(
    name = "bibox",
    about = "Terminal-based bibliography manager with three-panel TUI and AI-agent-friendly notes",
    long_about = "bibox is a terminal-based bibliography manager built in Rust.\n\n\
Add papers by PDF, DOI, ISBN, arXiv ID, or URL — metadata is fetched automatically \
from Crossref, arXiv, and OpenLibrary. Manage your library through a three-panel TUI \
(collections | entries | preview) or a scriptable CLI.\n\n\
Notes are stored as Markdown files with section-level read/write via --stdin and --section, \
designed for AI agent workflows.\n\n\
Use `bibox init <path>` to create a portable home directory that can be synced with Git.\n\n\
Run `bibox` with no arguments to launch the TUI. Press ? for keybindings.",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a new entry from PDF, DOI, ISBN, arXiv ID, URL, or title search
    #[command(after_long_help = "\
Examples:
  bibox add paper.pdf --to ml
  bibox add --doi 10.1145/3290605.3300907
  bibox add --arxiv 2301.12345
  bibox add --isbn 978-0-13-468599-1
  bibox add --url https://arxiv.org/abs/2301.12345
  bibox add --search \"attention is all you need\"")]
    Add {
        /// PDF file to add
        file: Option<PathBuf>,

        /// DOI to look up (skips PDF if provided alone)
        #[arg(long)]
        doi: Option<String>,

        /// ISBN to look up via Open Library (skips PDF if provided alone)
        #[arg(long)]
        isbn: Option<String>,

        /// arXiv ID to look up (e.g., 2301.12345)
        #[arg(long, conflicts_with_all = ["doi", "isbn", "file", "url", "search"])]
        arxiv: Option<String>,

        /// URL to resolve (academic paper page)
        #[arg(long, conflicts_with_all = ["doi", "isbn", "file", "arxiv", "search"])]
        url: Option<String>,

        /// Search Crossref by title and select interactively
        #[arg(long, conflicts_with_all = ["doi", "isbn", "file", "arxiv", "url"])]
        search: Option<String>,

        /// Auto-select search result by index (0-based, for non-interactive/AI use)
        #[arg(long, requires = "search")]
        index: Option<usize>,

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

        /// Output added entry as JSON (for scripting and AI agents)
        #[arg(long)]
        json: bool,
    },

    /// List entries, optionally filtered by collection, type, tag, or year
    #[command(after_long_help = "\
Examples:
  bibox list                  # Show all collections with counts
  bibox list cs               # List entries in 'cs' collection
  bibox list --type article   # Only articles
  bibox list --year 2024      # Only 2024 entries")]
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

        /// Output as JSON (for scripting and AI agents)
        #[arg(long)]
        json: bool,
    },

    /// Search entries by keyword across all fields. Copies citekey to clipboard on Enter
    #[command(after_long_help = "\
Examples:
  bibox search \"transformer\"
  bibox search \"kim\" --field author
  bibox search \"2024\" --collection ml
  bibox search \"transformer\" --json")]
    Search {
        /// Search query
        query: String,

        /// Restrict search to a collection
        #[arg(long)]
        collection: Option<String>,

        /// Search in a specific field: title, author, journal, doi, tag
        #[arg(long)]
        field: Option<String>,

        /// Output as JSON (for scripting and AI agents)
        #[arg(long)]
        json: bool,
    },

    /// Show full metadata of an entry (title, authors, DOI, tags, collections, file path)
    #[command(after_long_help = "Examples:\n  bibox show kim2025rust\n  bibox show kim2025rust --json")]
    Show {
        /// Citation key or entry ID
        key: String,

        /// Output as JSON (for scripting and AI agents)
        #[arg(long)]
        json: bool,
    },

    /// Edit entry metadata. When --doi is provided, re-fetches from Crossref (preserving existing values)
    #[command(after_long_help = "\
Examples:
  bibox edit kim2025rust --title \"New Title\" --year 2025
  bibox edit kim2025rust --doi 10.1234/new   # re-fetch metadata
  bibox edit kim2025rust --tags-add \"ml,nlp\"")]
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

    /// Delete an entry and its associated PDF file
    #[command(after_long_help = "Examples:\n  bibox delete kim2025rust\n  bibox delete kim2025rust -y   # skip confirmation")]
    Delete {
        /// Citation key or entry ID
        key: String,

        /// Skip confirmation prompt
        #[arg(short = 'y', long)]
        yes: bool,
    },

    /// Add an entry to one or more collections
    #[command(after_long_help = "Examples:\n  bibox collect kim2025rust ml systems")]
    Collect {
        /// Citation key or entry ID
        key: String,

        /// Collection name(s)
        #[arg(required = true)]
        collections: Vec<String>,
    },

    /// Remove an entry from a collection
    #[command(after_long_help = "Examples:\n  bibox uncollect kim2025rust ml")]
    Uncollect {
        /// Citation key or entry ID
        key: String,

        /// Collection name to remove from
        collection: String,
    },

    /// Import entries from a BibTeX (.bib) file
    #[command(after_long_help = "Examples:\n  bibox import refs.bib\n  bibox import refs.bib --to ml")]
    Import {
        /// Path to .bib file
        file: PathBuf,

        /// Assign all imported entries to a collection
        #[arg(long)]
        to: Option<String>,
    },

    /// Export entries as BibTeX, YAML, RIS, or CSV. Optionally include PDF files
    #[command(alias = "out", after_long_help = "\
Examples:
  bibox export                                  # all entries as BibTeX
  bibox export kim2025 dijkstra1968             # specific entries
  bibox export --collection cs --format ris     # collection as RIS
  bibox export --include-pdf                    # BibTeX + PDFs
  bibox export --as-pdf --zip                   # PDFs only, zipped
  bibox export kim2025 --clipboard              # copy to clipboard
  bibox export --notes-only -o ~/notes          # copy note .md files to folder
  bibox export --collection ml --notes-only -o ~/ml-notes")]
    Export {
        /// Citation keys to export (omit for collection/all)
        keys: Vec<String>,

        /// Export entries from this collection
        #[arg(long)]
        collection: Option<String>,

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

        /// Also export PDF files alongside the bibliography
        #[arg(long)]
        include_pdf: bool,

        /// Compress output into a ZIP archive
        #[arg(long)]
        zip: bool,

        /// Output format: bibtex (default), yaml, ris, csv
        #[arg(long, default_value = "bibtex")]
        format: String,

        /// Export note (.md) files to the specified directory (use -o to set destination)
        #[arg(long)]
        notes_only: bool,
    },

    /// Open the PDF file for an entry in the system viewer
    #[command(after_long_help = "Examples:\n  bibox open kim2025rust")]
    Open {
        /// Citation key or entry ID
        key: String,
    },

    /// Reconcile the bibox directory with the database (detect orphaned PDFs, missing entries)
    #[command(after_long_help = "Examples:\n  bibox sync\n  bibox sync --yes   # non-interactive")]
    Sync {
        /// Auto-confirm all prompts (for scripting and AI agents)
        #[arg(long, short = 'y')]
        yes: bool,

        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },

    /// Initialize a portable bibox home directory. All data (db, pdfs, notes) lives in one folder
    #[command(after_long_help = "\
Examples:
  bibox init ~/bibox              # create home
  bibox init ~/bibox --migrate    # create + copy existing data")]
    Init {
        /// Path to the home directory (e.g., ~/bibox)
        path: PathBuf,

        /// Migrate existing data from Library to the new home
        #[arg(long)]
        migrate: bool,

        /// Output result as JSON
        #[arg(long)]
        json: bool,
    },

    /// Read, write, or edit per-entry Markdown notes. Supports section-level updates for AI agents
    #[command(after_long_help = "\
Examples:
  bibox note kim2025rust                                           # open in $EDITOR
  bibox note kim2025rust --show                                    # print to stdout
  bibox note kim2025rust --path                                    # print file path
  bibox note kim2025rust --template ai-summary                     # init from template
  echo \"text\" | bibox note kim2025rust --stdin --section \"Summary\"  # write section
  bibox note kim2025rust --from notes.md --section \"Results\"        # write from file")]
    Note {
        /// Citation key or entry ID
        key: String,

        /// Read content from stdin (non-interactive)
        #[arg(long)]
        stdin: bool,

        /// Read content from a file
        #[arg(long, conflicts_with = "stdin")]
        from: Option<PathBuf>,

        /// Target a specific ## section (requires --stdin or --from)
        #[arg(long)]
        section: Option<String>,

        /// Initialize note from a template
        #[arg(long)]
        template: Option<String>,

        /// Print note content to stdout
        #[arg(long, conflicts_with_all = ["stdin", "from", "template"])]
        show: bool,

        /// Print note file path to stdout
        #[arg(long, conflicts_with_all = ["stdin", "from", "template", "show"])]
        path: bool,

        /// Allow --template to overwrite existing note
        #[arg(long)]
        force: bool,

        /// Output as JSON (for --show and --path)
        #[arg(long)]
        json: bool,
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

    /// Show current configuration and all resolved paths
    Config {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Print a structured guide for AI agents describing all bibox capabilities and workflows
    #[command(name = "agent-guide")]
    AgentGuide {
        /// Output as JSON (structured for machine parsing)
        #[arg(long)]
        json: bool,
    },

    /// Check for a newer version and update via cargo install
    Update {
        /// Check for updates without installing
        #[arg(long)]
        check: bool,
    },

    /// Manage note templates (list, show, create, edit, delete, export built-ins)
    Template {
        #[command(subcommand)]
        action: TemplateAction,
    },
}

#[derive(Subcommand)]
enum TemplateAction {
    /// List all available templates (built-in + custom)
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Print template content to stdout
    Show {
        /// Template name (e.g. "ai-summary", "reading-notes", or custom name)
        name: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Create a new custom template from stdin or $EDITOR
    Create {
        /// Template name (used as filename: <name>.md)
        name: String,

        /// Read template content from stdin
        #[arg(long)]
        stdin: bool,
    },

    /// Edit an existing template in $EDITOR. For built-ins, exports to custom dir first
    Edit {
        /// Template name
        name: String,
    },

    /// Delete a custom template
    Delete {
        /// Template name
        name: String,
    },

    /// Export a built-in template to the custom templates directory for modification
    Export {
        /// Built-in template name (e.g. "ai-summary", "reading-notes")
        name: String,
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
            arxiv,
            url,
            search,
            index,
            to,
            key,
            title,
            author,
            year,
            r#type,
            journal,
            publisher,
            booktitle,
            json,
        }) => {
            commands::cmd_add(
                file, to, doi, isbn, arxiv, url, search, index, key, title, author, year, r#type, journal,
                publisher, booktitle, json, &config,
            )
            .await?;
        }

        Some(Commands::List {
            collection,
            r#type,
            tag,
            year,
            limit,
            json,
        }) => {
            commands::cmd_list(collection, r#type, tag, year, limit, json, &config)?;
        }

        Some(Commands::Search {
            query,
            collection,
            field,
            json,
        }) => {
            commands::cmd_search(query, collection, field, json, &config)?;
        }

        Some(Commands::Show { key, json }) => {
            commands::cmd_show(key, json, &config)?;
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
            ).await?;
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

        Some(Commands::Import { file, to }) => {
            commands::cmd_import(file, to, &config)?;
        }

        Some(Commands::Export {
            keys,
            collection,
            r#type,
            tag,
            output,
            clipboard,
            as_pdf,
            include_pdf,
            zip,
            format,
            notes_only,
        }) => {
            commands::cmd_export(
                keys, collection, output, clipboard, r#type, tag, as_pdf, include_pdf, zip, format, notes_only, &config,
            )?;
        }

        Some(Commands::Open { key }) => {
            commands::cmd_open(key, &config)?;
        }

        Some(Commands::Sync { yes, json }) => {
            commands::cmd_sync(yes, json, &config)?;
        }

        Some(Commands::Init { path, migrate, json }) => {
            commands::cmd_init(path, migrate, json, &config)?;
        }

        Some(Commands::Note { key, stdin, from, section, template, show, path, force, json }) => {
            commands::cmd_note(key, stdin, from, section, template, show, path, force, json, &config)?;
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

        Some(Commands::Config { json }) => {
            commands::cmd_config(json, &config)?;
        }

        Some(Commands::AgentGuide { json }) => {
            commands::cmd_agent_guide(json)?;
        }

        Some(Commands::Update { check }) => {
            commands::cmd_update(check)?;
        }

        Some(Commands::Template { action }) => {
            match action {
                TemplateAction::List { json } => commands::cmd_template_list(json, &config)?,
                TemplateAction::Show { name, json } => commands::cmd_template_show(&name, json, &config)?,
                TemplateAction::Create { name, stdin } => commands::cmd_template_create(&name, stdin, &config)?,
                TemplateAction::Edit { name } => commands::cmd_template_edit(&name, &config)?,
                TemplateAction::Delete { name } => commands::cmd_template_delete(&name, &config)?,
                TemplateAction::Export { name } => commands::cmd_template_export(&name, &config)?,
            }
        }
    }

    Ok(())
}
