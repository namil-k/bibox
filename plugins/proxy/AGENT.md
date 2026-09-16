# proxy

Opens an entry in the browser through the user's library proxy or link resolver, so a paywalled paper opens with the library's subscription. bibox itself downloads nothing: the browser holds the library login, bibox does not. The `links` setting under `[plugins.proxy]` holds one link per line, the first is the default (people with more than one institution, or an expired account, keep several). Each link is either a prefix put before the entry's URL (`https://ezproxy.example.edu/login?url=`) or a template with `{url}`, `{url_encoded}` or `{doi}`. The URL is `https://doi.org/<doi>` when the entry has a DOI, otherwise the entry's `url`.

## From the shell

    bibox proxy find <words...>                         # libraries whose name or country contains every word, with their links
    bibox proxy update                                  # refetch the directory (libproxy-db.org and Zotero's resolver list) into registry.json
    bibox proxy list                                   # the configured links, numbered, first is the default
    bibox proxy url <key> [--via N_OR_HOST] [--link L]  # print the proxied URL, open nothing
    bibox proxy open <key> [--via N_OR_HOST] [--link L] # open it in the default browser, then print it

`--via` picks one of the configured links by its number or by a piece of its host (`--via khu`); `--link` uses a link that is not configured. Exit code 1 with a one-line reason on stderr when nothing is configured, `--via` matches no link or several, the entry does not exist, or the entry has neither DOI nor URL (or no DOI when the link uses `{doi}`). Use `url` to hand a link to the user or to check that their settings are right; do not fetch the proxied URL yourself, it answers with a login page.

## In the TUI

`u` opens the selected entries through the first link; `v` asks which link first (with a single link it opens right away); `proxy.find` (no default key) asks for a library name, shows the matches and saves the picked link as the new first line of `links`. All three are in the right-click menu. The status popup says `opened 2 via ezproxy.example.edu, skipped 1 (no DOI or URL)`. With no links it says where to set them.
