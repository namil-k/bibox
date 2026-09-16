# zotero

Moves a Zotero library into bibox in one go: entries, nested collections, your own tags, one PDF per entry and your notes (as a `## Zotero` section of the bibox note). It reads a copy of Zotero's database, so Zotero can stay open, and it never changes anything in Zotero.

```
bibox plugin install namil-k/bibox/plugins/zotero
bibox zotero import --dry-run      # what would happen
bibox zotero import                # asks, then imports
```

Entries already in bibox (same DOI, or same title and year) are merged instead of added again, so running it twice is harmless. `--collection "Parent/Child"` imports one branch, `--all-tags` also takes the tags Zotero attached automatically, `--data-dir` points at a Zotero data directory somewhere else. Better BibTeX citation keys are kept when the add-on is installed. Python 3, no dependencies.
