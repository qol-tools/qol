#!/usr/bin/env bash
# Sim 03 — V3: config probe.
# V3 contract: every feature exposes a read-only config surface.
LAB="$(cd "$(dirname "$0")/.." && pwd)"
BINS=(qol-alt-tab qol-bluetooth qol-cli-sessions qol-controllers qol-ide-checkout qol-keyremap qol-launcher qol-lights \
      qol-os-themes qol-pointz qol-shot qol-voice qol-removeapp qol-window-actions qol-tray-install)

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
"$LAB/bins/qol-bluetooth" doctor --json | grep -o '"config_readable","status":"[a-z]*"' | head -1
echo "(only a boolean readable/not-readable verdict — no values)"
