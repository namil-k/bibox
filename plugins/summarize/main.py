import os
import shutil
import subprocess

from bibox_plugin import bibox, confirm, progress, serve

PROMPT = (
    "Summarize this paper for a researcher's reading notes. Use Markdown with these bold headings: "
    "**Question**, **Method**, **Findings**, **Limitations**, **Why it matters**. "
    "Be concrete and quote numbers when the paper gives them. Stay under 300 words.\n\n"
)


def extract_text(pdf_path):
    if shutil.which("pdftotext"):
        r = subprocess.run(["pdftotext", "-layout", pdf_path, "-"], capture_output=True, text=True)
        if r.returncode == 0 and r.stdout.strip():
            return r.stdout
    try:
        from pypdf import PdfReader
    except ImportError:
        raise RuntimeError("install pdftotext (poppler) or run: pip install pypdf")
    return "\n".join((page.extract_text() or "") for page in PdfReader(pdf_path).pages)


def summarize(ctx):
    entry = ctx.get("entry")
    if not entry:
        return {"error": "no entry selected"}
    if not entry.get("file_path"):
        return {"error": "this entry has no PDF attached"}
    cfg = ctx.get("config", {})
    model = cfg.get("model", "claude-opus-5")
    effort = cfg.get("effort", "medium")
    max_tokens = int(cfg.get("max_tokens", 4000))
    max_chars = int(cfg.get("max_chars", 400000))

    try:
        import anthropic
    except ImportError:
        return {"error": "run: pip install anthropic"}

    note_path = os.path.join(ctx["paths"]["notes"], entry["bibtex_key"] + ".md")
    if os.path.exists(note_path):
        with open(note_path, encoding="utf-8") as f:
            if "## Summary" in f.read() and not confirm("A Summary section exists. Overwrite?"):
                return {}

    progress("extracting text")
    text = extract_text(os.path.join(ctx["paths"]["pdfs"], entry["file_path"]))
    if len(text) > max_chars:
        return {"error": "PDF text is {} chars, over max_chars={}; raise [plugins.summarize] max_chars to allow it".format(len(text), max_chars)}

    progress("asking {}".format(model))
    client = anthropic.Anthropic()  # ANTHROPIC_API_KEY, or an `ant auth login` profile
    content = PROMPT + "Title: {}\nAuthors: {}\n\n{}".format(entry.get("title"), ", ".join(entry.get("author", [])), text)
    with client.beta.messages.stream(
        model=model,
        max_tokens=max_tokens,
        output_config={"effort": effort},
        betas=["server-side-fallback-2026-07-01"],
        fallbacks="default",
        messages=[{"role": "user", "content": content}],
    ) as stream:
        for _ in stream.text_stream:
            pass
        msg = stream.get_final_message()
    if msg.stop_reason == "refusal":
        return {"error": "the model declined to summarize this document"}
    summary = "".join(b.text for b in msg.content if b.type == "text").strip()
    if not summary:
        return {"error": "empty response from the model"}

    progress("writing note")
    bibox("note", entry["bibtex_key"], "--section", "Summary", "--stdin", input=summary + "\n", json_output=False)
    return {"refresh": True, "message": "Summary written by {}".format(msg.model)}


serve({"summarize": summarize})
