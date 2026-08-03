#!/usr/bin/env bash
# Sim 05 — V5: full journey + residual friction probes.
LAB="$(cd "$(dirname "$0")/.." && pwd)"
BINS=(alt-tab bluetooth cli-sessions controllers ide-checkout keyremap launcher lights \
      os-themes pointz qol-shot qol-voice removeapp template window-actions \
      qol-tray-install qol-tray-migrate qol)

echo "== V5 journey: doctor --json parses for every feature =="
for b in "${BINS[@]}"; do
  out="$("$LAB/bins/$b" --json doctor 2>&1 | head -c 60)"; code=$?
  printf '%-18s exit=%-3s  %s\n' "$b" "$code" "$out"
done

echo
echo "== V5 journey: doctor --fix where a fix exists =="
"$LAB/bins/controllers" doctor --fix
echo
"$LAB/bins/bluetooth" doctor --fix

echo
echo "== residual friction 1: legacy fallback swallows typos =="
out="$("$LAB/bins/pointz" stting 2>&1)"; code=$?
printf 'pointzerver stting -> exit=%-3s  %s\n' "$code" "$out"
out="$("$LAB/bins/pointz" --action stting 2>&1)"; code=$?
printf 'pointzerver --action stting -> exit=%-3s  %s\n' "$code" "$out"

echo
echo "== residual friction 2: dashed command names in help =="
"$LAB/bins/alt-tab" help | grep -E '^\s+--' | head -3
"$LAB/bins/launcher" help | grep -E '^\s+--' | head -3

echo
echo "== residual friction 3: exit code 2 means what? =="
"$LAB/bins/removeapp" remove "Foo" 2>/dev/null; echo "removeapp refusal exit=$?"
"$LAB/bins/bluetooth" doctor >/dev/null 2>&1; echo "doctor warn exit=$?"
"$LAB/bins/qol-tray-migrate" run >/dev/null 2>&1; echo "no-op migrate exit=$?"

echo
echo "== residual friction 4: no-args safety, final state =="
for b in qol-shot qol-tray-install qol-tray-migrate removeapp; do
  out="$("$LAB/bins/$b" 2>&1 | head -1)"; code=$?
  printf '%-18s exit=%-3s  %s\n' "$b" "$code" "$out"
done
