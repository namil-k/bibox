"""Open an entry through your library's proxy or link resolver.

CLI (`bibox proxy url|open <key>`, BIBOX_CLI=1) and TUI command (`proxy.open`) share proxied().
The `link` setting is a prefix (`https://ezproxy.example.edu/login?url=`) or a template with
{url}, {url_encoded} or {doi}. Nothing is downloaded here: the browser is logged in, bibox is not.
"""
import argparse
import json
import os
import subprocess
import sys
import urllib.parse

PLACEHOLDERS = ("{url}", "{url_encoded}", "{doi}")
SETUP_HINT = "Set proxy.link in Settings (,): your library's proxy prefix. See the proxy plugin README"


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


def summary(opened, skipped):
    text = "opened {}".format(opened)
    if skipped:
        text += ", skipped {} (no DOI or URL)".format(skipped)
    return text


# ── TUI command ─────────────────────────────────────────────────────────────

def open_cmd(params):
    from bibox_plugin import config

    link = (config.get("link") or "").strip()
    if not link:
        return SETUP_HINT
    entries = params.get("entries") or ([params["entry"]] if params.get("entry") else [])
    if not entries:
        return "no entry selected"
    return summary(*open_entries(link, entries))


# ── CLI ─────────────────────────────────────────────────────────────────────

def _bibox(*args):
    """Run the bibox CLI (kept local so CLI mode does not import bibox_plugin, which redirects stdout)."""
    cmd = [os.environ.get("BIBOX_BIN", "bibox")] + list(args)
    p = subprocess.run(cmd, stdin=subprocess.DEVNULL, capture_output=True, text=True)
    if p.returncode != 0:
        raise RuntimeError(p.stderr.strip() or "bibox {} failed".format(" ".join(args)))
    return json.loads(p.stdout)


def link_from_config():
    """CLI mode gets no settings, only $BIBOX_CONFIG_DIR: read [plugins.proxy] link from config.toml there."""
    d = os.environ.get("BIBOX_CONFIG_DIR")
    if not d:
        return ""
    try:
        import tomllib
        with open(os.path.join(d, "config.toml"), "rb") as f:
            return str(tomllib.load(f).get("plugins", {}).get("proxy", {}).get("link") or "")
    except (ImportError, OSError, ValueError):
        return ""


def cli(argv):
    ap = argparse.ArgumentParser(prog="bibox proxy", description="Open entries through your library proxy or link resolver.")
    sub = ap.add_subparsers(dest="cmd")
    for name, help_ in (("url", "print the proxied URL of an entry"), ("open", "open the entry in the browser through the proxy")):
        p = sub.add_parser(name, help=help_)
        p.add_argument("key", help="citation key")
        p.add_argument("--link", help="proxy prefix or template (default: [plugins.proxy] link in config.toml)")
    args = ap.parse_args(argv)
    if args.cmd not in ("url", "open"):
        ap.print_help()
        return 2
    link = (args.link or link_from_config()).strip()
    if not link:
        print("bibox proxy: no link. Pass --link or set [plugins.proxy] link in config.toml (see the plugin README)", file=sys.stderr)
        return 1
    try:
        entry = _bibox("show", args.key, "--json")
    except RuntimeError as e:
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

    serve({"open": open_cmd})
