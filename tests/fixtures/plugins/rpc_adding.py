#!/usr/bin/env python3
"""library/adding에 title을 fixed로 바꿔 돌려준다."""
import json, sys

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n"); sys.stdout.flush()

for line in sys.stdin:
    m = json.loads(line)
    method, rid, p = m.get("method"), m.get("id"), m.get("params", {})
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": rid, "result": {"name": "adding", "protocol": 2}})
    elif method == "library/adding":
        e = dict(p.get("entry") or {}); e["title"] = "fixed"
        send({"jsonrpc": "2.0", "id": rid, "result": {"entry": e}})
    elif method == "shutdown":
        send({"jsonrpc": "2.0", "id": rid, "result": {}}); break
    elif rid is not None:
        send({"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": "unknown"}})
