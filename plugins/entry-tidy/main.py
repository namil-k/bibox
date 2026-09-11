import re

from bibox_plugin import serve

SMALL = {"a", "an", "the", "and", "or", "of", "in", "on", "for", "to", "with", "at", "by", "from", "vs"}


def normalize_name(name):
    """'kim, jinho' -> 'Kim, J.'   'Jinho Kim' -> 'Kim, J.'   'Kim, J.' stays."""
    name = " ".join(name.split())
    if not name:
        return name
    if "," in name:
        last, _, first = name.partition(",")
    else:
        parts = name.split(" ")
        if len(parts) > 1 and re.fullmatch(r"[A-Z]\.?", parts[-1]):
            # "PARK S." : a trailing initial means the surname came first
            last, first = parts[0], " ".join(parts[1:])
        elif len(parts) == 1:
            return parts[0].title() if not parts[0].istitle() else parts[0]
        else:
            last, first = parts[-1], " ".join(parts[:-1])
    last = last.strip()
    if last.islower() or last.isupper():
        last = last.title()
    initials = " ".join(p[0].upper() + "." for p in re.split(r"[\s.\-]+", first.strip()) if p)
    return "{}, {}".format(last, initials) if initials else last


def tidy_title(title):
    title = " ".join(title.split())
    if title.isupper() and len(title) > 3:
        words = title.lower().split(" ")
        title = " ".join(w if (i and w in SMALL) else w.capitalize() for i, w in enumerate(words))
    elif title and title[0].islower():
        title = title[0].upper() + title[1:]
    return title


def tidy(entry):
    e = dict(entry)
    e["author"] = [normalize_name(a) for a in e.get("author", [])]
    if e.get("title"):
        e["title"] = tidy_title(e["title"])
    for k, v in list(e.items()):
        if isinstance(v, str):
            e[k] = v.strip()
    return e


def run(ctx):
    entries = ctx.get("entries") or ([ctx["entry"]] if ctx.get("entry") else [])
    if not entries:
        return {"error": "no entry selected"}
    tidied = [tidy(e) for e in entries]
    changed = sum(1 for a, b in zip(entries, tidied) if a != b)
    return {"apply": tidied, "message": "Tidied {} of {} entries".format(changed, len(entries))}


serve({"tidy": run})
