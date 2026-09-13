# bibox

<p align="center">
  <img src="assets/bbox_bass.png" alt="bibox" width="320">
</p>

[![Crates.io](https://img.shields.io/crates/v/bibox)](https://crates.io/crates/bibox)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**Using AI agents (Claude, OpenClaw, Hermes) to search and summarize papers?**
**Let them manage your bibliography and notes too.**
**You just browse the TUI. bibox makes it work.**

Just give your agent the GitHub link. It'll figure out the rest.

For humans: browse and edit in the TUI. For agents: manage entries and notes through the CLI.

> **Best for:** researchers who use AI agents for literature review and want papers, notes, and BibTeX in one Git-syncable folder.
>
> **Not for:** GUI, PDF annotation, or Word integration. Use Zotero instead.

**Sources:** Crossref, Open Library, arXiv, Unpaywall
**Formats:** BibTeX, YAML, RIS, CSV, Markdown
**Import:** `.bib`, `.ris` from Zotero, Mendeley, EndNote

## Features

- **Smart import** - Drop a PDF and bibox extracts the DOI, fetches metadata from Crossref, and files it automatically
- **Multiple sources** - Add entries via PDF, DOI, ISBN, arXiv ID, URL, or title search
- **Three-panel TUI** - Collections, entries, and preview (info / notes / PDF) side by side
- **Mouse support** - Click to select entries/collections, scroll wheel, right-click context menu, click preview tabs
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
- **Plugins** - Any executable becomes a bibox command, a context-menu item or a save hook. Declared by `plugin.toml`, talks JSON over stdin/stdout, can open bibox's own popups. Built-in plugins ship inside the binary and can be removed like any other; git sync is one.

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
| `n`/`p`, `+`/`-`, `0`, `H`/`L` | PDF tab: next/previous page, zoom in/out, fit width, pan (see below) |
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
| `Y` | Copy a formatted citation (APA, IEEE or Chicago) to clipboard |
| `d` | Delete entry |
| `c` | Manage collections (works on multi-selected entries too) |
| `t` | Edit tags |
| `N` | Edit note in `$EDITOR` |
| `Ctrl+z` | Undo |
| `Ctrl+y` | Redo |
| `,` | Settings: General, Appearance, Export, Plugins. `Tab` switches columns, `h`/`l` change a value, `/` searches every setting |
| `?` | Help |
| `q` | Quit |
| `Esc` | Clear selection (or quit if nothing selected) |
| `gs` / `gt` | Sync / status of the git-backed portable home (git-sync built-in plugin) |
| **Mouse** | |
| Left click | Select entry/collection, switch panel focus, click preview tabs |
| Right click | Context menu (open, export, delete, etc.) |
| Scroll wheel | Navigate entries/collections, scroll preview |

Press `` ` ``, `~` or `F1` for a searchable list of every key that works in the focused panel. `/` filters it, and the filter matches descriptions too, so typing `clipboard` finds `y`.

The bar at the bottom carries panel navigation and the few actions used many times a day; the help screen covers the rest. Its keys are read from the active keymap, so they follow a remap. Turn it off with `status_bar = false` or from the settings screen, and the row goes back to the panels.

### PDF tab

The PDF tab is drawn by the built-in `pdf-view` plugin, which needs poppler (`brew install poppler`, or `apt install poppler-utils`). In a terminal that can show pictures (kitty, Ghostty, WezTerm, iTerm2, foot; anything else falls back to half-block characters) each page is rendered to fit the panel width; elsewhere, and inside tmux or screen, the tab shows the page's text. The status line at the bottom says `page 3/14  100%` (or `text`).

| Key | Action |
|-----|--------|
| `j`/`k` | Scroll three rows; at the bottom `j` turns to the next page, at the top `k` to the bottom of the previous one |
| `Ctrl+d`/`u` | Half a panel |
| `gg`/`G` | First / last page |
| `n`/`p` | Next / previous page |
| `+` (`=`) / `-` | Zoom in / out by 25% (25% to 400%, `max_zoom` on the plugin page) |
| `0` | Fit the page to the panel width again |
| `H`/`L` | Pan left / right when the page is wider than the panel |

Remove the plugin (`,` then Plugins) and the tab disappears; `bibox plugin install pdf-view` brings it back.

### Customizing keybindings

Every key is remappable through `keymap.toml`, next to `config.toml`:

- macOS: `~/Library/Application Support/bibox/keymap.toml`
- Linux: `~/.config/bibox/keymap.toml`

```toml
[normal.entries]
prepend_keymap = [
  { on = "<C-n>",    run = "entry_down" },
  { on = ["g", "b"], run = "entry_bottom", desc = "Jump to the last entry" },
  { on = "d",        run = "noop" },              # disable delete
]

[normal.preview]
prepend_keymap = [
  { on = "<C-n>", run = "preview_scroll_down" },
]
```

**Layers.** The Normal mode splits by which panel has focus: `[normal.collections]`, `[normal.entries]` and `[normal.preview]`. The layers are independent, so a key that should work in every panel has to be written in every layer. That is also why the help screen shows only what is live in the panel you are in.

**Merging.** Each layer's effective list is `prepend_keymap` + the defaults + `append_keymap`, and lookup takes the first match. So `prepend_keymap` overrides a default, `append_keymap` adds to it, and `clear_defaults = true` drops the defaults for that layer only.

**Fields.** `on` takes one key or a list for a sequence. `run` takes one action or a list to run in order. `desc` is optional and replaces the description shown in help. `noop` disables a key.

**Key notation.** `<C-x>`, `<A-x>`, `<S-x>`, `<Esc>`, `<Space>`, `<Tab>`, `<Enter>`, `<Backspace>`, `<Left>`, `<Right>`, `<Up>`, `<Down>`, `<F1>` through `<F12>`. Anything else is the character itself. Shift is not written for letters: `G`, not `<S-g>`.

**Two limits.** Digits cannot be bound, because they are the count prefix that makes `5j` work; the exception is a leading `0`, which is not a count and stays bindable. And a count now reaches every action in a sequence, so `5<Space>` selects one entry and moves down five.

**Actions.**

| Layer | Actions |
|-------|---------|
| Any | `quit` `cancel` `undo` `redo` `next_preview_tab` `search` `copy_citekey` `open_pdf` `open_web` `fetch_metadata` `export_menu` `delete` `help` `edit_note` `sort_menu` `collections` `tags` `attach_pdf` `settings` `noop` |
| `normal.entries` | `entry_down` `entry_up` `entry_top` `entry_bottom` `entry_screen_top` `entry_screen_middle` `entry_screen_bottom` `entry_half_page_down` `entry_half_page_up` `toggle_select` `select_all` `focus_collections` `focus_preview` |
| `normal.collections` | `collection_down` `collection_up` `collection_top` `collection_bottom` `collection_half_page_down` `collection_half_page_up` `focus_entries` |
| `normal.preview` | `preview_scroll_down` `preview_scroll_up` `preview_top` `preview_bottom` `preview_half_page_down` `preview_half_page_up` `next_tab` `prev_tab` `prev_tab_or_focus_entries` `focus_entries` |

Action names say what they move, not which panel has focus, so binding `entry_down` inside `[normal.preview]` is allowed and does what it says.

Plugin commands are bound by `<plugin>.<command>`, for example `run = "git-sync.sync"`. Their default keys sit below the built-in ones, so a built-in key always wins; the help screen lists live plugin commands under Plugins.

**When it breaks.** A bad `keymap.toml` never stops bibox. Every problem is reported before the TUI opens and the default keymap is used for that run, so you are never locked out by your own config. `bibox doctor` reports the same problems without opening the TUI, and `bibox doctor --json` gives them to an editor.


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
bibox show kim2025rust --cite apa   # One formatted citation (apa, ieee, chicago)
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

Export your library as `.bib` or `.ris` from any reference manager, then:

```bash
bibox import library.bib                     # Import from BibTeX
bibox import library.ris                     # Import from RIS
bibox import ml-papers.bib --to ml           # Import into a collection
```

Migrating from Zotero? Export each collection as a separate `.bib` or `.ris` and import with `--to`:

```bash
bibox import zotero-ml.bib --to ml
bibox import zotero-cv.ris --to cv
bibox import zotero-acl2025.bib --to digest/acl2025
```

Or just give the file to your agent. It'll handle it.

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

# Sync with Git (db + notes only, PDFs stored separately)
cd ~/bibox && git init && git add . && git commit -m "init"

# The git-sync plugin (built in, installed by default) commits db.json and notes on every write
# once this folder is a git repository. Press g t for status and g s to pull --rebase and push.
# Settings: , then Plugins > git-sync, or [plugins.git-sync] include_pdfs = true / push_on_write = true in config.toml.

# Store PDFs in iCloud, Google Drive, Dropbox, etc.
# Add to ~/.config/bibox/config.toml:
pdf_dir = "~/Library/Mobile Documents/com~apple~CloudDocs/bibox-pdfs"  # iCloud
# pdf_dir = "~/Google Drive/bibox-pdfs"                                # Google Drive
# pdf_dir = "~/Dropbox/bibox-pdfs"                                     # Dropbox
```

## Plugins

A plugin is a directory with a `plugin.toml` and a program in any language. bibox starts the program the first time it is needed, keeps it running while the TUI is open, and talks to it one JSON line at a time.

```
~/Library/Application Support/bibox/plugins/   (Linux: ~/.config/bibox/plugins/)
  summarize/
    plugin.toml
    main.py
```

**Install.** `bibox plugin install namil-k/bibox/plugins/summarize` clones from GitHub, `bibox plugin install ./my-plugin` symlinks a local directory, `bibox plugin list` shows what is installed and where it came from (`built-in`, `local`, `git`, `dir`), `bibox plugin remove <name>` deletes it. Turning a plugin off is removing it; a built-in comes back with `bibox plugin install <name>` and needs no network. Installing from a repository shows where the code comes from and what it runs, then asks. Nobody has reviewed code that is not in a registry.

**In the TUI.** `,` then Plugins lists what is installed and which built-in plugins are not. `Enter` opens a plugin page: description, an `Installed` toggle (`h`/`l`; removing an external plugin asks first) and the settings the plugin declares. `Install from…` at the end of the list takes `owner/repo`, a git URL or a local path and shows the same confirmation as the CLI.

**Write one.** `bibox plugin new my-plugin` creates a working skeleton with a Python helper. The whole contract for other languages is four lines:

```
bibox  → plugin   {"type":"command","id":"copy","trigger":"key","context":{"entry":{...},"entries":[...],"config":{},"paths":{...}}}
plugin → bibox    {"ui":"pick","title":"Citation style","items":["APA","IEEE"]}
bibox  → plugin   {"index":0}
plugin → bibox    {"message":"Copied APA citation"}
```

A line with `"ui"` asks bibox to show something (`pick`, `prompt`, `confirm`, `progress`) and gets one line back. A line without `"ui"` ends the command: `message` shows text, `apply` replaces entries (undoable, all or nothing), `refresh` re-reads the database after you changed it through `$BIBOX_BIN`, `error` reports a failure. Flush stdout after every line.

**Declare.** `plugin.toml`:

```toml
api = 1
name = "summarize"                   # must equal the directory name
run = "python3 main.py"              # split on spaces, no shell; cwd is the plugin directory

[[commands]]
id = "summarize"
desc = "Summarize the PDF into the note"
key = "S"                            # default key; users override it in keymap.toml as run = "summarize.summarize"
menu = true                          # right-click menu

[[hooks]]
on = "after_write"                   # before_add | after_write | after_note_save
run = "summarize"

[[tabs]]                             # optional: a tab in the preview panel next to Info and Note
title = "PDF"                        # 1 to 12 characters
run = "render"                       # a command id; bibox calls it with trigger = "tab"

[[settings]]                         # optional: shown on the plugin page and written to [plugins.summarize] in config.toml
key = "model"
type = "choice"                      # bool | int | string | choice
choices = ["claude-opus-5", "claude-sonnet-5"]
default = "claude-opus-5"
desc = "Claude model for the summary"

[cli]                                # optional: `bibox summarize ...` runs this with your stdio
run = "python3 cli.py"
```

`before_add` runs before an entry is saved and may return `apply` to change it; if the plugin fails the entry is added unchanged. `after_write` and `after_note_save` run in the background after the save and cannot open popups. Plugin settings live in `config.toml` under `[plugins.<name>]` and arrive as `context.config`. Declared settings get a typed row on the plugin page and `bibox doctor` warns about values of the wrong type and keys that match no declaration (with a spelling suggestion). bibox does not fill in defaults: read `context.config` with a fallback as before.

A `[[tabs]]` entry adds a tab to the preview panel. When the tab is visible bibox calls its command in the background with `trigger = "tab"` and `tab: {"page": 3, "width_px": 840, "images": true}`; the command answers `{"tab": {"image": "/path/page.png", "pages": 14}}` or, when `images` is false, `{"tab": {"lines": ["..."], "pages": 14}}`. bibox owns page, zoom, scrolling and the cache; the plugin only renders one page at one width. It cannot open popups. The built-in `pdf-view` is written this way.

**Environment.** Every plugin process gets `BIBOX_BIN`, `BIBOX_CONFIG_DIR`, `BIBOX_DB_PATH`, `BIBOX_NOTES_DIR`, `BIBOX_PDF_DIR`, `BIBOX_HOME` (when set) and `BIBOX_PLUGIN_DIR`. To change the library, call `$BIBOX_BIN` (`modify`, `note --stdin`, `add --json`) and return `refresh`, or return `apply` for a few entries. The plugin's stderr goes to `plugins/<name>/stderr.log`, truncated on every start.

**When it breaks.** A broken plugin never stops bibox. Manifest problems are shown before the TUI opens and by `bibox doctor`. A plugin that exits, hangs (press Esc) or prints something that is not JSON is reported in the status line and restarted on the next call. Default keys that collide with built-in keys are dropped with a warning; bind them yourself in `keymap.toml`.

**Built-in plugins** live inside the bibox binary and appear in `bibox plugin list` as `built-in`: `git-sync` (commits db.json and notes on every write when the portable home is a git repository; `g s` syncs, `g t` shows status) and `pdf-view` (the PDF tab; needs poppler). Remove one like any plugin; `bibox plugin install git-sync` puts it back. **Example plugin** (`plugins/` in this repository): `summarize` (`S`, PDF to the note's Summary section with Claude; needs `pip install anthropic` and `ANTHROPIC_API_KEY`).

## Settings

Press `,` in the TUI: sections on the left (General, Appearance, Export, Plugins), values on the right. `Tab` moves between the columns, `j`/`k` move, `h`/`l` change a value, `Enter` opens a directory picker or a text prompt, `/` searches every setting including plugin settings (`git-sync/push`, or a word from a description such as `commit`). Changes are saved as you make them; `Esc` closes. `bibox config` prints the same settings and paths. Plugin settings go under `[plugins.<name>]`, for example `[plugins.git-sync] include_pdfs = true`, and appear on the plugin's page in the Plugins section.

```toml
line_numbers = "absolute"              # absolute, relative, none
panel_ratio = [2, 4, 4]               # left : center : right (sum = 10)
natural_scroll = false                 # true for macOS-style natural scrolling
status_bar = true                      # hint bar at the bottom; false reclaims that row
images = "auto"                        # auto, off, kitty, iterm2, sixel, halfblocks; auto never asks inside tmux/screen
theme = "terminal"                     # terminal, dark, light, or a file name from themes/
citekey_format = "{author}{year}{title}" # {author}, {year}, {title} variables
bib_export_dir = "~/Downloads"         # BibTeX export location
export_dir = "~/Downloads"             # Other exports location
home = "~/bibox"                       # Portable home (set by bibox init)
pdf_dir = "~/iCloud/bibox-pdfs"        # Separate PDF storage (iCloud, Google Drive, etc.)
```

`images` decides how plugin tabs such as the PDF tab draw pictures. `auto` asks the terminal once at start (never inside tmux or screen, where the tab shows text). If a terminal does not answer and keys go missing afterwards, set `images = "off"` or name the protocol. Changing it takes effect on the next start.

`theme` picks the colors. `terminal` (the default) uses the terminal's own palette. `dark` and `light` are built in (VS Code's Dark Modern and Light Modern). Any other name is a file `themes/<name>.json` next to `config.toml`, in VS Code's color theme format: copy a theme file from a VS Code extension (`extensions/<publisher>.<theme>/themes/*.json`) and pick it in Settings. bibox reads these keys and ignores the rest; missing keys keep the terminal color. When `editor.background` is present the whole screen is painted with it, so delete that line to keep a transparent terminal background.

| bibox uses it for | VS Code key |
|---|---|
| text and background | `editor.foreground`, `editor.background` |
| focused border, active tab, cursor row text | `focusBorder` |
| titles, prompts, citekeys | `pickerGroup.foreground` |
| descriptions, hints, dividers | `descriptionForeground` |
| selected row | `list.activeSelectionBackground`, `list.activeSelectionForeground` |
| row highlight in an unfocused panel, code in notes | `list.inactiveSelectionBackground`, `list.inactiveSelectionForeground` |
| success marks | `terminal.ansiGreen` |
| warnings, errors | `editorWarning.foreground`, `editorError.foreground` |

Colors are sent as true color, so a terminal without 24-bit color support will approximate them.

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

**Writing a plugin as an agent.** Run `bibox plugin new <name>`, edit `main.py` (the helper's `serve`, `pick`, `prompt`, `confirm`, `progress`, `bibox` and `copy_to_clipboard` are documented at the top of `bibox_plugin.py`), then test it without the TUI: `printf '<request json>\n' | python3 main.py`. Failures land in `plugins/<name>/stderr.log`. When your plugin calls `bibox` from inside a hook, the helper sets `BIBOX_IN_HOOK=1` so that call does not fire hooks again.

## License

MIT
