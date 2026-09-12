#!/bin/sh
# Feeds one request to the example plugin and checks the final line. Needs python3 only.
set -eu
cd "$(dirname "$0")/.."

ENTRY='{"id":"1","bibtex_key":"kim2025rust","entry_type":"article","title":"rust systems programming","author":["kim, jinho","PARK S."],"year":2025,"journal":"J. Sys","volume":"1","number":"2","pages":"1--10","publisher":null,"editor":null,"edition":null,"isbn":null,"booktitle":null,"doi":"10.1/x","url":null,"abstract":null,"tags":[],"howpublished":null,"month":null,"note":null,"collections":[],"file_path":null,"created_at":"2026-01-01 00:00:00","updated_at":null}'

req() {
  printf '{"type":"command","id":"%s","trigger":"key","context":{"focus":"entries","collection":null,"entry":%s,"entries":[%s],"config":{},"paths":{"config_dir":"/tmp","db":"/tmp/db.json","notes":"/tmp/notes","pdfs":"/tmp/pdfs","home":null},"hook":null}}\n' "$1" "$ENTRY" "$ENTRY"
}

fail() { echo "FAIL $1"; echo "$2"; exit 1; }

out=$(cd plugins/summarize && unset ANTHROPIC_API_KEY && req summarize | python3 main.py 2>/dev/null)
echo "$out" | grep -q 'no PDF attached' || fail "summarize: expected the no-PDF error" "$out"

out=$(cd plugins/summarize && printf '{"type":"command","id":"nope","trigger":"key","context":{}}\n' | python3 main.py 2>/dev/null)
echo "$out" | grep -q 'unknown command' || fail "helper: unknown command" "$out"

echo "plugin smoke: ok"
