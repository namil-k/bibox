"""A directory of library proxies and link resolvers, so a library can be picked by name (what Google Scholar's
"Library links" setting does with its private registry).

Two public sources, merged into registry.json next to this file:
- libproxy-db.org (github.com/tom5760/ezproxy-db, CC BY-SA 4.0): proxy prefixes, `$@` marks the target URL
- zotero.org/support/locate/openurl_resolvers (raw wiki export): OpenURL link resolvers, one `|Name|%%url%%|` per line

`python3 registry.py` (or `bibox proxy update`) refetches both and rewrites the snapshot. Entries are
{"name", "country", "link", "kind": "proxy" | "resolver"} where `link` is what the `links` setting takes.
"""
import json
import os
import re
import sys
import urllib.parse
import urllib.request

LIBPROXY_URL = "https://libproxy-db.org/proxies.json"
ZOTERO_URL = "https://www.zotero.org/support/locate/openurl_resolvers?do=export_raw"
SNAPSHOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "registry.json")
OPENURL_QUERY = "url_ver=Z39.88-2004&rfr_id=info:sid/bibox&rft_id=info:doi/{doi}"


def convert_libproxy(items):
    out = []
    for it in items:
        url = it.get("url") or ""
        name = (it.get("name") or "").strip()
        if not url or not name:
            continue
        link = url[:-2] if url.endswith("$@") else url.replace("$@", "{url}")
        out.append({"name": name, "country": (it.get("country") or "").strip(), "link": link, "kind": "proxy"})
    return out


def parse_zotero(text):
    """`===== Region =====` then `==== Country ====` headings; a region may list institutions directly."""
    out, region, country = [], "", ""
    for line in text.splitlines():
        m = re.match(r"^=====\s*(.+?)\s*=====$", line)
        if m:
            region, country = m.group(1), ""
            continue
        m = re.match(r"^====\s*(.+?)\s*====$", line)
        if m:
            country = m.group(1)
            continue
        m = re.match(r"^\|(.+?)\|%%(.+?)%%\|", line)
        if not m:
            continue
        base = m.group(2).strip()
        link = base + ("&" if "?" in base else "?") + OPENURL_QUERY
        out.append({"name": m.group(1).strip(), "country": country or region, "link": link, "kind": "resolver"})
    return out


def search(entries, query):
    """Every word of the query must appear in the name or the country, case-insensitively. Sorted by name."""
    words = [w for w in (query or "").lower().split() if w]
    if not words:
        return []
    hits = [e for e in entries if all(w in (e["name"] + " " + e["country"]).lower() for w in words)]
    return sorted(hits, key=lambda e: (e["name"].lower(), e["kind"]))


def host(link):
    return urllib.parse.urlsplit(link).netloc or link


def label(entry):
    """Popups cut long labels on the right, so the kind comes before the host."""
    where = ("resolver " if entry["kind"] == "resolver" else "") + host(entry["link"])
    return "{} ({})  {}".format(entry["name"], entry["country"], where) if entry["country"] else "{}  {}".format(entry["name"], where)


def load(path=SNAPSHOT):
    with open(path) as f:
        return json.load(f)


def fetch(timeout=20):
    with urllib.request.urlopen(LIBPROXY_URL, timeout=timeout) as r:
        proxies = convert_libproxy(json.load(r))
    with urllib.request.urlopen(ZOTERO_URL, timeout=timeout) as r:
        resolvers = parse_zotero(r.read().decode("utf-8"))
    return proxies + resolvers


def update(path=SNAPSHOT, timeout=20):
    """Rewrite the snapshot from the network. Returns (proxies, resolvers) counts."""
    entries = fetch(timeout)
    tmp = path + ".tmp"
    with open(tmp, "w") as f:
        json.dump(entries, f, ensure_ascii=False, indent=0)
        f.write("\n")
    os.replace(tmp, path)
    return sum(1 for e in entries if e["kind"] == "proxy"), sum(1 for e in entries if e["kind"] == "resolver")


if __name__ == "__main__":
    p, r = update()
    print("registry.json: {} proxies, {} link resolvers".format(p, r), file=sys.stderr)
