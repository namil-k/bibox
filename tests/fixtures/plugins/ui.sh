#!/bin/sh
while IFS= read -r line; do
  printf '{"ui":"pick","title":"Style","items":["APA","IEEE"]}\n'
  IFS= read -r answer
  printf '{"message":"picked %s"}\n' "$(printf '%s' "$answer" | tr -d '{}" ')"
done
