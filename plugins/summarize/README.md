# summarize

Press `S` on an entry with a PDF attached. The plugin extracts the text, sends it to Claude, and writes the answer into the entry's note under a `## Summary` heading (Question, Method, Findings, Limitations, Why it matters). If the note already has a Summary section it asks before overwriting. The status line shows the stages: extracting text, asking the model, writing the note.

## Requirements

- `pip install anthropic`
- `pdftotext` (poppler) on PATH, or `pip install pypdf` as a fallback
- Credentials: `ANTHROPIC_API_KEY` in the environment, or a profile from `ant auth login`

## Settings

Put these under `[plugins.summarize]` in `config.toml`. All are optional.

| Key | Default | Meaning |
|---|---|---|
| `model` | `"claude-opus-5"` | Model id |
| `effort` | `"medium"` | `low`, `medium`, `high`, `xhigh` or `max`. Controls thinking depth and cost |
| `max_tokens` | `4000` | Output cap. The prompt asks for under 300 words, so this is generous |
| `max_chars` | `400000` | Refuse PDFs whose extracted text is longer than this, to keep one summary from costing more than expected |

## Refusal fallback

The request opts into Anthropic's server-side refusal fallback (`fallbacks="default"` with the `server-side-fallback-2026-07-01` beta header). If the model declines a document on policy grounds, the API re-runs the same request on a fallback model inside the same call, and the message shows which model actually answered. To turn it off, delete the `betas=` and `fallbacks=` lines in `main.py`. If your installed SDK rejects `fallbacks` with a `TypeError`, remove those two lines and change `client.beta.messages.stream` to `client.messages.stream`.

Install: `bibox plugin install namil-k/bibox/plugins/summarize`
