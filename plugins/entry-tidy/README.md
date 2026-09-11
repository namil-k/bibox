# entry-tidy

Normalizes the selected entries: authors become `Last, F.`, all-caps titles become Title Case, a lowercase first letter is capitalized, and stray whitespace is trimmed. Bound to `=` by default and shown in the right-click menu. Also runs as a `before_add` hook, so every entry added from the CLI is tidied before it is saved; if the plugin fails, the entry is added unchanged.

Install: `bibox plugin install namil-k/bibox/plugins/entry-tidy`
