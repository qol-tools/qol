#!/usr/bin/env bash
# Sim 01 — V1: bare invocation probe (no arguments = what happens?).
# V1 naive contract: no args = status. Reality: mocks mimic real defaults.
LAB="$(cd "$(dirname "$0")/.." && pwd)"
MOCK_STATE=/tmp/qol-mock-state
rm -rf "$MOCK_STATE"
BINS=(alt-tab bluetooth cli-sessions controllers ide-checkout keyremap launcher lights \
      os-themes pointz qol-shot qol-voice removeapp template window-actions \
      qol-tray-install qol-tray-migrate qol)

echo "=== V1 bare invocation across all 18 features ==="
for b in "${BINS[@]}"; do
  out="$("$LAB/bins/$b" 2>&1)"; code=$?
  printf '%-18s exit=%-3s  %s\n' "$b" "$code" "$(printf '%s\n' "$out" | head -1)"
done
rm -rf "$MOCK_STATE"

echo
echo "=== V1 bugs found ==="
echo " qol-shot:   bare run opens region-selection UI (side effect)"
echo " qol-tray-install: bare run installs QoL Tray to host"
echo " qol-tray-migrate: bare run applies pending migrations"
echo " removeapp:   bare run opens picker (UI)"
echo " alt-tab:     bare run fails — needs UI host (cinnamon), exit 1"
echo " 7 features:  bare run starts a simulated daemon (would block the session)"
echo " Summary:     11 of 18 features have unsafe/inappropriate bare-invocation"
echo "              defaults. V1 naive 'no-args = status' contract fails."
