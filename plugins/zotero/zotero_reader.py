"""Read a Zotero data directory and map its items to the bibox import JSON shape.

Pure: standard library only, no bibox_plugin import (that module redirects stdout when imported).
Zotero 7 schema: items/itemData/itemDataValues/fields, itemCreators/creators, collections/collectionItems,
itemAttachments, itemNotes, itemTags/tags, deletedItems. Better BibTeX keys come from better-bibtex.sqlite.
"""
import glob
import html
import os
import re
import shutil
import sqlite3
from collections import namedtuple
from html.parser import HTMLParser


class ZoteroError(Exception):
    pass


Mapped = namedtuple("Mapped", "entry notes_md missing_pdf")

MONTHS = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"]
TYPE_MAP = {"journalArticle": "article", "book": "book", "conferencePaper": "inproceedings", "bookSection": "inproceedings"}
SKIP_TYPES = {"attachment", "note", "annotation"}


# ── locating the data ────────────────────────────────────────────────────────

def read_prefs(profiles_dir):
    """`extensions.zotero.dataDir` and `baseAttachmentPath` from the first prefs.js under `profiles_dir`."""
    out = {}
    for prefs in sorted(glob.glob(os.path.join(profiles_dir, "*", "prefs.js"))):
        try:
            with open(prefs, encoding="utf-8", errors="replace") as f:
                text = f.read()
        except OSError:
            continue
        for key in ("dataDir", "baseAttachmentPath"):
            m = re.search(r'user_pref\("extensions\.zotero\.%s",\s*"((?:[^"\\]|\\.)*)"\)' % key, text)
            if m:
                out[key] = m.group(1).encode().decode("unicode_escape")
        if out:
            break
    return out


def profiles_dir(home):
    """Zotero profile directory for this OS (macOS, Linux; Windows is a best effort)."""
    candidates = [
        os.path.join(home, "Library", "Application Support", "Zotero", "Profiles"),
        os.path.join(home, ".zotero", "zotero"),
        os.path.join(os.environ.get("APPDATA", ""), "Zotero", "Zotero", "Profiles"),
    ]
    for c in candidates:
        if os.path.isdir(c):
            return c
    return candidates[0]


def find_data_dir(explicit=None, home=None):
    """Argument > BIBOX_ZOTERO_DATA_DIR > prefs.js dataDir > ~/Zotero. Must contain zotero.sqlite."""
    home = home or os.path.expanduser("~")
    tried = []
    for d in (explicit, os.environ.get("BIBOX_ZOTERO_DATA_DIR"), read_prefs(profiles_dir(home)).get("dataDir"), os.path.join(home, "Zotero")):
        if not d:
            continue
        d = os.path.expanduser(d)
        tried.append(d)
        if os.path.isfile(os.path.join(d, "zotero.sqlite")):
            return d
        if explicit and d == os.path.expanduser(explicit):
            break
    raise ZoteroError("no zotero.sqlite in {}; pass --data-dir".format(", ".join(tried) or "the usual places"))


def base_attachment_path(home=None):
    """Zotero's linked attachment base directory: BIBOX_ZOTERO_BASE_PATH, else prefs.js."""
    if os.environ.get("BIBOX_ZOTERO_BASE_PATH"):
        return os.environ["BIBOX_ZOTERO_BASE_PATH"]
    home = home or os.path.expanduser("~")
    return read_prefs(profiles_dir(home)).get("baseAttachmentPath")


def snapshot_db(data_dir, tmp_dir):
    """Copy zotero.sqlite (and its journal files) so a running Zotero's lock does not matter. Returns the copy's path."""
    os.makedirs(tmp_dir, exist_ok=True)
    src = os.path.join(data_dir, "zotero.sqlite")
    dst = os.path.join(tmp_dir, "zotero.sqlite")
    shutil.copy2(src, dst)
    for suffix in ("-journal", "-wal", "-shm"):
        if os.path.exists(src + suffix):
            shutil.copy2(src + suffix, dst + suffix)
    return dst


def citation_keys(data_dir):
    """Better BibTeX citekeys by itemID, or {} when the add-on is not installed."""
    path = os.path.join(data_dir, "better-bibtex.sqlite")
    if not os.path.isfile(path):
        return {}
    try:
        db = sqlite3.connect("file:{}?mode=ro".format(path), uri=True)
        rows = db.execute("SELECT itemID, citationKey FROM citationkey").fetchall()
        db.close()
    except sqlite3.Error:
        return {}
    return {int(i): k for i, k in rows if k}


# ── small conversions ────────────────────────────────────────────────────────

def date_parts(value):
    """Zotero stores "YYYY-MM-DD original text". (year, month abbreviation or None)."""
    if not value:
        return None, None
    head = value.split(" ", 1)[0]
    m = re.match(r"^(\d{4})(?:-(\d{2}))?", head)
    if not m:
        m = re.search(r"(\d{4})", value)
        return (int(m.group(1)), None) if m else (None, None)
    year = int(m.group(1))
    month = int(m.group(2)) if m.group(2) else 0
    return year, (MONTHS[month - 1] if 1 <= month <= 12 else None)


