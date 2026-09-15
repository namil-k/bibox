"""Tests for zotero_reader and main. `make_fixture` builds a small Zotero 7 style database from scratch.

Run: python3 -m unittest discover -s plugins/zotero -p 'test_*.py'
"""
import os
import sqlite3
import tempfile
import unittest

import zotero_reader as zr

NOTE_HTML = '<div class="zotero-note znv1"><p>Comment: <b>Draft</b> v2 &amp; more</p><ul><li>one</li><li>two</li></ul></div>'


def make_fixture(root, bbt=None):
    """A Zotero data directory under `root`: zotero.sqlite, storage/, a linked PDF dir and a base attachment dir.

    Items: 1 journalArticle (2 authors, editor, DOI, tags manual+automatic, collection Parent/Child, linked PDF, child note),
    2 preprint (arXiv repository, extra, storage PDF that does not exist), 3 conferencePaper (attachments: PDF under the base
    path), 8 trashed article, 9 standalone attachment, 10 standalone note. Returns (data_dir, base_attachment_path).
    """
    data_dir = os.path.join(root, "Zotero")
    os.makedirs(os.path.join(data_dir, "storage"), exist_ok=True)
    linked = os.path.join(root, "linked")
    base = os.path.join(root, "base", "sub")
    os.makedirs(linked, exist_ok=True)
    os.makedirs(base, exist_ok=True)
    with open(os.path.join(linked, "deep.pdf"), "wb") as f:
        f.write(b"%PDF-1.4\n")
    with open(os.path.join(base, "livebot.pdf"), "wb") as f:
        f.write(b"%PDF-1.4\n")

    db = sqlite3.connect(os.path.join(data_dir, "zotero.sqlite"))
    c = db.cursor()
    c.executescript(
        """
        CREATE TABLE itemTypes (itemTypeID INTEGER PRIMARY KEY, typeName TEXT);
        CREATE TABLE items (itemID INTEGER PRIMARY KEY, itemTypeID INT NOT NULL, dateAdded TIMESTAMP, dateModified TIMESTAMP, key TEXT NOT NULL);
        CREATE TABLE fields (fieldID INTEGER PRIMARY KEY, fieldName TEXT);
        CREATE TABLE itemDataValues (valueID INTEGER PRIMARY KEY, value UNIQUE);
        CREATE TABLE itemData (itemID INT, fieldID INT, valueID, PRIMARY KEY (itemID, fieldID));
        CREATE TABLE creatorTypes (creatorTypeID INTEGER PRIMARY KEY, creatorType TEXT);
        CREATE TABLE creators (creatorID INTEGER PRIMARY KEY, firstName TEXT, lastName TEXT, fieldMode INT);
        CREATE TABLE itemCreators (itemID INT NOT NULL, creatorID INT NOT NULL, creatorTypeID INT NOT NULL DEFAULT 1, orderIndex INT NOT NULL DEFAULT 0);
        CREATE TABLE collections (collectionID INTEGER PRIMARY KEY, collectionName TEXT NOT NULL, parentCollectionID INT DEFAULT NULL);
        CREATE TABLE collectionItems (collectionID INT NOT NULL, itemID INT NOT NULL, orderIndex INT NOT NULL DEFAULT 0);
        CREATE TABLE itemAttachments (itemID INTEGER PRIMARY KEY, parentItemID INT, linkMode INT, contentType TEXT, path TEXT);
        CREATE TABLE itemNotes (itemID INTEGER PRIMARY KEY, parentItemID INT, note TEXT, title TEXT);
        CREATE TABLE tags (tagID INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE);
        CREATE TABLE itemTags (itemID INT NOT NULL, tagID INT NOT NULL, type INT NOT NULL);
        CREATE TABLE deletedItems (itemID INTEGER PRIMARY KEY, dateDeleted DEFAULT CURRENT_TIMESTAMP);
        """
    )
    c.executemany("INSERT INTO itemTypes VALUES (?, ?)", [(1, "journalArticle"), (2, "preprint"), (3, "conferencePaper"), (4, "attachment"), (5, "note"), (6, "book")])
    c.executemany("INSERT INTO creatorTypes VALUES (?, ?)", [(1, "author"), (2, "editor")])
    field_names = ["title", "date", "DOI", "url", "abstractNote", "pages", "volume", "issue", "publicationTitle", "proceedingsTitle", "repository", "extra", "publisher", "bookTitle"]
    fields = {name: i + 1 for i, name in enumerate(field_names)}
    c.executemany("INSERT INTO fields VALUES (?, ?)", [(i, n) for n, i in fields.items()])
    items = [
        (1, 1, "AAAA0001"), (2, 2, "PREP0002"), (3, 3, "CONF0003"), (4, 4, "ATT00004"), (5, 5, "NOTE0005"), (6, 4, "ATT00006"),
        (7, 4, "ATT00007"), (8, 1, "TRSH0008"), (9, 4, "ATT00009"), (10, 5, "NOTE0010"),
    ]
    c.executemany("INSERT INTO items (itemID, itemTypeID, key) VALUES (?, ?, ?)", items)
    values = {}

    def put(item_id, field, value):
        if value not in values:
            values[value] = len(values) + 1
            c.execute("INSERT INTO itemDataValues VALUES (?, ?)", (values[value], value))
        c.execute("INSERT INTO itemData VALUES (?, ?, ?)", (item_id, fields[field], values[value]))

    put(1, "title", "Deep Learning")
    put(1, "date", "2015-05-27 2015-05-27")
    put(1, "DOI", "10.1038/nature14539")
    put(1, "issue", "7553")
    put(1, "volume", "521")
    put(1, "pages", "436-444")
    put(1, "publicationTitle", "Nature")
    put(1, "abstractNote", "Abs")
    put(2, "title", "Attention Is All You Need")
    put(2, "date", "2017")
    put(2, "repository", "arXiv")
    put(2, "extra", "arXiv: 1706.03762\ntype: article")
    put(2, "url", "https://arxiv.org/abs/1706.03762")
    put(3, "title", "LiveBot")
    put(3, "date", "2019-07-17 2019-07-17")
    put(3, "proceedingsTitle", "AAAI")
    put(3, "DOI", "10.1609/aaai.v33i01.33016810")
    put(8, "title", "Trashed")
    put(8, "date", "2020")
    c.executemany("INSERT INTO creators VALUES (?, ?, ?, 0)", [(1, "Yann", "LeCun"), (2, "Yoshua", "Bengio"), (3, "Ed", "Editor"), (4, "Ashish", "Vaswani"), (5, "Shuming", "Ma")])
    c.executemany("INSERT INTO itemCreators VALUES (?, ?, ?, ?)", [(1, 2, 1, 1), (1, 1, 1, 0), (1, 3, 2, 0), (2, 4, 1, 0), (3, 5, 1, 0), (8, 1, 1, 0)])
    c.executemany("INSERT INTO collections VALUES (?, ?, ?)", [(10, "Parent", None), (11, "Child", 10), (12, "Empty", None)])
    c.executemany("INSERT INTO collectionItems VALUES (?, ?, 0)", [(11, 1), (10, 3)])
    c.executemany("INSERT INTO itemAttachments VALUES (?, ?, ?, ?, ?)", [
        (4, 1, 2, "application/pdf", os.path.join(linked, "deep.pdf")),
        (6, 2, 0, "application/pdf", "storage:vaswani.pdf"),
        (7, 3, 2, "application/pdf", "attachments:sub/livebot.pdf"),
        (9, None, 0, "application/pdf", "storage:alone.pdf"),
    ])
    c.executemany("INSERT INTO itemNotes VALUES (?, ?, ?, ?)", [(5, 1, NOTE_HTML, "Comment"), (10, None, "<p>standalone</p>", "s")])
    c.executemany("INSERT INTO tags VALUES (?, ?)", [(1, "deep"), (2, "Computer Science - Machine Learning")])
    c.executemany("INSERT INTO itemTags VALUES (?, ?, ?)", [(1, 1, 0), (1, 2, 1)])
    c.execute("INSERT INTO deletedItems (itemID) VALUES (8)")
    db.commit()
    db.close()
    if bbt:
        b = sqlite3.connect(os.path.join(data_dir, "better-bibtex.sqlite"))
        b.execute("CREATE TABLE citationkey (itemID INT, itemKey TEXT, libraryID INT, citationKey TEXT, pinned INT)")
        b.executemany("INSERT INTO citationkey VALUES (?, ?, 1, ?, 0)", [(i, "", k) for i, k in bbt.items()])
        b.commit()
        b.close()
    return data_dir, os.path.join(root, "base")


class ReaderTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="bibox-zotero-")
        self.data_dir, self.base = make_fixture(self.tmp)
        self.lib = zr.ZoteroLibrary(os.path.join(self.data_dir, "zotero.sqlite"), self.data_dir, self.base)

    def by_key(self, key):
        return next(i for i in self.lib.items() if i["key"] == key)

    def test_skips_trash_attachments_and_standalone_notes(self):
        keys = sorted(i["key"] for i in self.lib.items())
        self.assertEqual(keys, ["AAAA0001", "CONF0003", "PREP0002"])

    def test_maps_journal_article_fields_creators_tags_collections_and_file(self):
        m = zr.map_item(self.by_key("AAAA0001"), {}, all_tags=False)
        e = m.entry
        self.assertEqual(e["entry_type"], "article")
        self.assertEqual(e["title"], "Deep Learning")
        self.assertEqual(e["author"], ["LeCun, Yann", "Bengio, Yoshua"], "orderIndex order, not insertion order")
        self.assertEqual(e["editor"], "Editor, Ed")
        self.assertEqual((e["year"], e["month"]), (2015, "may"))
        self.assertEqual((e["journal"], e["volume"], e["number"], e["pages"]), ("Nature", "521", "7553", "436-444"))
        self.assertEqual(e["doi"], "10.1038/nature14539")
        self.assertEqual(e["abstract"], "Abs")
        self.assertEqual(e["tags"], ["deep"])
        self.assertEqual(e["collections"], ["Parent/Child"])
        self.assertEqual(e["file"], os.path.join(self.tmp, "linked", "deep.pdf"))
        self.assertIsNone(m.missing_pdf)
        self.assertNotIn("bibtex_key", e)
        self.assertEqual(len(m.notes_md), 1)
        self.assertIn("**Draft**", m.notes_md[0])

    def test_all_tags_includes_automatic(self):
        m = zr.map_item(self.by_key("AAAA0001"), {}, all_tags=True)
        self.assertEqual(m.entry["tags"], ["deep", "Computer Science - Machine Learning"])

    def test_preprint_becomes_misc_with_howpublished_and_note(self):
        m = zr.map_item(self.by_key("PREP0002"), {})
        e = m.entry
        self.assertEqual(e["entry_type"], "misc")
        self.assertEqual(e["howpublished"], "arXiv")
        self.assertEqual(e["url"], "https://arxiv.org/abs/1706.03762")
        self.assertEqual(e["year"], 2017)
        self.assertNotIn("month", e)
        self.assertEqual(e["note"], "arXiv: 1706.03762")
        self.assertNotIn("file", e)
        self.assertTrue(m.missing_pdf.endswith(os.path.join("storage", "ATT00006", "vaswani.pdf")), m.missing_pdf)

    def test_conference_paper_gets_booktitle_and_attachments_path(self):
        m = zr.map_item(self.by_key("CONF0003"), {})
        self.assertEqual(m.entry["entry_type"], "inproceedings")
        self.assertEqual(m.entry["booktitle"], "AAAI")
        self.assertEqual(m.entry["collections"], ["Parent"])
        self.assertEqual(m.entry["file"], os.path.join(self.base, "sub", "livebot.pdf"))

    def test_resolves_storage_linked_and_attachments_paths(self):
        d = self.data_dir
        self.assertEqual(zr.resolve_attachment("storage:a b.pdf", "K1", d, None), os.path.join(d, "storage", "K1", "a b.pdf"))
        self.assertEqual(zr.resolve_attachment("/abs/x.pdf", "K1", d, None), "/abs/x.pdf")
        self.assertEqual(zr.resolve_attachment("attachments:sub/y.pdf", "K1", d, "/base"), os.path.join("/base", "sub", "y.pdf"))
        self.assertIsNone(zr.resolve_attachment("attachments:sub/y.pdf", "K1", d, None), "no base path configured")
        self.assertIsNone(zr.resolve_attachment(None, "K1", d, None))

    def test_date_parts_handles_zotero_and_bare_years(self):
        self.assertEqual(zr.date_parts("2020-08-31 2020-08-31"), (2020, "aug"))
        self.assertEqual(zr.date_parts("2017"), (2017, None))
        self.assertEqual(zr.date_parts("2019-07 July 2019"), (2019, "jul"))
        self.assertEqual(zr.date_parts("2019-00-00 2019"), (2019, None))
        self.assertEqual(zr.date_parts(""), (None, None))
        self.assertEqual(zr.date_parts(None), (None, None))

    def test_better_bibtex_keys_win(self):
        data_dir, base = make_fixture(os.path.join(self.tmp, "bbt"), bbt={1: "lecun2015deep"})
        keys = zr.citation_keys(data_dir)
        self.assertEqual(keys, {1: "lecun2015deep"})
        lib = zr.ZoteroLibrary(os.path.join(data_dir, "zotero.sqlite"), data_dir, base)
        item = next(i for i in lib.items() if i["key"] == "AAAA0001")
        self.assertEqual(zr.map_item(item, keys).entry["bibtex_key"], "lecun2015deep")
        self.assertEqual(zr.citation_keys(self.data_dir), {}, "no better-bibtex.sqlite: no keys")

    def test_read_prefs_and_find_data_dir_precedence(self):
        home = os.path.join(self.tmp, "home")
        prof = os.path.join(home, "Library", "Application Support", "Zotero", "Profiles", "abcd.default")
        os.makedirs(prof)
        with open(os.path.join(prof, "prefs.js"), "w") as f:
            f.write('user_pref("extensions.zotero.dataDir", "%s");\nuser_pref("extensions.zotero.baseAttachmentPath", "/some/base");\n' % self.data_dir)
        prefs = zr.read_prefs(os.path.dirname(prof))
        self.assertEqual(prefs["dataDir"], self.data_dir)
        self.assertEqual(prefs["baseAttachmentPath"], "/some/base")
        self.assertEqual(zr.find_data_dir(None, home=home), self.data_dir, "prefs.js wins over ~/Zotero")
        os.makedirs(os.path.join(home, "Zotero"))
        with open(os.path.join(home, "Zotero", "zotero.sqlite"), "wb"):
            pass
        self.assertEqual(zr.find_data_dir(self.data_dir, home=home), self.data_dir, "an explicit dir wins")
        os.remove(os.path.join(prof, "prefs.js"))
        self.assertEqual(zr.find_data_dir(None, home=home), os.path.join(home, "Zotero"), "fallback")
        with self.assertRaises(zr.ZoteroError):
            zr.find_data_dir(os.path.join(self.tmp, "nowhere"), home=home)

    def test_snapshot_copies_the_database_and_opens_read_only(self):
        snap = zr.snapshot_db(self.data_dir, os.path.join(self.tmp, "snap"))
        self.assertTrue(os.path.isfile(snap))
        self.assertNotEqual(snap, os.path.join(self.data_dir, "zotero.sqlite"))
        lib = zr.ZoteroLibrary(snap, self.data_dir, self.base)
        self.assertEqual(len(lib.items()), 3)


class NoteTests(unittest.TestCase):
    def test_html_to_markdown_basic_blocks_and_inline(self):
        md = zr.html_to_markdown('<div class="zotero-note znv1"><p>Comment: <b>Draft</b> v2</p><p>Second <i>para</i></p></div>')
        self.assertEqual(md, "Comment: **Draft** v2\n\nSecond *para*")

    def test_html_to_markdown_lists_links_headings_and_code(self):
        md = zr.html_to_markdown('<div data-schema-version="9"><h1>Title</h1><ul><li>one</li><li>two</li></ul><ol><li>a</li></ol><p><a href="https://x.org">x</a> and <code>y</code></p></div>')
        self.assertEqual(md, "# Title\n\n- one\n- two\n\n1. a\n\n[x](https://x.org) and `y`")

    def test_html_to_markdown_strips_wrapper_entities_and_blank_runs(self):
        md = zr.html_to_markdown("<div><p>A &amp; B</p><br><br><p></p><p>C</p></div>")
        self.assertEqual(md, "A & B\n\nC")
        self.assertEqual(zr.html_to_markdown(""), "")


if __name__ == "__main__":
    unittest.main()
