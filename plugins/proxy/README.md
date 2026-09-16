# proxy

Open a paper through your university library, from bibox. Press `u` on an entry and the browser opens the paper through your library's proxy or link resolver, so the publisher sees your subscription. This is what Google Scholar's "FindIt@..." links do: they only build the link, the login and the download happen in your browser. bibox does not download anything through the proxy; unattended downloads through a library proxy get accounts suspended.

## Setup

Install, then put your library's link in Settings (`,` then Plugins) or in `config.toml`:

    bibox plugin install namil-k/bibox/plugins/proxy

    [plugins.proxy]
    link = "https://ezproxy.example.edu/login?url="

`link` is a prefix that goes before the paper's URL (`https://doi.org/<doi>`, or the entry's URL when it has no DOI), or a template with `{url}`, `{url_encoded}` or `{doi}`:

| Your library uses | `link` |
|---|---|
| EZproxy (most universities) | `https://ezproxy.example.edu/login?url=` |
| A "prefix" gateway such as Kyung Hee's OpenLink | `https://openlink.khu.ac.kr/link.n2s?url=` |
| OpenAthens | `https://go.openathens.net/redirector/example.edu?url={url_encoded}` |
| An OpenURL link resolver (the one behind Google Scholar's "FindIt@...") | `https://resolver.example.edu/openurl?sid=bibox&id=doi:{doi}` |

**Finding your value.** Any of these works:

- Your library's "proxy bookmarklet" (search the library site for "bookmarklet" or "off-campus access"): the text between `location.href='` and `'+` is the prefix.
- Off campus, click any database link on the library site and look at the address bar: everything before the database's own URL is the prefix.
- In Google Scholar, set your library under Settings > Library links, search a paper and copy the "FindIt@..." link. The part before `?` is your resolver; use it with `?sid=bibox&id=doi:{doi}`.

Check it with `bibox proxy url <key>`, which prints the URL without opening anything.

## Use

- `u` in the entry list, or right-click > "Open the entry through your library proxy or link resolver". Works on a multi-selection.
- `bibox proxy url <key>` prints the proxied URL, `bibox proxy open <key>` opens it. `--link` overrides the setting for one call.
- To make the built-in `w` go through the proxy too, add to `keymap.toml`:

      [normal.entries]
      prepend_keymap = [{ on = "w", run = "proxy.open" }]

Entries without a DOI or URL are skipped and counted in the message. Set `BIBOX_PROXY_OPENER` to a program that takes a URL to replace `open`/`xdg-open`.

## Tests

    python3 -m unittest test_proxy -v
