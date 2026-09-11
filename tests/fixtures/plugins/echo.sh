#!/bin/sh
while IFS= read -r line; do
  printf '{"message":"cwd=%s dir=%s api=%s"}\n' "$(pwd)" "$BIBOX_PLUGIN_DIR" "$BIBOX_API"
done
