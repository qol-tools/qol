#!/usr/bin/env bash
# Sim 04 — V4: repair probe.
# V4 contract: doctor reports a fix; doctor --fix applies it.
LAB="$(cd "$(dirname "$0")/.." && pwd)"
BINS=(bluetooth controllers qol-shot qol-voice os-themes alt-tab)

echo "== V4 repair probe: doctor --fix =="
for b in "${BINS[@]}"; do
  out="$("$LAB/bins/$b" doctor --fix 2>&1 | head -1)"; code=$?
  printf '%-18s exit=%-3s  %s\n' "$b" "$code" "$out"
done

echo
echo "== what do doctor reports actually offer? =="
"$LAB/bins/bluetooth" doctor
echo
"$LAB/bins/controllers" doctor
echo
echo "(fixes are prose instructions — a script cannot apply them)"
