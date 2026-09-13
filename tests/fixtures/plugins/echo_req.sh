#!/bin/sh
# 받은 요청 줄을 그대로 message에 담아 돌려준다(JSON 문자열로 인코딩).
while IFS= read -r line; do
  printf '%s' "$line" | python3 -c 'import json,sys; print(json.dumps({"message": sys.stdin.read()}))'
done
