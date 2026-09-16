# zotero

Imports a Zotero library into bibox: entries with their metadata, collections (nested, as `Parent/Child`), the tags the user added by hand, the first PDF attachment of each item (copied, not moved) and child notes (converted to markdown into a `## Zotero` section of the entry's note). Reads a copy of `zotero.sqlite`, so Zotero may stay open. Better BibTeX citation keys are used when `better-bibtex.sqlite` exists; otherwise bibox generates keys.

## From the shell

    bibox zotero import [--data-dir DIR] [--dry-run] [--all-tags] [--collection "Parent/Child"] [--yes]

Prints a summary (entries, collections, PDFs found and missing, notes) and asks before writing; `--yes` skips the question, `--dry-run` writes nothing. Re-running is safe: entries already in bibox (same DOI, or same title and year) are merged, not duplicated, and their `## Zotero` note section is rewritten from Zotero. `--data-dir` is needed when Zotero's preferences are not under the current HOME; `BIBOX_ZOTERO_DATA_DIR` and `BIBOX_ZOTERO_BASE_PATH` (the linked attachment base directory) override the same two settings. Exit code 0 on success, 1 when the library cannot be found or bibox refused.

## In the TUI

The `zotero.import` command (bind a key in keymap.toml, or run it from the command list) shows the same summary in a confirmation popup, then refreshes the list. Settings under `[plugins.zotero]`: `data_dir`, `all_tags`.

## What is not imported

Attachments that are not PDFs, standalone notes and attachments (no parent item), items in the trash, and tags Zotero added automatically unless `--all-tags`. Missing PDF files are reported and skipped; the entry is still imported.
