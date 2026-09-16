# library-proxy

Open a paper through your university library, from bibox. Press `u` on an entry and the browser opens the paper through your library's proxy or link resolver, so the publisher sees your subscription. Several libraries can be registered (a second institution, a public library, an account that may have expired): `u` uses the first, `v` lets you pick, and the same popup manages the list. This is what Google Scholar's "FindIt@..." links do: they only build the link, the login and the download happen in your browser. bibox does not download anything through the proxy; unattended downloads through a library proxy get accounts suspended.

## Setup

Install, then find your library by name, the way Google Scholar's "Library links" setting works:

    bibox plugin install namil-k/bibox/plugins/library-proxy

In bibox, press `u` (or `v`) with no library set up yet: a popup offers "Find a library by name" and "Add a link by hand". Type part of the name (`seoul national`, `paris-saclay`, `max planck`) and pick the match; the link is saved as your default. Later, `v` > "Manage links" (or `library-proxy.links` in the right-click menu) lists your libraries, and picking one offers "Make it the default", "Move up", "Move down" and "Remove". `bibox library-proxy find seoul national` prints the same matches from the shell. The directory is a snapshot of two public lists, 1,513 proxies from [libproxy-db.org](https://libproxy-db.org/) (CC BY-SA 4.0) and 507 link resolvers from [Zotero's directory](https://www.zotero.org/support/locate/openurl_resolvers); `bibox library-proxy update` refetches both. A library that is missing can be added to those lists, or written by hand:

    [plugins.library-proxy]
    links = "https://ezproxy.example.edu/login?url="

More than one library: separate the links with spaces (or one per line in a `"""` string), the first is the one `u` uses. The Settings screen (`,` then Plugins) shows and edits the same value.

    [plugins.library-proxy]
    links = "https://ezproxy.example.edu/login?url= https://openlink.khu.ac.kr/link.n2s?url="

Each link is a prefix that goes before the paper's URL (`https://doi.org/<doi>`, or the entry's URL when it has no DOI), or a template with `{url}`, `{url_encoded}` or `{doi}`:

| Your library uses | link |
|---|---|
| EZproxy (most universities) | `https://ezproxy.example.edu/login?url=` |
| A "prefix" gateway such as Kyung Hee's OpenLink | `https://openlink.khu.ac.kr/link.n2s?url=` |
| OpenAthens | `https://go.openathens.net/redirector/example.edu?url={url_encoded}` |
| An OpenURL link resolver (the one behind Google Scholar's "FindIt@...") | `https://resolver.example.edu/openurl?sid=bibox&id=doi:{doi}` |

**Finding your value by hand** (when the directory does not have your library). Any of these works:

- Your library's "proxy bookmarklet" (search the library site for "bookmarklet" or "off-campus access"): the text between `location.href='` and `'+` is the prefix.
- Off campus, click any database link on the library site and look at the address bar: everything before the database's own URL is the prefix.
- In Google Scholar, set your library under Settings > Library links, search a paper and copy the "FindIt@..." link. The part before `?` is your resolver; use it with `?sid=bibox&id=doi:{doi}`.

Check it with `bibox library-proxy url <key>`, which prints the URL without opening anything, and `bibox library-proxy list`, which shows the links in order.

## Use

- `u` in the entry list opens through the first link; `v` asks which link, with "Manage links" as the last item. Both are in the right-click menu and work on a multi-selection.
- `bibox library-proxy url <key>` prints the proxied URL, `bibox library-proxy open <key>` opens it. `--via 2` or `--via khu` picks a configured link by number or by a piece of its host; `--link` uses one that is not configured.
- To make the built-in `w` go through the proxy too, add to `keymap.toml`:

      [normal.entries]
      prepend_keymap = [{ on = "w", run = "library-proxy.open" }]

Entries without a DOI or URL are skipped and counted in the message. Set `BIBOX_PROXY_OPENER` to a program that takes a URL to replace `open`/`xdg-open`.

## Tests

    python3 -m unittest test_proxy -v
