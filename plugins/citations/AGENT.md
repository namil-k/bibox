# citations

Citation counts from Crossref (`is-referenced-by-count`). Entries without a DOI have no count.

## From the shell

    bibox citations <key...>

Prints one count per line in the order given; a blank line means unknown (no DOI or Crossref has no record). Missing counts are fetched on the spot, so the first call for a key can take a second.

## In the TUI

Each entry row shows `★ n` on the right of the first line. The status bar shows `citations 12/167` while counts load. `<C-r>` (or the right-click menu) refetches the selected entries.

## Settings

`mailto` puts you in Crossref's polite pool (faster, optional). `max_age_days` (default 7) is how long a count is trusted before it is fetched again.
