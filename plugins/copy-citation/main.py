from bibox_plugin import copy_to_clipboard, pick, serve

STYLES = ["APA", "IEEE", "Chicago"]


def pages(e):
    return (e.get("pages") or "").replace("--", "-")


def apa(e):
    authors = e.get("author", [])
    if len(authors) <= 1:
        who = "".join(authors)
    elif len(authors) <= 20:
        who = ", ".join(authors[:-1]) + ", & " + authors[-1]
    else:
        who = ", ".join(authors[:19]) + ", ... " + authors[-1]
    s = "{} ({}). {}.".format(who, e.get("year") or "n.d.", e.get("title") or "")
    if e.get("journal"):
        s += " " + e["journal"]
        if e.get("volume"):
            s += ", " + e["volume"]
        if e.get("number"):
            s += "(" + e["number"] + ")"
        if pages(e):
            s += ", " + pages(e)
        s += "."
    elif e.get("booktitle"):
        s += " In " + e["booktitle"] + "."
    elif e.get("publisher"):
        s += " " + e["publisher"] + "."
    if e.get("doi"):
        s += " https://doi.org/" + e["doi"]
    return s


def ieee(e):
    names = []
    for a in e.get("author", []):
        last, _, first = a.partition(",")
        initials = " ".join(p[0] + "." for p in first.replace(".", " ").split() if p)
        names.append((initials + " " + last.strip()).strip())
    if not names:
        who = ""
    elif len(names) == 1:
        who = names[0]
    else:
        who = ", ".join(names[:-1]) + ", and " + names[-1]
    s = '{}, "{},"'.format(who, e.get("title") or "")
    if e.get("journal"):
        s += " " + e["journal"]
        if e.get("volume"):
            s += ", vol. " + e["volume"]
        if e.get("number"):
            s += ", no. " + e["number"]
        if pages(e):
            s += ", pp. " + pages(e)
    elif e.get("booktitle"):
        s += " in " + e["booktitle"]
    if e.get("year"):
        s += ", {}".format(e["year"])
    s += "."
    if e.get("doi"):
        s += " doi: " + e["doi"]
    return s


def chicago(e):
    authors = e.get("author", [])
    who = " and ".join(authors) if len(authors) <= 2 else ", ".join(authors[:-1]) + ", and " + authors[-1]
    s = '{}. {}. "{}."'.format(who, e.get("year") or "n.d.", e.get("title") or "")
    if e.get("journal"):
        s += " " + e["journal"]
        if e.get("volume"):
            s += " " + e["volume"]
        if e.get("number"):
            s += " (" + e["number"] + ")"
        if pages(e):
            s += ": " + pages(e)
        s += "."
    elif e.get("booktitle"):
        s += " In " + e["booktitle"] + "."
    if e.get("doi"):
        s += " https://doi.org/" + e["doi"] + "."
    return s


FORMAT = {"APA": apa, "IEEE": ieee, "Chicago": chicago}


def copy(ctx):
    entries = ctx.get("entries") or []
    if not entries:
        return {"error": "no entry selected"}
    i = pick("Citation style", STYLES)
    if i is None:
        return {}
    style = STYLES[i]
    copy_to_clipboard("\n".join(FORMAT[style](e) for e in entries))
    return {"message": "Copied {} {} citation(s)".format(len(entries), style)}


serve({"copy": copy})
