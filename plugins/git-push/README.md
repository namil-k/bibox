# git-push

Runs `git push` on the library repository after every write. It needs `git = true` in `config.toml`, which makes bibox commit each write; this plugin then pushes that commit in the background. Nothing is shown when the push succeeds. A failure (no remote, no network, rejected push) is reported once in the status line and does not block the save.

Settings under `[plugins.git-push]` in `config.toml`:

| Key | Default | Meaning |
|---|---|---|
| `remote` | `"origin"` | Remote to push to |

The repository is `home` from `config.toml` when set, otherwise the directory that holds the database file.

Install: `bibox plugin install namil-k/bibox/plugins/git-push`
