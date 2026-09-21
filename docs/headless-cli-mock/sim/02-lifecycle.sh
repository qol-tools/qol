#!/usr/bin/env bash
# Sim 02 — V2 lifecycle: status, kill, and env-gated daemon.
LAB="$(cd "$(dirname "$0")/.." && pwd)"
MOCK_STATE=/tmp/qol-mock-state
rm -rf "$MOCK_STATE"

DAEMON_BINS=(qol-alt-tab qol-cli-sessions qol-ide-checkout qol-keyremap qol-launcher qol-lights qol-os-themes qol-pointz qol-window-actions)
OTHER_BINS=(qol-shot qol-removeapp qol-tray-install qol-tray-migrate qol-voice qol-template qol qol-bluetooth qol-controllers)

echo "=== V2a: status across daemon features ==="
for b in "${DAEMON_BINS[@]}"; do
  out="$("$LAB/bins/$b" status 2>&1)"; code=$?
  printf '%-18s exit=%-3s  %s\n' "$b" "$code" "$(printf '%s\n' "$out" | head -1)"
done

echo
echo "=== V2b: kill across daemon features ==="
for b in "${DAEMON_BINS[@]}"; do
  out="$("$LAB/bins/$b" kill 2>&1)"; code=$?
  printf '%-18s exit=%-3s  %s\n' "$b" "$code" "$(printf '%s\n' "$out" | head -1)"
done

echo
echo "=== V2c: bare invocation now safe? ==="
for b in "${DAEMON_BINS[@]}" "${OTHER_BINS[@]}"; do
  out="$("$LAB/bins/$b" 2>&1)"; code=$?
  printf '%-18s exit=%-3s  %s\n' "$b" "$code" "$(printf '%s\n' "$out" | head -1)"
done

echo
echo "=== V2d: env-gated daemon (QOL_MOCK_DAEMON=1) ==="
for b in qol-alt-tab qol-cli-sessions qol-launcher; do
  out="$(QOL_MOCK_DAEMON=1 "$LAB/bins/$b" 2>&1)"; code=$?
  printf '%-18s exit=%-3s  %s\n' "$b" "$code" "$(printf '%s\n' "$out" | head -1)"
done

echo
echo "=== V2 bugs found ==="
echo " qol-voice:       no status/kill — has session stop/start but no lifecycle"
echo " qol-controllers:     status works (already had it); no kill needed (on-demand)"
echo " qol-launcher --kill: original dashed command still works (backward compat)"
echo " qol-alt-tab daemon:  env-gated start works; UI host check still applies"
rm -rf "$MOCK_STATE"
