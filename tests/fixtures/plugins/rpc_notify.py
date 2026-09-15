#!/usr/bin/env python3
"""initialize 직후 status/set 알림을 밀고, bibox가 보낸 알림은 test/got으로 되돌려준다."""
import json, sys

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n"); sys.stdout.flush()

for line in sys.stdin:
    m = json.loads(line)
    method, rid = m.get("method"), m.get("id")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": rid, "result": {"name": "notify", "protocol": 2}})
        send({"jsonrpc": "2.0", "method": "status/set", "params": {"field": "s", "text": "hello"}})
    elif method == "shutdown":
        send({"jsonrpc": "2.0", "id": rid, "result": {}}); break
    elif rid is None:
        send({"jsonrpc": "2.0", "method": "test/got", "params": {"method": method}})
    else:
        send({"jsonrpc": "2.0", "id": rid, "result": {}})
