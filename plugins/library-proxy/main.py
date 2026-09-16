"""Open an entry through your library's proxy or link resolver.

CLI (`bibox library-proxy list|url|open|find|update`, BIBOX_CLI=1) and TUI commands (`library-proxy.open`, `library-proxy.choose`, `library-proxy.find`) share proxied().
`find` looks a library up by name in registry.json (see registry.py) and saves its link first through settings/set.
Settings link1..link5 hold the libraries, the lowest filled slot is the default (bibox's Settings screen shows them as five
rows, so they can be edited there too). Each is a prefix (`https://ezproxy.example.edu/login?url=`) or a template with
{url}, {url_encoded} or {doi}. Nothing is downloaded here: the browser is logged in, bibox is not.
"""
import argparse
import json
import os
import subprocess
import sys
import urllib.parse

import registry

PLACEHOLDERS = ("{url}", "{url_encoded}", "{doi}")
MANAGE = "Manage links (add, reorder, remove, find by name)"
FIND_ITEM = "+ Find a library by name"
ADD_ITEM = "+ Add a link by hand"


SLOTS = 5


def slot(i):
    return "link{}".format(i + 1)


def parse_links(table):
    """The filled slots in order, gaps skipped. `links` (the older single string) is read after them."""
    table = table or {}
    links = [str(table.get(slot(i)) or "").strip() for i in range(SLOTS)]
    links = [l for l in links if l]
    return links or str(table.get("links") or "").split()


def label(link):
    """What the pick popup shows: the link without its scheme."""
    return link.split("://", 1)[1] if "://" in link else link


def host(link):
    return urllib.parse.urlsplit(link).netloc or label(link)


def target_url(entry):
    """What `w` would open: https://doi.org/<doi>, else the entry's url, else None."""
    doi = (entry.get("doi") or "").strip()
    if doi:
        return "https://doi.org/" + doi
    return (entry.get("url") or "").strip() or None


def proxied(link, entry):
    """The URL to open. None when the entry lacks what the link needs (DOI for {doi}, DOI or URL otherwise)."""
    link = (link or "").strip()
    if not link:
        raise ValueError("no link")
    url = target_url(entry)
    doi = (entry.get("doi") or "").strip()
    if not any(p in link for p in PLACEHOLDERS):
        return link + url if url else None
    if "{doi}" in link and not doi:
        return None
    if ("{url}" in link or "{url_encoded}" in link) and not url:
        return None
    # A DOI may contain <>() and friends; encode what would break a query string, keep / and : readable.
    return (link.replace("{url}", url or "")
                .replace("{url_encoded}", urllib.parse.quote(url or "", safe=""))
                .replace("{doi}", urllib.parse.quote(doi, safe="/:;")))


