#!/usr/bin/env bash
# Sim 03 — V3: config probe.
# V3 contract: every feature exposes a read-only config surface.
LAB="$(cd "$(dirname "$0")/.." && pwd)"
BINS=(alt-tab bluetooth cli-sessions controllers ide-checkout keyremap launcher lights \
      os-themes pointz qol-shot qol-voice removeapp window-actions qol-tray-install)

echo "== V3 config probe: config show =="
for b in "${BINS[@]}"; do
  out="$("$LAB/bins/$b" config show 2>&1 | head -1)"; code=$?
  printf '%-18s exit=%-3s  %s\n' "$b" "$code" "$out"
done

echo
echo "== V3 config probe: config get <key> =="
for b in "${BINS[@]}"; do
  out="$("$LAB/bins/$b" config get anything 2>&1 | head -1)"; code=$?
  printf '%-18s exit=%-3s  %s\n' "$b" "$code" "$out"
done

echo
echo "== what config questions can a script answer today? =="
"$LAB/bins/bluetooth" doctor --json | grep -o '"config_readable","status":"[a-z]*"' | head -1
echo "(only a boolean readable/not-readable verdict — no values)"