def resolve_attachment(raw_path, attachment_key, data_dir, base_attachment_path):
    """Absolute path of an attachment, or None when it cannot be resolved (not: when the file is missing)."""
    if not raw_path:
        return None
    if raw_path.startswith("storage:"):
        return os.path.join(data_dir, "storage", attachment_key, raw_path[len("storage:"):])
    if raw_path.startswith("attachments:"):
        if not base_attachment_path:
            return None
        return os.path.join(base_attachment_path, *raw_path[len("attachments:"):].split("/"))
    return raw_path


class _Markdown(HTMLParser):
    """Enough HTML to markdown for Zotero notes: paragraphs, headings, lists, bold/italic, links, code."""

    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.out = []
        self.list_stack = []
        self.href = None

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        if tag in ("p", "div", "br", "tr"):
            self.out.append("\n\n" if tag != "br" else "\n")
        elif tag in ("h1", "h2", "h3", "h4"):
            self.out.append("\n\n" + "#" * int(tag[1]) + " ")
        elif tag in ("b", "strong"):
            self.out.append("**")
        elif tag in ("i", "em"):
            self.out.append("*")
        elif tag == "code":
            self.out.append("`")
        elif tag in ("ul", "ol"):
            self.list_stack.append([tag, 0])
            self.out.append("\n\n")
        elif tag == "li":
            if self.list_stack:
                kind, n = self.list_stack[-1]
                self.list_stack[-1][1] = n + 1
                self.out.append("\n" + ("- " if kind == "ul" else "{}. ".format(n + 1)))
        elif tag == "a":
            self.href = a.get("href")
            self.out.append("[")

    def handle_endtag(self, tag):
        if tag in ("b", "strong"):
            self.out.append("**")
        elif tag in ("i", "em"):
            self.out.append("*")
        elif tag == "code":
            self.out.append("`")
        elif tag in ("p", "div", "h1", "h2", "h3", "h4"):
            self.out.append("\n\n")
        elif tag in ("ul", "ol"):
            if self.list_stack:
                self.list_stack.pop()
            self.out.append("\n\n")
        elif tag == "a":
            self.out.append("]({})".format(self.href or ""))
            self.href = None

    def handle_data(self, data):
        self.out.append(re.sub(r"\s+", " ", data))


def html_to_markdown(text):
    """Zotero note HTML to markdown. The outer wrapper div (any) is stripped; blank runs collapse."""
    if not text:
        return ""
    p = _Markdown()
    p.feed(text)
    p.close()
    md = html.unescape("".join(p.out))
    lines = [ln.rstrip() for ln in md.split("\n")]
    out = []
    for ln in lines:
        if ln.strip() == "" and (not out or out[-1] == ""):
            continue
        out.append(ln.strip() if ln.strip() == "" else ln.lstrip() if not ln.startswith(("- ", "1")) else ln)
    return "\n".join(out).strip()


# ── the library ──────────────────────────────────────────────────────────────

