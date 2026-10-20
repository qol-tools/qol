#!/usr/bin/env bash
bin="$1"
test="$2"
dir="$3"
n="$4"
if "$bin" --exact "$test" --nocapture >"$dir/$n.log" 2>&1; then
  rm -f "$dir/$n.log"
fi
