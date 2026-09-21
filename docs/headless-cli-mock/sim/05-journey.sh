#!/usr/bin/env bash
# Sim 05 — V5: full journey + residual friction probes.
LAB="$(cd "$(dirname "$0")/.." && pwd)"
BINS=(qol-alt-tab qol-bluetooth qol-cli-sessions qol-controllers qol-ide-checkout qol-keyremap qol-launcher qol-lights \
      qol-os-themes qol-pointz qol-shot qol-voice qol-removeapp qol-template qol-window-actions \
      qol-tray-install qol-tray-migrate qol)

echo "== V5 journey: doctor --json parses for every feature =="
for b in "${BINS[@]}"; do
  out="$("$LAB/bins/$b" --json doctor 2>&1 | head -c 60)"; code=$?
  printf '%-18s exit=%-3s  %s\n' "$b" "$code" "$out"
done

echo
echo "== V5 journey: doctor --fix where a fix exists =="
"$LAB/bins/qol-controllers" doctor --fix
echo
"$LAB/bins/qol-bluetooth" doctor --fix

echo
echo "== residual friction 1: legacy fallback swallows typos =="
out="$("$LAB/bins/qol-pointz" stting 2>&1)"; code=$?
printf 'qol-pointz stting -> exit=%-3s  %s\n' "$code" "$out"
out="$("$LAB/bins/qol-pointz" --action stting 2>&1)"; code=$?
printf 'qol-pointz --action stting -> exit=%-3s  %s\n' "$code" "$out"

echo
echo "== residual friction 2: dashed command names in help =="
"$LAB/bins/qol-alt-tab" help | grep -E '^\s+--' | head -3
"$LAB/bins/qol-launcher" help | grep -E '^\s+--' | head -3

echo
echo "== residual friction 3: exit code 2 means what? =="
"$LAB/bins/qol-removeapp" remove "Foo" 2>/dev/null; echo "qol-removeapp refusal exit=$?"
"$LAB/bins/qol-bluetooth" doctor >/dev/null 2>&1; echo "doctor warn exit=$?"
"$LAB/bins/qol-tray-migrate" run >/dev/null 2>&1; echo "no-op migrate exit=$?"

echo
echo "== residual friction 4: no-args safety, final state =="
for b in qol-shot qol-tray-install qol-tray-migrate qol-removeapp; do
  out="$("$LAB/bins/$b" 2>&1 | head -1)"; code=$?
  printf '%-18s exit=%-3s  %s\n' "$b" "$code" "$out"
done
