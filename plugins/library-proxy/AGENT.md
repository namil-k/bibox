# library-proxy

Opens an entry in the browser through the user's library proxy or link resolver, so a paywalled paper opens with the library's subscription. bibox itself downloads nothing: the browser holds the library login, bibox does not. Settings `link1` to `link5` under `[plugins.library-proxy]` hold the libraries in order of preference; the lowest filled slot is the default (people with more than one institution, or an expired account, keep several). Each link is either a prefix put before the entry's URL (`https://ezproxy.example.edu/login?url=`) or a template with `{url}`, `{url_encoded}` or `{doi}`. The URL is `https://doi.org/<doi>` when the entry has a DOI, otherwise the entry's `url`.

## From the shell

    bibox library-proxy find <words...>                         # libraries whose name or country contains every word, with their links
    bibox library-proxy update                                  # refetch the directory (libproxy-db.org and Zotero's resolver list) into registry.json
    bibox library-proxy list                                   # the configured links, numbered, first is the default
    bibox library-proxy url <key> [--via N_OR_HOST] [--link L]  # print the proxied URL, open nothing
    bibox library-proxy open <key> [--via N_OR_HOST] [--link L] # open it in the default browser, then print it

`--via` picks one of the configured links by its number or by a piece of its host (`--via khu`); `--link` uses a link that is not configured. Exit code 1 with a one-line reason on stderr when nothing is configured, `--via` matches no link or several, the entry does not exist, or the entry has neither DOI nor URL (or no DOI when the link uses `{doi}`). Use `url` to hand a link to the user or to check that their settings are right; do not fetch the proxied URL yourself, it answers with a login page.

## In the TUI

`u` opens the selected entries through the first link (with no links yet it opens the manager instead); `v` asks which link, with "Manage links" as the last item; `library-proxy.links` is that manager (add by hand, find by name, make default, move, remove; loops until Escape); `library-proxy.find` asks for a library name, shows the matches and saves the pick as the first link. All are in the right-click menu. The five slots are ordinary string settings, so the Settings screen (`,` then Plugins) edits them too. The status popup says `opened 2 via ezproxy.example.edu, skipped 1 (no DOI or URL)`. With no links it says where to set them.
