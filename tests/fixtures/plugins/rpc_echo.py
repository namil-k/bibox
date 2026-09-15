#!/usr/bin/env python3
"""initialize에 답하고, commands/run은 메시지로 되돌려주고, fields/get은 키마다 count=1, shutdown이면 끝난다."""
import json, sys

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n"); sys.stdout.flush()

for line in sys.stdin:
    m = json.loads(line)
    method, rid, p = m.get("method"), m.get("id"), m.get("params", {})
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": rid, "result": {"name": "echo", "protocol": 2}})
    elif method == "commands/run":
        key = (p.get("entry") or {}).get("bibtex_key", "-")
        send({"jsonrpc": "2.0", "id": rid, "result": {"message": "ran {} on {}".format(p.get("command"), key)}})
    elif method == "fields/get":
        send({"jsonrpc": "2.0", "id": rid, "result": {"fields": {k: {"count": {"text": "1"}} for k in p.get("keys", [])}}})
    elif method == "shutdown":
        send({"jsonrpc": "2.0", "id": rid, "result": {}}); break
    elif rid is not None:
        send({"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": "unknown method " + str(method)}})
