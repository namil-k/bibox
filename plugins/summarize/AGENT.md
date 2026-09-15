# summarize

TUI only: a person presses `S` on an entry with a PDF attached, and the plugin extracts the text, asks Claude, and writes the answer into the entry's note under `## Summary` (Question, Method, Findings, Limitations, Why it matters). There is no `[cli]`, so an agent cannot call it as `bibox summarize`.

If you are an agent asked to summarize a paper, do the same thing directly: read the PDF (`bibox show <key> --json` gives `file_path`), write the summary, and store it with `bibox note <key> --section Summary --stdin`. That is exactly what this plugin does through `$BIBOX_BIN`.

Needs: `pip install anthropic`, `pdftotext` on PATH (or `pip install pypdf`), and `ANTHROPIC_API_KEY` (or an `ant auth login` profile). Settings under `[plugins.summarize]`: `model`, `effort`, `max_tokens`, `max_chars` (see README.md).
