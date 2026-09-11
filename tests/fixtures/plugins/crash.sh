#!/bin/sh
IFS= read -r line
echo "Traceback (most recent call last): boom" >&2
exit 1
