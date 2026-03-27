# bibox

[![Crates.io](https://img.shields.io/crates/v/bibox)](https://crates.io/crates/bibox)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

A terminal-based bibliography manager built in Rust. Add papers by PDF, DOI, ISBN, arXiv ID, or URL — metadata is fetched automatically. Manage your library through a three-panel TUI or a scriptable CLI designed for AI agent workflows.

## Features

- **Smart import** — Drop a PDF and bibox extracts the DOI, fetches metadata from Crossref, and files it automatically
- **Multiple sources** — Add entries via PDF, DOI, ISBN, arXiv ID, URL, or title search
- **Three-panel TUI** — Collections, entries, and preview (info/notes/PDF) side by side
- **Vim-style navigation** — `hjkl`, `gg`/`G`, `{n}j`, `Ctrl+d/u`, multi-select with `Space`
- **AI-agent friendly notes** — Markdown notes with `--stdin`, `--section`, and `--template` flags for programmatic access
- **Portable home** — `bibox init ~/bibox` puts everything in one folder, ready for Git sync
- **Export** — BibTeX, YAML, RIS, CSV. Include PDFs. Copy to clipboard.

## Install

**From crates.io:**

```bash
cargo install bibox
```

**From source:**

```bash
git clone https://github.com/namil-k/bibox.git
cd bibox/bibox
cargo install --path .
```

Build dependencies: OpenSSL (`brew install openssl` on macOS, `sudo apt install libssl-dev pkg-config` on Ubuntu).

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
│ h←collections  l→preview  / search  ? help  q quit   │
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
| `/` | Search (live filter) |
| `s` | Sort menu |
| `o` | Open PDF (or fetch from web) |
| `e` | Export menu |
| `y` | Copy citekey to clipboard |
| `c` | Manage collections |
| `t` | Edit tags |
| `N` | Edit note in `$EDITOR` |
| `,` | Settings |
| `?` | Help |
| `q` | Quit |

## CLI

**Browse:**

```bash
bibox list                          # Show collections
```

```bash
bibox list cs                       # List entries in a collection
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
bibox collect kim2025rust ml systems       # Add to collections
```

```bash
bibox uncollect kim2025rust ml             # Remove from collection
```

**Export:**

```bash
bibox export                               # Export all as BibTeX
```

```bash
bibox export kim2025 dijkstra1968          # Export specific entries
```

```bash
bibox export --collection cs --format ris  # Export collection as RIS
```

```bash
bibox export --include-pdf                 # Export BibTeX + PDF files
```

**Delete:**

```bash
bibox delete kim2025rust
```

## Notes (AI Agent Workflow)

Notes are stored as Markdown files, one per entry. Designed for both human editing and AI agent pipelines.

Initialize with a template:

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

Built-in templates: `ai-summary`, `reading-notes`. Custom templates go in `~/.config/bibox/templates/`.

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

# Or use the TUI: press , → Git → Enter to sync
```

## Settings

Press `,` in the TUI or edit `~/.config/bibox/config.toml`:

```toml
line_numbers = "absolute"    # absolute, relative, none
panel_ratio = [2, 4, 4]     # left : center : right
bib_export_dir = "."        # BibTeX export location
export_dir = "~/Downloads"  # Other exports location
home = "~/bibox"            # Portable home (set by bibox init)
```

## Tech Stack

- **Rust** — clap, serde, reqwest, ratatui, crossterm, arboard
- **APIs** — Crossref, Unpaywall, arXiv, OpenLibrary
- **Storage** — JSON database, flat PDF directory, Markdown notes

## License

MIT
