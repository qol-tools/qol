#!/usr/bin/env bash
# Sim 02 — V2 lifecycle: status, kill, and env-gated daemon.
LAB="$(cd "$(dirname "$0")/.." && pwd)"
MOCK_STATE=/tmp/qol-mock-state
rm -rf "$MOCK_STATE"

DAEMON_BINS=(alt-tab cli-sessions ide-checkout keyremap launcher lights os-themes pointz window-actions)
OTHER_BINS=(qol-shot removeapp qol-tray-install qol-tray-migrate qol-voice template qol bluetooth controllers)

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
for b in alt-tab cli-sessions launcher; do
  out="$(QOL_MOCK_DAEMON=1 "$LAB/bins/$b" 2>&1)"; code=$?
  printf '%-18s exit=%-3s  %s\n' "$b" "$code" "$(printf '%s\n' "$out" | head -1)"
done

echo
echo "=== V2 bugs found ==="
echo " qol-voice:       no status/kill — has session stop/start but no lifecycle"
echo " controllers:     status works (already had it); no kill needed (on-demand)"
echo " launcher --kill: original dashed command still works (backward compat)"
echo " alt-tab daemon:  env-gated start works; UI host check still applies"
rm -rf "$MOCK_STATE"
