# bibox

<p align="center">
  <img src="assets/bbox_bass.png" alt="bibox" width="320">
</p>

[![Crates.io](https://img.shields.io/crates/v/bibox)](https://crates.io/crates/bibox)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**Using AI agents (OpenClaw) to search and summarize papers?**
**Let them manage your bibliography and notes too.**
**You just browse the TUI. bibox makes it work.**

Just give your agent the GitHub link. It'll figure out the rest.

For humans: browse and edit in the TUI. For agents: manage entries and notes through the CLI.

> **Best for:** researchers who use AI agents for literature review and want papers, notes, and BibTeX in one Git-syncable folder.
>
> **Not for:** GUI, PDF annotation, or Word integration. Use Zotero instead.

**Sources:** Crossref, Open Library, arXiv, Unpaywall
**Formats:** BibTeX, YAML, RIS, CSV, Markdown
**Import:** `.bib` from Zotero, Mendeley, EndNote

## Features

- **Smart import** - Drop a PDF and bibox extracts the DOI, fetches metadata from Crossref, and files it automatically
- **Multiple sources** - Add entries via PDF, DOI, ISBN, arXiv ID, URL, or title search
- **Three-panel TUI** - Collections, entries, and preview (info / notes / PDF) side by side
- **Vim-style navigation** - `hjkl`, `gg`/`G`, `{n}j`, `Ctrl+d/u`, multi-select with `Space`
- **Fetch & refresh** - Press `f` to pull metadata from Crossref, preview changes, pick which fields to update
- **Undo/redo** - `Ctrl+z`/`Ctrl+y` with in-memory snapshots
- **AI-agent-friendly** - Every command supports `--json`. Notes have `--stdin`, `--section`, `--template` for programmatic access
- **Markdown notes** - Per-entry notes with section-level updates, rendered with syntax highlighting in the TUI
- **Sub-collections** - Hierarchical collections with path notation (`digest/2026-04`). Unlimited depth, works like a filesystem
- **Portable home** - `bibox init` puts everything in one Git-syncable folder
- **Export** - BibTeX, YAML, RIS, CSV, notes (`.md`). Include PDFs. Copy to clipboard. Zip it up.
- **Templates** - Built-in and custom note templates with `{{variable}}` substitution
- **Doctor** - `bibox doctor` diagnoses and auto-repairs DB issues: bad citekeys, LaTeX escapes, orphaned files

## Install

**With Rust (macOS / Linux desktop):**

```bash
cargo install bibox
```

**With cargo-binstall (fast, no compile):**

```bash
cargo binstall bibox
```

**On a server (no Rust needed):**

```bash
curl -L https://github.com/namil-k/bibox/releases/latest/download/bibox-x86_64-unknown-linux-musl -o ~/.local/bin/bibox
chmod +x ~/.local/bin/bibox
```

Pre-built binaries for Linux x86_64, macOS arm64, and macOS x86_64 are available on the [releases page](https://github.com/namil-k/bibox/releases/latest).

**Update:**

```bash
bibox update
```

## Quick Start

Add a paper by PDF (auto-extracts DOI → fetches metadata):

```bash
bibox add paper.pdf
```

Add by DOI, arXiv ID, ISBN, or URL:

```bash
bibox add --doi 10.1145/3290605.3300907
```

```bash
bibox add --arxiv 2301.12345
```

```bash
bibox add --isbn 978-0-13-468599-1
```

```bash
bibox add --url https://arxiv.org/abs/2301.12345
```

Search by title on Crossref:

```bash
bibox add --search "attention is all you need"
```

Add a web page, product, or any non-paper reference as misc:

```bash
bibox add --title "Varjo Aero" --url https://varjo.com/products/aero/ --author "Varjo" --year 2024
```

Launch TUI:

```bash
bibox
```

## TUI

```
┌ Collections ─┬ Entries ──────────┬ Info │ Note │ PDF ──┐
│ > All (15)   │  1  kim2025 ◆     │ Title: Rust Syst.. │
│   cs (8)     │  2  dijkstra1968  │ Author: Kim, J.    │
│   ml (5)     │  3  manco2017 ◆   │ Year: 2025         │
│              │                   │ DOI: 10.1234/...   │
├──────────────┴───────────────────┴────────────────────┤
│ / search  s sort  o open  w web  e export  ? help     │
└───────────────────────────────────────────────────────┘
```

### Keybindings

| Key | Action |
|-----|--------|
| `h`/`l` | Focus left/right panel |
| `j`/`k` | Navigate within panel |
| `gg`/`G` | Jump to top/bottom |
| `{n}j` | Move n lines (e.g., `5j`) |
| `Ctrl+d`/`u` | Half-page down/up |
| `Tab` | Switch preview mode (Info → Note → PDF) |
| `Space` | Toggle select entry |
| `V` | Select/deselect all |
| `/` | Search (entries or collections, based on focus) |
| `s` | Sort menu |
| `f` | Fetch/refresh metadata from Crossref (preview changes, select which to apply) |
| `o` | Open PDF (or fetch from web; re-fetches if file missing; opens browser on 403) |
| `A` | Attach a local PDF via file picker (copies and renames to citekey.pdf) |
| `w` | Open paper web page in browser |
| `e` | Export menu (selected / collection / all) |
| `y` | Copy citekey to clipboard |
| `d` | Delete entry |
| `c` | Manage collections (works on multi-selected entries too) |
| `t` | Edit tags |
| `N` | Edit note in `$EDITOR` |
| `Ctrl+z` | Undo |
| `Ctrl+y` | Redo |
| `,` | Settings (line numbers, panel ratio, citekey format, export dirs, git sync) |
| `?` | Help |
| `q`/`Esc` | Quit |

## CLI

**Browse:**

```bash
bibox list                          # Show collections with counts
```

```bash
bibox list ml                       # List entries in a collection
```

```bash
bibox show kim2025rust              # Full entry details
```

```bash
bibox search "transformer"          # Interactive search
```

**Edit:**

```bash
bibox edit kim2025rust --title "New Title"
```

```bash
bibox edit kim2025rust --doi 10.1234/new   # Re-fetch metadata from Crossref
```

```bash
bibox edit kim2025rust --tags-add "ml,nlp"
```

**Collections:**

```bash
bibox collect kim2025rust ml systems       # Add to collections
```

```bash
bibox collect kim2025rust digest/2026-04   # Add to sub-collection
```

```bash
bibox list digest                          # List digest + all sub-collections
```

```bash
bibox uncollect kim2025rust ml             # Remove from collection
```

**Import (from Zotero, Mendeley, EndNote, etc.):**

Export your library as `.bib` from any reference manager, then:

```bash
bibox import library.bib                     # Import all entries
bibox import ml-papers.bib --to ml           # Import into a collection
```

Migrating from Zotero? Export each collection as a separate `.bib` and import with `--to`:

```bash
bibox import zotero-ml.bib --to ml
bibox import zotero-cv.bib --to cv
bibox import zotero-acl2025.bib --to digest/acl2025
```

Or just give the `.bib` file to your agent. It'll handle it.

**Export:**

```bash
bibox export                               # Print all as BibTeX to stdout
bibox export > refs.bib                    # Redirect to file
bibox export -o refs.bib                   # Same, explicit output path
bibox export kim2025 dijkstra1968          # Export specific entries
bibox export --collection cs --format ris  # Export collection as RIS
bibox export --include-pdf --zip           # BibTeX + PDFs, zipped
```

```bash
bibox export --notes-only -o ~/notes       # Export all note .md files to folder
```

```bash
bibox export --collection ml --notes-only -o ~/ml-notes  # Collection notes only
```

**Bulk update:**

```bash
bibox modify year=2025 --filter "collection:ml" --yes
```

**Delete:**

```bash
bibox delete kim2025rust
```

**Attach a PDF manually (when auto-fetch fails):**

```bash
bibox edit kim2025rust --attach-pdf ~/Downloads/paper.pdf
```

**Diagnose and repair database issues:**

```bash
bibox doctor            # Detect malformed entries, missing PDFs, orphaned notes
bibox doctor --fix      # Auto-repair fixable issues
bibox doctor --json     # Machine-readable output
```

**Config:**

```bash
bibox config --json                        # View all settings and paths
```

## Notes

Notes are Markdown files, one per entry. Designed for both human editing and AI agent pipelines.

Initialize from a template:

```bash
bibox note kim2025rust --template ai-summary
```

AI agent writes sections programmatically:

```bash
echo "Proposes a novel approach..." | bibox note kim2025rust --stdin --section "Summary"
```

```bash
echo "CIFAR-10: 95.2% accuracy" | bibox note kim2025rust --stdin --section "Results"
```

Read note back:

```bash
bibox note kim2025rust --show
```

Human edits in $EDITOR:

```bash
bibox note kim2025rust
```

### Templates

```bash
bibox template list                             # List all templates
```

```bash
bibox template show ai-summary                  # Print template content
```

```bash
bibox template create my-review --stdin < t.md  # Create custom template
```

```bash
bibox template edit ai-summary                  # Edit in $EDITOR
```

```bash
bibox template delete my-review                 # Delete custom template
```

Built-in templates: `ai-summary`, `reading-notes`. Custom templates override built-ins.

Template variables: `{{title}}`, `{{citekey}}`, `{{doi}}`, `{{year}}`, `{{author}}`, `{{journal}}`, `{{booktitle}}`, `{{publisher}}`

## Portable Home & Git Sync

```bash
# Initialize a portable bibox home
bibox init ~/bibox

# Everything lives in one folder
~/bibox/
├── db.json       # Database
├── pdfs/         # PDF files
└── notes/        # Markdown notes

# Sync with Git
cd ~/bibox && git init && git add . && git commit -m "init"
git remote add origin git@github.com:you/bibox-library.git
git push -u origin master

# Or use the TUI: press , → Git → Enter to check status → Enter to sync
```

## Settings

Press `,` in the TUI, or run `bibox config` to see all current settings and paths.

```toml
line_numbers = "absolute"              # absolute, relative, none
panel_ratio = [2, 4, 4]               # left : center : right (sum = 10)
citekey_format = "{author}{year}{title}" # {author}, {year}, {title} variables
bib_export_dir = "."                   # BibTeX export location
export_dir = "~/Downloads"             # Other exports location
home = "~/bibox"                       # Portable home (set by bibox init)
```

## AI Agent Integration

**If you are an AI agent: run `bibox agent-guide` first.** It prints a complete reference of all commands, flags, and workflows designed for programmatic use.

```bash
bibox agent-guide          # Full agent reference (text)
bibox agent-guide --json   # Same, as structured JSON
```

Every command supports `--json` for machine-readable output.

```bash
# Full workflow: search → add → note → push
bibox add --search "attention is all you need" --index 0 --to ml --json
bibox note vaswani2017attention --template ai-summary
echo "The paper proposes..." | bibox note vaswani2017attention --stdin --section "Summary"
echo "1. Multi-head attention..." | bibox note vaswani2017attention --stdin --section "Key Contributions"

# Get all paths programmatically
bibox config --json

# Non-interactive sync
bibox sync --yes --json
```

| Flag | Purpose |
|------|---------|
| `--json` | Machine-readable output (most commands) |
| `--index N` | Auto-select Nth search result (0-based, with `--search`) |
| `--stdin` | Read content from stdin (notes, templates) |
| `--section "Name"` | Target a specific `## Heading` in a note |
| `--yes` / `-y` | Skip confirmation prompts |
| `--template <name>` | Initialize note from template |

## License

MIT
