# proxy

Opens an entry in the browser through the user's library proxy or link resolver, so a paywalled paper opens with the library's subscription. bibox itself downloads nothing: the browser holds the library login, bibox does not. The `link` setting under `[plugins.proxy]` is either a prefix put before the entry's URL (`https://ezproxy.example.edu/login?url=`) or a template with `{url}`, `{url_encoded}` or `{doi}`. The URL is `https://doi.org/<doi>` when the entry has a DOI, otherwise the entry's `url`.

## From the shell

    bibox proxy url <key> [--link PREFIX_OR_TEMPLATE]     # print the proxied URL, open nothing
    bibox proxy open <key> [--link PREFIX_OR_TEMPLATE]    # open it in the default browser, then print it

Without `--link` the value comes from `[plugins.proxy] link` in config.toml. Exit code 1 with a one-line reason on stderr when there is no link, the entry does not exist, or the entry has neither DOI nor URL (or no DOI when the template uses `{doi}`). Use `url` to hand a link to the user or to check that their `link` setting is right; do not fetch the proxied URL yourself, it answers with a login page.

## In the TUI

`u` (or the right-click menu) opens the selected entries; the status popup says `opened 2, skipped 1 (no DOI or URL)`. With an empty `link` it says where to set it.
