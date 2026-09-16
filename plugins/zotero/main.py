"""Import a Zotero library into bibox.

CLI (`bibox zotero import`, BIBOX_CLI=1) and TUI command (`zotero.import`) share build_plan/run_import.
All writes go through `$BIBOX_BIN import <json> --json` and `$BIBOX_BIN note <key> --section Zotero --stdin`.
"""
import argparse
import json
import os
import subprocess
import sys
import tempfile
from collections import namedtuple

import zotero_reader as zr

Plan = namedtuple("Plan", "data_dir entries notes missing_pdfs counts")


def _bibox(*args, input=None, json_output=True):
    """Run the bibox CLI (kept local so CLI mode does not import bibox_plugin, which redirects stdout)."""
    cmd = [os.environ.get("BIBOX_BIN", "bibox")] + list(args)
    stdin = subprocess.DEVNULL if input is None else None
    p = subprocess.run(cmd, input=input, stdin=stdin, capture_output=True, text=True)
    if p.returncode != 0:
        raise RuntimeError(p.stderr.strip() or "bibox {} failed".format(" ".join(args)))
    return json.loads(p.stdout) if json_output and p.stdout.strip() else p.stdout


def _in_collection(entry, wanted):
    return any(c == wanted or c.startswith(wanted + "/") for c in entry.get("collections", []))


def build_plan(data_dir=None, all_tags=False, collection=None, home=None, base_path=None):
    """Read the Zotero library and map it. Nothing is written. `notes` maps entry index to markdown."""
    data_dir = zr.find_data_dir(data_dir, home=home)
    base_path = base_path or zr.base_attachment_path(home)
    snap = zr.snapshot_db(data_dir, tempfile.mkdtemp(prefix="bibox-zotero-"))
    lib = zr.ZoteroLibrary(snap, data_dir, base_path)
    keys = zr.citation_keys(data_dir)
    entries, notes, missing = [], {}, []
    for item in lib.items():
        m = zr.map_item(item, keys, all_tags=all_tags)
        if collection and not _in_collection(m.entry, collection):
            continue
        if m.notes_md:
            notes[len(entries)] = "\n\n".join(m.notes_md)
        if m.missing_pdf:
            missing.append(m.missing_pdf)
        entries.append(m.entry)
    counts = {
        "items": len(entries),
        "collections": len({c for e in entries for c in e.get("collections", [])}),
        "pdfs_found": sum(1 for e in entries if "file" in e),
        "pdfs_missing": len(missing),
        "notes": len(notes),
    }
    return Plan(data_dir, entries, notes, missing, counts)


def run_import(plan, dry_run=False, log=print, progress=None):
    """Hand the entries to bibox, then write notes for the entries that landed. Returns counts and the raw outcomes."""
    progress = progress or (lambda text: None)
    with tempfile.NamedTemporaryFile("w", suffix=".json", prefix="bibox-zotero-", delete=False) as f:
        json.dump(plan.entries, f)
        path = f.name
    progress("importing {} entries".format(len(plan.entries)))
    args = ["import", path, "--json"] + (["--dry-run"] if dry_run else [])
    outcomes = _bibox(*args)
    result = {"added": 0, "merged": 0, "skipped": 0, "notes": 0, "pdfs": plan.counts["pdfs_found"], "outcomes": outcomes}
    for o in outcomes:
        result[o.get("status", "skipped")] = result.get(o.get("status", "skipped"), 0) + 1
        reason = o.get("reason") or ""
        if "file not found" in reason:
            log("  ! {}: {}".format(o.get("key"), reason))
    if not dry_run:
        for o in outcomes:
            md = plan.notes.get(o.get("input"))
            if md and o.get("status") in ("added", "merged") and o.get("key"):
                progress("writing note for {}".format(o["key"]))
                _bibox("note", o["key"], "--section", "Zotero", "--stdin", input=md + "\n", json_output=False)
                result["notes"] += 1
    result["entries_file"] = path  # left in the temp dir so a failed import can be inspected
    return result


def summary_text(plan, result=None):
    c = plan.counts
    lines = ["{} entries in {} collections, {} PDFs ({} missing), {} note{} from {}".format(
        c["items"], c["collections"], c["pdfs_found"], c["pdfs_missing"], c["notes"], "" if c["notes"] == 1 else "s", plan.data_dir)]
    if result:
        lines.append("added {}, merged {}, skipped {}, notes written {}".format(result["added"], result["merged"], result["skipped"], result["notes"]))
    return "\n".join(lines)


# ── CLI ─────────────────────────────────────────────────────────────────────

def cli(argv):
    ap = argparse.ArgumentParser(prog="bibox zotero", description="Import a Zotero library into bibox.")
    sub = ap.add_subparsers(dest="cmd")
    imp = sub.add_parser("import", help="read zotero.sqlite and import entries, PDFs and notes")
    imp.add_argument("--data-dir", help="Zotero data directory (default: Zotero's prefs.js, then ~/Zotero)")
    imp.add_argument("--dry-run", action="store_true", help="show what would happen, write nothing")
    imp.add_argument("--all-tags", action="store_true", help="also import tags Zotero added automatically")
    imp.add_argument("--collection", help="only this Zotero collection (and its subcollections), e.g. \"Parent/Child\"")
    imp.add_argument("--yes", "-y", action="store_true", help="do not ask before importing")
    args = ap.parse_args(argv)
    if args.cmd != "import":
        ap.print_help()
        return 2
    try:
        plan = build_plan(data_dir=args.data_dir, all_tags=args.all_tags, collection=args.collection)
    except zr.ZoteroError as e:
        print("bibox zotero: {}".format(e), file=sys.stderr)
        return 1
    print(summary_text(plan))
    for p in plan.missing_pdfs:
        print("  missing PDF: {}".format(p))
    if not plan.entries:
        return 0
    if not args.yes and not args.dry_run:
        try:
            answer = input("Import into bibox? [y/N] ")
        except EOFError:
            answer = ""
        if answer.strip().lower() not in ("y", "yes"):
            print("cancelled")
            return 1
    try:
        result = run_import(plan, dry_run=args.dry_run)
    except RuntimeError as e:
        print("bibox zotero: {}".format(e), file=sys.stderr)
        return 1
    print(summary_text(plan, result).splitlines()[-1] + (" (dry run)" if args.dry_run else ""))
    return 0


# ── TUI command ─────────────────────────────────────────────────────────────

def import_cmd(params):
    from bibox_plugin import config, library, window

    window.progress("reading Zotero")
    plan = build_plan(data_dir=config.get("data_dir") or None, all_tags=bool(config.get("all_tags")))
    c = plan.counts
    if not plan.entries:
        return "Zotero library at {} has no entries".format(plan.data_dir)
    title = "Import {} entries ({} PDFs, {} notes, {} collections) from {}?".format(c["items"], c["pdfs_found"], c["notes"], c["collections"], plan.data_dir)
    if not window.confirm(title):
        return "cancelled"
    result = run_import(plan, dry_run=False, log=lambda *_: None, progress=window.progress)
    library.refresh()
    return summary_text(plan, result).splitlines()[-1]


if __name__ == "__main__":
    if os.environ.get("BIBOX_CLI") == "1":
        sys.exit(cli(sys.argv[1:]))
    from bibox_plugin import serve

    serve({"import": import_cmd})
