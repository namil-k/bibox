"""Open an entry through your library's proxy or link resolver.

CLI (`bibox proxy list|url|open|find|update`, BIBOX_CLI=1) and TUI commands (`proxy.open`, `proxy.choose`, `proxy.find`) share proxied().
`find` looks a library up by name in registry.json (see registry.py) and saves its link first through settings/set.
The `links` setting holds one link per line, the first is the default: a prefix (`https://ezproxy.example.edu/login?url=`)
or a template with {url}, {url_encoded} or {doi}. Nothing is downloaded here: the browser is logged in, bibox is not.
"""
import argparse
import json
import os
import subprocess
import sys
import urllib.parse

import registry

PLACEHOLDERS = ("{url}", "{url_encoded}", "{doi}")
SETUP_HINT = "Set proxy.links in Settings (,): your library's proxy prefix, one per line. See the proxy plugin README"


def parse_links(text):
    """One link per line (or space separated), first is the default. Links never contain whitespace."""
    return (text or "").split()


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


def open_cmd(params):
    """`u`: the first link."""
    from bibox_plugin import config

    links = parse_links(config.get("links"))
    if not links:
        return SETUP_HINT
    return _open_via(links[0], params)


def choose_cmd(params):
    """`v`: pick a link first; with a single link there is nothing to pick."""
    from bibox_plugin import config, window

    links = parse_links(config.get("links"))
    if not links:
        return SETUP_HINT
    if len(links) == 1:
        return _open_via(links[0], params)
    index = window.pick("Open through", [label(l) for l in links])
    if index is None:
        return "cancelled"
    return _open_via(links[index], params)


def find_cmd(params):
    """Look a library up by name, put its link first, save. The Google Scholar "Library links" flow."""
    from bibox_plugin import config, settings, window

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
    links = [chosen["link"]] + [l for l in parse_links(config.get("links")) if l != chosen["link"]]
    try:
        settings.set("links", "\n".join(links))
    except RuntimeError as e:
        return "could not save: {}".format(e)
    return "{} is now your default library ({})".format(chosen["name"], host(chosen["link"]))


# ── CLI ─────────────────────────────────────────────────────────────────────

def _bibox(*args):
    """Run the bibox CLI (kept local so CLI mode does not import bibox_plugin, which redirects stdout)."""
    cmd = [os.environ.get("BIBOX_BIN", "bibox")] + list(args)
    p = subprocess.run(cmd, stdin=subprocess.DEVNULL, capture_output=True, text=True)
    if p.returncode != 0:
        raise RuntimeError(p.stderr.strip() or "bibox {} failed".format(" ".join(args)))
    return json.loads(p.stdout)


def links_from_config():
    """CLI mode gets no settings, only $BIBOX_CONFIG_DIR: read [plugins.proxy] links from config.toml there."""
    d = os.environ.get("BIBOX_CONFIG_DIR")
    if not d:
        return []
    try:
        import tomllib
        with open(os.path.join(d, "config.toml"), "rb") as f:
            return parse_links(tomllib.load(f).get("plugins", {}).get("proxy", {}).get("links"))
    except (ImportError, OSError, ValueError):
        return []


def cli(argv):
    ap = argparse.ArgumentParser(prog="bibox proxy", description="Open entries through your library proxy or link resolver.")
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
            print("bibox proxy: no library matches {!r}. The plugin README says how to find the link by hand".format(" ".join(args.query)), file=sys.stderr)
            return 1
        for i, e in enumerate(hits, 1):
            print("{}  {}  {}".format(i, registry.label(e).split("  ")[0], e["link"]))
        return 0
    if args.cmd == "update":
        try:
            p, r = registry.update()
        except OSError as e:
            print("bibox proxy: could not fetch the directory: {}".format(e), file=sys.stderr)
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
            raise ValueError("no links. Pass --link or set [plugins.proxy] links in config.toml (see the plugin README)")
        elif args.via:
            link = find_link(links, args.via)
        else:
            link = links[0]
        entry = _bibox("show", args.key, "--json")
    except (RuntimeError, ValueError) as e:
        print("bibox proxy: {}".format(e), file=sys.stderr)
        return 1
    url = proxied(link, entry)
    if url is None:
        print("bibox proxy: {} has no DOI or URL to put in the link".format(args.key), file=sys.stderr)
        return 1
    if args.cmd == "open":
        open_in_browser(url)
    print(url)
    return 0


if __name__ == "__main__":
    if os.environ.get("BIBOX_CLI") == "1":
        sys.exit(cli(sys.argv[1:]))
    from bibox_plugin import serve

    serve({"open": open_cmd, "choose": choose_cmd, "find": find_cmd})
