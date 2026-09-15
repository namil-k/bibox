#!/usr/bin/env python3
"""commands/run 중에 window/pick을 묻고 답을 받아 메시지에 넣는다. fields/get에는 영영 답하지 않는다(타임아웃 테스트)."""
import json, sys

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n"); sys.stdout.flush()

def read():
    line = sys.stdin.readline()
    return json.loads(line) if line else None

while True:
    m = read()
    if m is None: break
    method, rid = m.get("method"), m.get("id")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": rid, "result": {"name": "ask", "protocol": 2}})
    elif method == "commands/run":
        send({"jsonrpc": "2.0", "id": 100, "method": "window/pick", "params": {"title": "Style", "items": ["APA", "IEEE"]}})
        answer = read()
        idx = (answer or {}).get("result", {}).get("index")
        send({"jsonrpc": "2.0", "id": rid, "result": {"message": "picked {}".format(idx)}})
    elif method == "fields/get":
        pass
    elif method == "shutdown":
        send({"jsonrpc": "2.0", "id": rid, "result": {}}); break
