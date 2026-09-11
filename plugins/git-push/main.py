import os
import subprocess

from bibox_plugin import serve


def push(ctx):
    home = ctx["paths"].get("home") or os.path.dirname(ctx["paths"]["db"])
    remote = ctx.get("config", {}).get("remote", "origin")
    r = subprocess.run(["git", "-C", home, "push", "-q", remote], capture_output=True, text=True)
    if r.returncode != 0:
        lines = r.stderr.strip().splitlines() or ["git push failed"]
        # git follows "fatal: ..." with several lines of advice; report the fatal line, not the advice.
        headline = next((l for l in lines if l.startswith(("fatal:", "error:"))), lines[-1])
        return {"error": headline}
    return {}


serve({"push": push})
