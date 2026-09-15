"""bibox plugin helper, protocol 2 (JSON-RPC 2.0 over stdin/stdout, one message per line).

Copy this file next to your main.py. It has no dependencies beyond the standard library.
"""
import json
import os
import subprocess
import sys
import threading

_out = sys.stdout
sys.stdout = sys.stderr  # a stray print() must not corrupt the protocol
_lock = threading.Lock()
_next_id = 1
_pending = {}      # id -> (Event, [result, error])
_deferred = []     # host lines that arrived while we waited for an answer
_handlers = {}     # event method -> fn(params)
config = {}
paths = {}
capabilities = {}


def _write(obj):
    with _lock:
        _out.write(json.dumps(obj) + "\n")
        _out.flush()


def _notify(method, params):
    _write({"jsonrpc": "2.0", "method": method, "params": params})


def _request(method, params):
    """Send a request and read until its answer arrives. Other lines are handled after."""
    global _next_id
    with _lock:
        rid = _next_id
        _next_id += 1
    _write({"jsonrpc": "2.0", "id": rid, "method": method, "params": params})
    while True:
        line = sys.stdin.readline()
        if not line:
            sys.exit(0)
        try:
            msg = json.loads(line)
        except ValueError:
            continue
        if msg.get("id") == rid and "method" not in msg:
            if "error" in msg:
                raise RuntimeError(msg["error"].get("message", "error"))
            return msg.get("result")
        _deferred.append(msg)


class window:
    @staticmethod
    def pick(title, items):
        return _request("window/pick", {"title": title, "items": list(items)}).get("index")

    @staticmethod
    def prompt(title, default=""):
        return _request("window/prompt", {"title": title, "default": default}).get("text")

    @staticmethod
    def confirm(title):
        return bool(_request("window/confirm", {"title": title}).get("yes"))

    @staticmethod
    def progress(text):
        _notify("window/progress", {"text": text})

    @staticmethod
    def message(text, level="info"):
        _notify("window/message", {"text": text, "level": level})


class status:
    @staticmethod
    def set(field, text, color=None):
        _notify("status/set", {"field": field, "text": text, "color": color})


def _norm_fields(m):
    return {k: {i: (v if isinstance(v, dict) else {"text": str(v)}) for i, v in row.items()} for k, row in m.items()}


class fields:
    @staticmethod
    def set(m):
        _notify("fields/set", {"fields": _norm_fields(m)})


class library:
    @staticmethod
    def refresh():
        _notify("library/refresh", {})

    @staticmethod
    def apply(entries):
        return _request("library/apply", {"entries": list(entries)}).get("applied", 0)


class commands:
    @staticmethod
    def execute(name):
        _request("commands/execute", {"command": name})


def on(event):
    def deco(fn):
        _handlers[event] = fn
        return fn
    return deco


class cache:
    _path = None
    _data = None

    @classmethod
    def _load(cls):
        if cls._data is None:
            cls._path = os.path.join(os.environ.get("BIBOX_PLUGIN_DIR", "."), "cache.json")
            try:
                with open(cls._path) as f:
                    cls._data = json.load(f)
            except (OSError, ValueError):
                cls._data = {}
        return cls._data

    @classmethod
    def get(cls, key):
        return cls._load().get(key)

    @classmethod
    def set(cls, key, value):
        d = cls._load()
        d[key] = value
        tmp = cls._path + ".tmp"
        with open(tmp, "w") as f:
            json.dump(d, f)
        os.replace(tmp, cls._path)


def spawn(fn, *args):
    t = threading.Thread(target=fn, args=args, daemon=True)
    t.start()
    return t


def bibox(*args, input=None, json_output=True):
    """Run the bibox CLI. Returns parsed JSON (or text). Raises on a non-zero exit."""
    cmd = [os.environ.get("BIBOX_BIN", "bibox")] + list(args)
    p = subprocess.run(cmd, input=input, capture_output=True, text=True)
    if p.returncode != 0:
        raise RuntimeError(p.stderr.strip() or "bibox {} failed".format(" ".join(args)))
    return json.loads(p.stdout) if json_output and p.stdout.strip() else p.stdout


def copy_to_clipboard(text):
    for cmd in (["pbcopy"], ["wl-copy"], ["xclip", "-selection", "clipboard"]):
        try:
            subprocess.run(cmd, input=text, text=True, check=True)
            return True
        except (OSError, subprocess.CalledProcessError):
            continue
    return False


def _error(rid, code, message):
    _write({"jsonrpc": "2.0", "id": rid, "error": {"code": code, "message": message}})


def serve(commands_map=None, *, fields=None, view=None, adding=None, name=None):
    """Answer requests until shutdown. Handlers raise to report an error; the message reaches the status line."""
    commands_map = commands_map or {}
    name = name or os.path.basename(os.environ.get("BIBOX_PLUGIN_DIR", "plugin"))
    fields_fn, view_fn, adding_fn = fields, view, adding

    def handle(msg):
        method, rid, params = msg.get("method"), msg.get("id"), msg.get("params") or {}
        if method is None:
            return  # a stray response
        try:
            if method == "initialize":
                config.clear(); config.update(params.get("config") or {})
                paths.clear(); paths.update(params.get("paths") or {})
                capabilities.clear(); capabilities.update(params.get("capabilities") or {})
                _write({"jsonrpc": "2.0", "id": rid, "result": {"name": name, "protocol": 2}})
            elif method == "shutdown":
                _write({"jsonrpc": "2.0", "id": rid, "result": {}})
                sys.exit(0)
            elif method == "config/changed":
                config.clear(); config.update(params.get("config") or {})
            elif method == "commands/run":
                fn = commands_map.get(params.get("command"))
                if fn is None:
                    return _error(rid, -32601, "unknown command {!r}".format(params.get("command")))
                r = fn(params)
                if isinstance(r, str):
                    r = {"message": r}
                _write({"jsonrpc": "2.0", "id": rid, "result": r or {}})
            elif method == "fields/get":
                if fields_fn is None:
                    return _error(rid, -32601, "no fields provider")
                r = fields_fn(params.get("keys") or [], params.get("entries") or [])
                _write({"jsonrpc": "2.0", "id": rid, "result": {"fields": _norm_fields(r or {})}})
            elif method == "views/render":
                if view_fn is None:
                    return _error(rid, -32601, "no view provider")
                _write({"jsonrpc": "2.0", "id": rid, "result": view_fn(params) or {}})
            elif method == "library/adding":
                r = adding_fn(params.get("entry")) if adding_fn else None
                _write({"jsonrpc": "2.0", "id": rid, "result": {"entry": r} if r else {}})
            elif method in _handlers:
                _handlers[method](params)
            elif rid is not None:
                _error(rid, -32601, "unknown method {}".format(method))
        except SystemExit:
            raise
        except Exception as e:  # noqa: BLE001 - every failure must become a protocol line
            import traceback
            traceback.print_exc(file=sys.stderr)
            if rid is not None:
                _error(rid, -32000, "{}: {}".format(type(e).__name__, e))

    while True:
        if _deferred:
            handle(_deferred.pop(0))
            continue
        line = sys.stdin.readline()
        if not line:
            return
        try:
            handle(json.loads(line))
        except ValueError:
            continue
