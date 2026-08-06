#!/usr/bin/env bash
# sessions-relay-verify.sh — guest-VM verification for the sessions relay.
#
# Proves, on a clean Mint guest with kitty 0.32.2 (the Ubuntu/Mint build with
# the send-key --match bug):
#   1. headless relay: list/read/send/wait work with zero qol processes
#      (no qol-tray, no cli-sessions daemon, no plugin socket)
#   2. submit into an UNFOCUSED window executes (the DeliveryMode::Submit fix)
#   3. wait --expect skips the echo of the last send (echo-exclusion)
#
# Usage:
#   verify/sessions-relay-verify.sh <worktree-absolute-path> [kitty-debs-dir]
#
# Inputs:
#   worktree: qol-monorepo worktree whose target/debug/qol will be bundled
#   kitty-debs-dir: dir of .deb files for kitty + closure (default
#                   /tmp/telepathy-vm/kitty-debs)
#
# Output:
#   verify/reports/<run-id>/report.json with status pass|failed
#
# Leaves the guest running on failure for inspection; stops it on pass.

set -euo pipefail

WORKTREE="${1:?usage: sessions-relay-verify.sh <worktree> [kitty-debs-dir]}"
DEBS_DIR="${2:-/tmp/telepathy-vm/kitty-debs}"
ENV=linux/mint-cinnamon
REPORTS_ROOT="$WORKTREE/verify/reports"

cd "$WORKTREE"

if [ ! -f target/debug/qol ]; then
    echo "building qol (cold target dir)..."
    cargo build -p qol
fi
if [ ! -d "$DEBS_DIR" ] || [ -z "$(ls "$DEBS_DIR"/*.deb 2>/dev/null)" ]; then
    echo "error: kitty debs missing in $DEBS_DIR"
    exit 1
fi

echo "== env up =="
UP_OUT=$(qol env up "$ENV" --dev-worktree "$WORKTREE" 2>&1)
echo "$UP_OUT" | tail -2
RUN_ID=$(echo "$UP_OUT" | grep -oE 'linux-mint-cinnamon-[0-9a-f]+-[0-9a-f]+-[0-9a-f]+' | head -1)
if [ -z "$RUN_ID" ]; then echo "error: no run id from env up"; echo "$UP_OUT"; exit 1; fi
CASES="$WORKTREE/target/qol-env/cases/$RUN_ID"
echo "run id: $RUN_ID"

echo "== wait for guest control =="
READY=0
for i in $(seq 1 40); do
    if qol env exec "$RUN_ID" /bin/true >/dev/null 2>&1; then READY=1; break; fi
    sleep 6
done
if [ "$READY" != 1 ]; then echo "error: guest never ready"; exit 1; fi

