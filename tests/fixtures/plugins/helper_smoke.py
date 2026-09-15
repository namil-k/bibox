#!/usr/bin/env python3
"""헬퍼 v2를 쓰는 최소 플러그인. 호스트 테스트가 이것으로 헬퍼를 검증한다."""
import os, sys
sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "..", "plugins", "lib"))
from bibox_plugin import serve, on, window, status, fields, config

def go(params):
    i = window.pick("Style", ["APA", "IEEE"])
    status.set("s", "picked {}".format(i))
    return "ran {} with model {}".format(params["command"], config.get("model", "-"))

def get_fields(keys, entries):
    return {k: {"count": "1"} for k in keys}

@on("library/written")
def written(params):
    fields.set({e["bibtex_key"]: {"count": {"text": "2", "color": "accent"}} for e in params["entries"]})

serve({"go": go}, fields=get_fields)
