#!/usr/bin/env python3
"""commands/run을 받으면 답 없이 코드 3으로 죽는다. shutdown은 무시한다(kill 경로 테스트)."""
import json, sys, time

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n"); sys.stdout.flush()

for line in sys.stdin:
    m = json.loads(line)
    method, rid = m.get("method"), m.get("id")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": rid, "result": {"name": "die", "protocol": 2}})
    elif method == "commands/run":
        sys.exit(3)
    elif method == "shutdown":
        time.sleep(30)
