"""bibox plugin helper. Copy this file next to your main.py. Standard library only, Python 3.8+.

Wire format: bibox writes one JSON request per line to stdin. The plugin answers with zero or
more {"ui": ...} lines (bibox answers each with one line) and then one final line without "ui".
Final fields: message, apply (list of full entries), refresh (bool), error.
"""
import json
import os
import shutil
import subprocess
import sys

_out = sys.stdout  # protocol channel; serve() redirects sys.stdout to stderr
_trigger = ""


def _send(obj):
    _out.write(json.dumps(obj, ensure_ascii=False))
    _out.write("\n")
    _out.flush()


def _ask(obj):
    _send(obj)
    line = sys.stdin.readline()
    if not line:
        raise SystemExit(0)  # bibox closed stdin
    return json.loads(line)


def pick(title, items):
    """Show a list; returns the chosen index or None when cancelled."""
    return _ask({"ui": "pick", "title": title, "items": list(items)}).get("index")


def prompt(title, default=""):
    """Ask for one line of text; returns the string or None when cancelled."""
    return _ask({"ui": "prompt", "title": title, "default": default}).get("text")


def confirm(title):
    return bool(_ask({"ui": "confirm", "title": title}).get("yes"))


def progress(text):
    """Update the spinner text. Never blocks on the user."""
    _ask({"ui": "progress", "text": text})


def bibox(*args, input=None, json_output=True):
    """Run `bibox <args>` (with --json unless json_output=False) and return parsed output."""
    env = dict(os.environ)
    if _trigger.startswith("hook:"):
        env["BIBOX_IN_HOOK"] = "1"  # that bibox must not fire plugin hooks again
    argv = [os.environ.get("BIBOX_BIN", "bibox"), *args] + (["--json"] if json_output else [])
    r = subprocess.run(argv, input=input, capture_output=True, text=True, env=env)
    if r.returncode != 0:
        raise RuntimeError(r.stderr.strip() or "bibox {} failed".format(" ".join(args)))
    if json_output and r.stdout.strip():
        return json.loads(r.stdout)
    return r.stdout


def copy_to_clipboard(text):
    for cmd in (["pbcopy"], ["wl-copy"], ["xclip", "-selection", "clipboard"]):
        if shutil.which(cmd[0]):
            subprocess.run(cmd, input=text, text=True, check=True)
            return
    raise RuntimeError("no clipboard command found (pbcopy, wl-copy or xclip)")


def serve(handlers):
    """Loop over requests. `handlers` maps a command id to fn(context) -> dict or None."""
    global _trigger
    sys.stdout = sys.stderr  # a stray print() must not corrupt the protocol
    for line in sys.stdin:
        try:
            req = json.loads(line)
        except ValueError:
            continue
        _trigger = req.get("trigger", "")
        fn = handlers.get(req.get("id"))
        if fn is None:
            _send({"error": "unknown command {!r}".format(req.get("id"))})
            continue
        try:
            result = fn(req.get("context", {}))
            _send(result if isinstance(result, dict) else {})
        except SystemExit:
            raise
        except Exception as e:  # noqa: BLE001 - every failure must become a protocol line
            import traceback

            traceback.print_exc(file=sys.stderr)
            _send({"error": "{}: {}".format(type(e).__name__, e)})