def open_in_browser(url):
    """Hand the URL to the desktop. $BIBOX_PROXY_OPENER replaces open/xdg-open (tests, unusual setups)."""
    opener = os.environ.get("BIBOX_PROXY_OPENER")
    if opener:
        cmd = [opener, url]
    elif sys.platform == "darwin":
        cmd = ["open", url]
    else:
        cmd = ["xdg-open", url]
    subprocess.Popen(cmd, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def open_entries(link, entries):
    """Returns (opened, skipped)."""
    opened = skipped = 0
    for e in entries:
        url = proxied(link, e)
        if url is None:
            skipped += 1
            continue
        open_in_browser(url)
        opened += 1
    return opened, skipped


def summary(link, opened, skipped):
    text = "opened {} via {}".format(opened, host(link))
    if skipped:
        text += ", skipped {} (no DOI or URL)".format(skipped)
    return text


def find_link(links, via):
    """`via` is a 1-based number or a case-insensitive piece of a link; exactly one must match."""
    if via.isdigit() and 1 <= int(via) <= len(links):
        return links[int(via) - 1]
    hits = [l for l in links if via.lower() in l.lower()]
    if len(hits) != 1:
        raise ValueError("no link matches {!r}".format(via) if not hits else "{!r} matches {} links, be more specific".format(via, len(hits)))
    return hits[0]


# ── TUI commands ────────────────────────────────────────────────────────────

def _selection(params):
    return params.get("entries") or ([params["entry"]] if params.get("entry") else [])


def _open_via(link, params):
    entries = _selection(params)
    if not entries:
        return "no entry selected"
    return summary(link, *open_entries(link, entries))


def _links():
    from bibox_plugin import config
    return parse_links(config)


def _save(links):
    """Write the list back into link1..link5, touching only the slots that change."""
    from bibox_plugin import config, settings
    for i in range(SLOTS):
        want = links[i] if i < len(links) else ""
        if str(config.get(slot(i)) or "").strip() != want:
            settings.set(slot(i), want)


def open_cmd(params):
    """`u`: the first link. With no links yet, the manager, so the first press sets things up."""
    links = _links()
    if not links:
        return links_cmd(params)
    return _open_via(links[0], params)


def choose_cmd(params):
    """`v`: pick a link (first is the default), or go to the manager."""
    from bibox_plugin import window

    links = _links()
    if not links:
        return links_cmd(params)
    items = [label(l) + (" (default)" if i == 0 else "") for i, l in enumerate(links)] + [MANAGE]
    index = window.pick("Open through", items)
    if index is None:
        return "cancelled"
    if index == len(links):
        return links_cmd(params)
    return _open_via(links[index], params)


def find_cmd(params):
    """Look a library up by name, put its link first, save. The Google Scholar "Library links" flow."""
    from bibox_plugin import window

    query = window.prompt("Library name (e.g. Seoul National, Paris-Saclay)")
    if not query or not query.strip():
        return "cancelled"
    hits = registry.search(registry.load(), query)
    if not hits:
        return "no library matches {!r}. The plugin README says how to find the link by hand".format(query.strip())
    index = window.pick("Use", [registry.label(e) for e in hits[:40]])
    if index is None:
        return "cancelled"
    chosen = hits[index]
    rest = [l for l in _links() if l != chosen["link"]]
    if len(rest) >= SLOTS:
        return "all {} slots are used; remove one first (v, Manage links)".format(SLOTS)
    try:
        _save([chosen["link"]] + rest)
    except RuntimeError as e:
        return "could not save: {}".format(e)
    return "{} is now your default library ({})".format(chosen["name"], host(chosen["link"]))


def _add_by_hand(window):
    text = window.prompt("Link: a prefix such as https://ezproxy.example.edu/login?url= or a template with {url}, {url_encoded} or {doi}")
    text = (text or "").strip()
    if not text:
        return None
    if " " in text or "://" not in text:
        window.message("not a link: {!r} (no spaces, must start with http:// or https://)".format(text), "warn")
        return None
    return text


def links_cmd(params):
    """The manager: list, reorder, remove, add by hand or from the directory. Loops until Escape."""
    from bibox_plugin import window

    while True:
        links = _links()
        items = ["{}. {}{}".format(i + 1, host(l), " (default)" if i == 0 else "") for i, l in enumerate(links)] + [FIND_ITEM, ADD_ITEM]
        index = window.pick("Library links, first is the default", items)
        if index is None:
            break
        try:
            if index == len(links):
                find_cmd(params)
            elif index == len(links) + 1:
                if len(links) >= SLOTS:
                    window.message("all {} slots are used; remove one first".format(SLOTS), "warn")
                    continue
                text = _add_by_hand(window)
                if text:
                    _save([text] + [l for l in links if l != text])
            else:
                link = links[index]
                action = window.pick(host(link), ["Make it the default", "Move up", "Move down", "Remove"])
                rest = [l for l in links if l != link]
                if action == 0:
                    _save([link] + rest)
                elif action == 1 and index > 0:
                    _save(rest[:index - 1] + [link] + rest[index - 1:])
                elif action == 2 and index < len(links) - 1:
                    _save(rest[:index + 1] + [link] + rest[index + 1:])
                elif action == 3:
                    _save(rest)
        except RuntimeError as e:
            return "could not save: {}".format(e)
    links = _links()
    return "links: " + ", ".join(host(l) for l in links) if links else "no links yet"


# ── CLI ─────────────────────────────────────────────────────────────────────

def _bibox(*args):
    """Run the bibox CLI (kept local so CLI mode does not import bibox_plugin, which redirects stdout)."""
    cmd = [os.environ.get("BIBOX_BIN", "bibox")] + list(args)
    p = subprocess.run(cmd, stdin=subprocess.DEVNULL, capture_output=True, text=True)
    if p.returncode != 0:
        raise RuntimeError(p.stderr.strip() or "bibox {} failed".format(" ".join(args)))
    return json.loads(p.stdout)


def links_from_config():
    """CLI mode gets no settings, only $BIBOX_CONFIG_DIR: read [plugins.library-proxy] from config.toml there."""
    d = os.environ.get("BIBOX_CONFIG_DIR")
    if not d:
        return []
    try:
        import tomllib
        with open(os.path.join(d, "config.toml"), "rb") as f:
            return parse_links(tomllib.load(f).get("plugins", {}).get("library-proxy", {}))
    except (ImportError, OSError, ValueError):
        return []


def cli(argv):
    ap = argparse.ArgumentParser(prog="bibox library-proxy", description="Open entries through your library proxy or link resolver.")
    sub = ap.add_subparsers(dest="cmd")
    sub.add_parser("list", help="print the configured links, first is the default")
    sub.add_parser("find", help="look a library up by name in the bundled directory").add_argument("query", nargs="+")
    sub.add_parser("update", help="refetch the library directory (libproxy-db.org and Zotero's resolver list)")
    for name, help_ in (("url", "print the proxied URL of an entry"), ("open", "open the entry in the browser through the proxy")):
        p = sub.add_parser(name, help=help_)
        p.add_argument("key", help="citation key")
        p.add_argument("--via", metavar="N_OR_HOST", help="which configured link: its number or a piece of its host (default: the first)")
        p.add_argument("--link", help="use this prefix or template instead of the configured ones")
    args = ap.parse_args(argv)
    if args.cmd not in ("list", "url", "open", "find", "update"):
        ap.print_help()
        return 2
    if args.cmd == "find":
        hits = registry.search(registry.load(), " ".join(args.query))
        if not hits:
            print("bibox library-proxy: no library matches {!r}. The plugin README says how to find the link by hand".format(" ".join(args.query)), file=sys.stderr)
            return 1
        for i, e in enumerate(hits, 1):
            print("{}  {}  {}".format(i, registry.label(e).split("  ")[0], e["link"]))
        return 0
    if args.cmd == "update":
        try:
            p, r = registry.update()
        except OSError as e:
            print("bibox library-proxy: could not fetch the directory: {}".format(e), file=sys.stderr)
            return 1
        print("registry.json: {} proxies, {} link resolvers".format(p, r))
        return 0
    links = links_from_config()
    if args.cmd == "list":
        for i, l in enumerate(links, 1):
            print("{}  {}".format(i, l))
        return 0
    try:
        if args.link:
            link = args.link.strip()
        elif not links:
            raise ValueError("no links. Pass --link or set [plugins.library-proxy] link1 in config.toml (see the plugin README)")
        elif args.via:
            link = find_link(links, args.via)
        else:
            link = links[0]
        entry = _bibox("show", args.key, "--json")
    except (RuntimeError, ValueError) as e:
        print("bibox library-proxy: {}".format(e), file=sys.stderr)
        return 1
    url = proxied(link, entry)
    if url is None:
        print("bibox library-proxy: {} has no DOI or URL to put in the link".format(args.key), file=sys.stderr)
        return 1
    if args.cmd == "open":
        open_in_browser(url)
    print(url)
    return 0


if __name__ == "__main__":
    if os.environ.get("BIBOX_CLI") == "1":
        sys.exit(cli(sys.argv[1:]))
    from bibox_plugin import serve

    serve({"open": open_cmd, "choose": choose_cmd, "find": find_cmd, "links": links_cmd})