echo "== carry kitty on the usb stick =="
rm -f "$CASES/usb-stick.raw"
qemu-img create -f raw "$CASES/usb-stick.raw" 64M >/dev/null
mformat -i "$CASES/usb-stick.raw" -c 1 -h 64 -s 32 -t 1024 ::
mcopy -i "$CASES/usb-stick.raw" -s "$DEBS_DIR"/* ::/ >/dev/null
qol emu insert --run-root target/qol-env/cases "$RUN_ID" >/dev/null 2>&1 || true
sleep 5

GUEST_SETUP=$(cat <<'SCRIPT'
set -e
VOL=$(ls /media/qol/ | head -1)
if [ -z "$VOL" ]; then echo "error: stick not mounted"; exit 1; fi
mkdir -p ~/.local/kitty-root
for d in /media/qol/$VOL/*.deb; do dpkg-deb -x "$d" ~/.local/kitty-root 2>/dev/null || true; done
mkdir -p ~/.config/kitty
echo "allow_remote_control yes" > ~/.config/kitty/kitty.conf
export PATH=$HOME/.local/kitty-root/usr/bin:$PATH
export DISPLAY=:0
for u in $(systemctl --user list-units --type=service --no-legend 2>/dev/null | awk '/qol-dev/{print $1}'); do
    systemctl --user stop "$u" 2>/dev/null || true
done
sleep 2
nohup kitty --listen-on unix:/tmp/kitty-relay -- python3 -u >/tmp/kitty1.log 2>&1 &
sleep 8
export KITTY_LISTEN_ON=unix:/tmp/kitty-relay
kitten @ launch --type=os-window -- bash -l >/dev/null 2>&1 || true
sleep 2
echo "windows: $(kitten @ ls | python3 -c 'import json,sys; print(len(json.load(sys.stdin)))')"
SCRIPT
)
qol env exec "$RUN_ID" /usr/bin/env bash -c "$GUEST_SETUP" 2>&1 | tail -3

QOL_BIN=/home/qol/.local/share/qol-dev/current/bin/qol
GUEST_TEST=$(cat <<'SCRIPT'
set -e
export KITTY_LISTEN_ON=unix:/tmp/kitty-relay
export PATH=$HOME/.local/kitty-root/usr/bin:$PATH
Q=$QOL_BIN
PROCS=$(ps aux | grep -E '[q]ol-tray|[c]li-sessions|[q]ol dev' | grep -v grep | grep -v "bash -c" | wc -l)
echo "--- headless proof: qol processes = $PROCS"
if [ "$PROCS" -ne 0 ]; then echo "error: qol processes running in guest"; exit 1; fi
$Q sessions list --json > /tmp/rows.json
python3 - <<'PY'
import json
rows = json.load(open('/tmp/rows.json'))
for r in rows:
    print('row:', r['session'], r['title'][:30], r['tool'], ','.join(r['capabilities']))
assert len(rows) >= 2, "expected two sessions"
py = [r for r in rows if 'python' in r['title'].lower()]
sh = [r for r in rows if r not in py]
assert py and sh, "need one python and one bash session"
open('/tmp/py-token','w').write(py[0]['session'])
open('/tmp/sh-token','w').write(sh[0]['session'])
print('python token:', py[0]['session'])
print('bash token:', sh[0]['session'])
PY
PY_TOKEN=$(cat /tmp/py-token)
SH_TOKEN=$(cat /tmp/sh-token)
echo "--- submit into UNFOCUSED python window (the fix):"
$Q sessions send "$PY_TOKEN" "print(6*7)" --submit
$Q sessions wait "$PY_TOKEN" --expect 42 --timeout-ms 15000 > /tmp/wait-py.json
python3 -c "import json; d=json.load(open('/tmp/wait-py.json')); print('py settled:', d['settled'], 'polls:', d['polls'], 'ms:', d['elapsed_ms']); assert d['settled'], 'submit into unfocused window failed'; assert '42' in d['screen'], '42 missing from screen'"
echo "--- submit into bash with echo-exclusion:"
$Q sessions send "$SH_TOKEN" "echo telepathy-ok" --submit
$Q sessions wait "$SH_TOKEN" --expect telepathy-ok --timeout-ms 15000 > /tmp/wait-sh.json
python3 -c "import json; d=json.load(open('/tmp/wait-sh.json')); print('sh settled:', d['settled'], 'polls:', d['polls'], 'ms:', d['elapsed_ms']); assert d['settled'], 'echo-exclusion wait failed'; assert 'telepathy-ok' in d['screen']"
echo "--- read back:"
$Q sessions read "$PY_TOKEN" | tail -2
echo VERIFY_PASS
SCRIPT
)
set +e
OUT=$(qol env exec "$RUN_ID" /usr/bin/env bash -c "QOL_BIN=$QOL_BIN; $GUEST_TEST" 2>&1)
EXEC_RC=$?
set -e
echo "$OUT" | tail -12

REPORT="$REPORTS_ROOT/$RUN_ID/report.json"
mkdir -p "$(dirname "$REPORT")"
if [ "$EXEC_RC" -eq 0 ] && echo "$OUT" | grep -q VERIFY_PASS; then
    python3 - "$REPORT" "$RUN_ID" "$OUT" <<'PY'
import json, sys, time
json.dump({
    "name": "sessions-relay-verify",
    "status": "pass",
    "started_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
    "evidence": sys.argv[3][-4000:],
}, open(sys.argv[1], "w"), indent=2)
PY
    echo "VERIFY PASS — report: $REPORT"
    qol env down "$RUN_ID" >/dev/null 2>&1 || true
else
    python3 - "$REPORT" "$RUN_ID" "$OUT" <<'PY'
import json, sys, time
json.dump({
    "name": "sessions-relay-verify",
    "status": "failed",
    "started_at": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
    "evidence": sys.argv[3][-4000:],
    "next": [f"qol env exec {sys.argv[2]} /usr/bin/env bash"],
}, open(sys.argv[1], "w"), indent=2)
PY
    echo "VERIFY FAILED — guest left running: $RUN_ID"
    echo "inspect: qol env exec $RUN_ID /usr/bin/env bash"
    exit 1
fi