class ZoteroLibrary:
    def __init__(self, sqlite_path, data_dir, base_attachment_path=None):
        self.data_dir = data_dir
        self.base_attachment_path = base_attachment_path
        self.db = sqlite3.connect("file:{}?mode=ro".format(sqlite_path), uri=True)
        self.db.row_factory = sqlite3.Row

    def collection_paths(self):
        rows = self.db.execute("SELECT collectionID, collectionName, parentCollectionID FROM collections").fetchall()
        by_id = {r["collectionID"]: (r["collectionName"], r["parentCollectionID"]) for r in rows}

        def path(cid, depth=0):
            name, parent = by_id[cid]
            if parent is None or parent not in by_id or depth > 50:
                return name
            return path(parent, depth + 1) + "/" + name

        return {cid: path(cid) for cid in by_id}

    def items(self):
        """Regular items (no attachments, notes, annotations, trash) with everything the mapper needs."""
        q = self.db.execute
        deleted = {r[0] for r in q("SELECT itemID FROM deletedItems")}
        types = {r["itemTypeID"]: r["typeName"] for r in q("SELECT itemTypeID, typeName FROM itemTypes")}
        paths = self.collection_paths()
        items = []
        for r in q("SELECT itemID, itemTypeID, key FROM items ORDER BY itemID"):
            if r["itemID"] in deleted or types.get(r["itemTypeID"]) in SKIP_TYPES:
                continue
            items.append({"itemID": r["itemID"], "key": r["key"], "type": types.get(r["itemTypeID"], "misc"), "fields": {}, "creators": [], "collections": [], "tags": [], "attachments": [], "notes": []})
        by_id = {i["itemID"]: i for i in items}
        if not by_id:
            return items
        for r in q("SELECT d.itemID, f.fieldName, v.value FROM itemData d JOIN fields f ON f.fieldID = d.fieldID JOIN itemDataValues v ON v.valueID = d.valueID"):
            if r["itemID"] in by_id:
                by_id[r["itemID"]]["fields"][r["fieldName"]] = r["value"]
        for r in q("SELECT ic.itemID, c.lastName, c.firstName, ct.creatorType FROM itemCreators ic JOIN creators c ON c.creatorID = ic.creatorID JOIN creatorTypes ct ON ct.creatorTypeID = ic.creatorTypeID ORDER BY ic.itemID, ic.orderIndex"):
            if r["itemID"] in by_id:
                by_id[r["itemID"]]["creators"].append((r["lastName"] or "", r["firstName"] or "", r["creatorType"]))
        for r in q("SELECT collectionID, itemID FROM collectionItems ORDER BY collectionID"):
            if r["itemID"] in by_id and r["collectionID"] in paths:
                by_id[r["itemID"]]["collections"].append(paths[r["collectionID"]])
        for r in q("SELECT it.itemID, t.name, it.type FROM itemTags it JOIN tags t ON t.tagID = it.tagID ORDER BY it.type, t.name"):
            if r["itemID"] in by_id:
                by_id[r["itemID"]]["tags"].append((r["name"], r["type"]))
        for r in q("SELECT a.itemID, a.parentItemID, a.linkMode, a.contentType, a.path, i.key FROM itemAttachments a JOIN items i ON i.itemID = a.itemID ORDER BY a.itemID"):
            parent = r["parentItemID"]
            if parent in by_id and r["itemID"] not in deleted:
                resolved = resolve_attachment(r["path"], r["key"], self.data_dir, self.base_attachment_path)
                by_id[parent]["attachments"].append({"path": resolved, "exists": bool(resolved and os.path.isfile(resolved)), "contentType": r["contentType"] or "", "linkMode": r["linkMode"]})
        for r in q("SELECT n.itemID, n.parentItemID, n.note FROM itemNotes n ORDER BY n.itemID"):
            if r["parentItemID"] in by_id and r["itemID"] not in deleted and r["note"]:
                by_id[r["parentItemID"]]["notes"].append(r["note"])
        return items


def _creator_name(last, first):
    last, first = last.strip(), first.strip()
    if last and first:
        return "{}, {}".format(last, first)
    return last or first


def map_item(item, citekeys, all_tags=False):
    """One Zotero item to the bibox import shape. `citekeys` maps itemID to a Better BibTeX key."""
    f = item["fields"]
    e = {}  # type: dict
    e["entry_type"] = TYPE_MAP.get(item["type"], "misc")
    key = citekeys.get(item["itemID"])
    if key:
        e["bibtex_key"] = key
    if f.get("title"):
        e["title"] = f["title"]
    authors = [_creator_name(l, n) for l, n, t in item["creators"] if t == "author"]
    e["author"] = [a for a in authors if a]
    editors = [_creator_name(l, n) for l, n, t in item["creators"] if t == "editor"]
    if editors:
        e["editor"] = " and ".join(a for a in editors if a)
    year, month = date_parts(f.get("date"))
    if year:
        e["year"] = year
    if month:
        e["month"] = month
    simple = {"publicationTitle": "journal", "volume": "volume", "issue": "number", "pages": "pages", "publisher": "publisher", "edition": "edition", "ISBN": "isbn", "DOI": "doi", "url": "url", "abstractNote": "abstract"}
    for src, dst in simple.items():
        if f.get(src):
            e[dst] = f[src].strip()
    booktitle = f.get("proceedingsTitle") or f.get("bookTitle")
    if booktitle:
        e["booktitle"] = booktitle
    if item["type"] == "preprint" and f.get("repository"):
        e["howpublished"] = f["repository"]
    extra = f.get("extra") or ""
    arxiv = next((ln.strip() for ln in extra.splitlines() if ln.lower().startswith("arxiv:")), None)
    if arxiv:
        e["note"] = arxiv
    e["tags"] = [name for name, kind in item["tags"] if all_tags or kind == 0]
    e["collections"] = list(item["collections"])
    missing = None
    for a in item["attachments"]:
        if a["contentType"] != "application/pdf" or not a["path"]:
            continue
        if a["exists"]:
            e["file"] = a["path"]
            missing = None
            break
        if missing is None:
            missing = a["path"]
    # bibox는 article/book/inproceedings에 제목·저자·연도를 요구한다. 빠진 항목은 버리지 않고 misc로 넣는다
    if e["entry_type"] != "misc" and (not e.get("title") or not e["author"] or "year" not in e):
        e["entry_type"] = "misc"
    notes = [html_to_markdown(n) for n in item["notes"]]
    return Mapped(e, [n for n in notes if n], missing)
