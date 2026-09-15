"""Citation counts from Crossref. Cached on disk for max_age_days; missing counts are fetched in the background and pushed."""
import json
import os
import sys
import time
import urllib.parse
import urllib.request

from bibox_plugin import bibox, cache, config, fields, serve, status, window

OFFLINE = os.environ.get("BIBOX_CITATIONS_OFFLINE") == "1"
_queue = []        # DOIs waiting for a fetch
_by_doi = {}       # doi -> [bibtex_key, ...]
_fetching = False


def _fresh(doi):
    row = cache.get(doi)
    if not row:
        return None
    age = time.time() - row.get("fetched_at", 0)
    return row["count"] if age < int(config.get("max_age_days", 7)) * 86400 else None


def _text(count):
    return "★ {}".format(count)


def _fetch(doi):
    url = "https://api.crossref.org/works/" + urllib.parse.quote(doi, safe="")
    mailto = config.get("mailto") or ""
    if mailto:
        url += "?mailto=" + urllib.parse.quote(mailto)
    req = urllib.request.Request(url, headers={"User-Agent": "bibox-citations/0.1 (mailto:{})".format(mailto or "none")})
    with urllib.request.urlopen(req, timeout=15) as r:
        return int(json.load(r)["message"].get("is-referenced-by-count", 0))


def _drain():
    """Fetch queued DOIs one by one, pushing each count as it lands. Runs on a worker thread."""
    global _fetching
    total = len(_queue)
    done = 0
    while _queue:
        doi = _queue.pop(0)
        done += 1
        status.set("progress", "citations {}/{}".format(done, total))
        try:
            n = _fetch(doi)
        except Exception as e:  # noqa: BLE001 - one bad DOI must not stop the rest
            print("citations: {}: {}".format(doi, e), file=sys.stderr)
            continue
        cache.set(doi, {"count": n, "fetched_at": time.time()})
        fields.set({k: {"count": _text(n)} for k in _by_doi.get(doi, [])})
    status.set("progress", "")
    _fetching = False


def _enqueue(entries):
    global _fetching
    for e in entries:
        doi = (e.get("doi") or "").strip()
        if not doi:
            continue
        _by_doi.setdefault(doi, [])
        if e["bibtex_key"] not in _by_doi[doi]:
            _by_doi[doi].append(e["bibtex_key"])
        if _fresh(doi) is None and doi not in _queue:
            _queue.append(doi)
    if _queue and not _fetching and not OFFLINE:
        _fetching = True
        from bibox_plugin import spawn
        spawn(_drain)


def get_fields(keys, entries):
    out = {}
    for e in entries:
        doi = (e.get("doi") or "").strip()
        n = _fresh(doi) if doi else None
        if n is not None:
            out[e["bibtex_key"]] = {"count": _text(n)}
    _enqueue(entries)
    return out


def refresh(params):
    entries = params.get("entries") or []
    dois = [(e.get("doi") or "").strip() for e in entries]
    dois = [d for d in dois if d]
    if not dois:
        raise RuntimeError("no DOI on the selected entries")
    for d in dois:
        cache.set(d, {"count": 0, "fetched_at": 0})
    _enqueue(entries)
    return "refetching {} count{}".format(len(dois), "" if len(dois) == 1 else "s")


def on_written(params):
    _enqueue(params.get("entries") or [])


def on_selected(params):
    _enqueue([params["entry"]])


def cli(argv):
    """bibox citations <key...>: one count per line, blank when unknown, fetching what is missing."""
    for key in argv:
        e = bibox("show", key, "--json")
        doi = (e.get("doi") or "").strip()
        n = _fresh(doi) if doi else None
        if n is None and doi and not OFFLINE:
            try:
                n = _fetch(doi)
                cache.set(doi, {"count": n, "fetched_at": time.time()})
            except Exception as ex:  # noqa: BLE001
                print("citations: {}: {}".format(key, ex), file=sys.stderr)
        print("" if n is None else n)


if __name__ == "__main__":
    if os.environ.get("BIBOX_CLI") == "1":
        cli(sys.argv[1:])
    else:
        from bibox_plugin import on
        on("library/written")(on_written)
        on("entry/selected")(on_selected)
        serve({"refresh": refresh}, fields=get_fields)
