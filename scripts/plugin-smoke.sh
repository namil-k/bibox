#!/bin/sh
# Feeds one request to each example plugin and checks the final line. Needs python3 only.
set -eu
cd "$(dirname "$0")/.."

ENTRY='{"id":"1","bibtex_key":"kim2025rust","entry_type":"article","title":"rust systems programming","author":["kim, jinho","PARK S."],"year":2025,"journal":"J. Sys","volume":"1","number":"2","pages":"1--10","publisher":null,"editor":null,"edition":null,"isbn":null,"booktitle":null,"doi":"10.1/x","url":null,"abstract":null,"tags":[],"howpublished":null,"month":null,"note":null,"collections":[],"file_path":null,"created_at":"2026-01-01 00:00:00","updated_at":null}'

req() {
  printf '{"type":"command","id":"%s","trigger":"key","context":{"focus":"entries","collection":null,"entry":%s,"entries":[%s],"config":{},"paths":{"config_dir":"/tmp","db":"/tmp/db.json","notes":"/tmp/notes","pdfs":"/tmp/pdfs","home":null},"hook":null}}\n' "$1" "$ENTRY" "$ENTRY"
}

fail() { echo "FAIL $1"; echo "$2"; exit 1; }

out=$(cd plugins/entry-tidy && req tidy | python3 main.py 2>/dev/null)
echo "$out" | grep -q '"Kim, J."' || fail "entry-tidy: author not normalized" "$out"
echo "$out" | grep -q '"Park, S."' || fail "entry-tidy: second author not normalized" "$out"
echo "$out" | grep -q '"Rust systems programming"' || fail "entry-tidy: title not tidied" "$out"
echo "$out" | grep -q 'Tidied 1 of 1' || fail "entry-tidy: message" "$out"

out=$(cd plugins/copy-citation && { req copy; printf '{"index":0}\n'; } | python3 main.py 2>/dev/null)
echo "$out" | head -1 | grep -q '"ui": *"pick"' || fail "copy-citation: no pick request" "$out"
echo "$out" | tail -1 | grep -Eq 'Copied 1 APA|no clipboard command' || fail "copy-citation: final line" "$out"

out=$(cd plugins/copy-citation && { req copy; printf '{"index":null}\n'; } | python3 main.py 2>/dev/null)
[ "$(echo "$out" | tail -1)" = "{}" ] || fail "copy-citation: cancel should end with {}" "$out"

out=$(cd plugins/summarize && unset ANTHROPIC_API_KEY && req summarize | python3 main.py 2>/dev/null)
echo "$out" | grep -q 'no PDF attached' || fail "summarize: expected the no-PDF error" "$out"

out=$(cd plugins/git-push && printf '{"type":"command","id":"push","trigger":"hook:after_write","context":{"paths":{"db":"/nonexistent/db.json","home":null},"config":{}}}\n' | python3 main.py 2>/dev/null)
echo "$out" | grep -q '"error"' || fail "git-push: expected an error outside a repository" "$out"

out=$(cd plugins/entry-tidy && printf '{"type":"command","id":"nope","trigger":"key","context":{}}\n' | python3 main.py 2>/dev/null)
echo "$out" | grep -q 'unknown command' || fail "helper: unknown command" "$out"

echo "plugin smoke: ok"
