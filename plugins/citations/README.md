# citations

Shows how many times each entry has been cited, using Crossref's `is-referenced-by-count`. Every entry row gets a `★ n` on the right of its first line and the Info tab a `Cited by:` line; entries without a DOI stay blank. Counts are cached in the plugin directory for `max_age_days` (default 7) and fetched in the background, with `citations k/n` in the status bar while they load. `<C-r>` or the right-click menu refetches the selected entries.

Install with `bibox plugin install <path-to-this-directory>` (or from the bibox repository). Optional settings under `[plugins.citations]` in `config.toml`: `mailto = "you@example.org"` joins Crossref's polite pool (faster), `max_age_days = 7`. From the shell, `bibox citations <key...>` prints one count per line for scripts and agents.
