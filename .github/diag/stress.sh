#!/usr/bin/env bash
set -uo pipefail

tray_bin="$1"
process_bin="$2"
label="$3"
window_iterations="${WINDOW_ITERATIONS:-5000}"
process_copies="${PROCESS_COPIES:-12}"
process_iterations="${PROCESS_ITERATIONS:-300}"
tray_iterations="${TRAY_ITERATIONS:-10000}"
tray_parallel="${TRAY_PARALLEL:-24}"

cgroup_mount=/sys/fs/cgroup
cgroup_path=$(awk -F: '$1 == "0" { print $3; exit }' /proc/self/cgroup)
cgroup_origin="$cgroup_mount$cgroup_path"
cgroup_root="$cgroup_origin/qol-diag-${GITHUB_RUN_ID}-${label}-$$"
journal_root="$RUNNER_TEMP/qol-process-cgroups-diag-${label}-$$"
mkdir -m 700 "$journal_root"
sudo mkdir "$cgroup_root"
sudo chown "$(id -u):$(id -g)" \
  "$cgroup_root" \
  "$cgroup_root/cgroup.procs" \
  "$cgroup_root/cgroup.subtree_control" \
  "$cgroup_root/cgroup.threads"
printf '%s\n' "$$" | sudo tee "$cgroup_root/cgroup.procs" >/dev/null
export QOL_PROCESS_CGROUP_ROOT="$cgroup_root"
export QOL_PROCESS_CGROUP_JOURNAL_ROOT="$journal_root"

out="$RUNNER_TEMP/diag-$label"
mkdir -p "$out/tray"
summary="$out/summary.md"
: >"$summary"
note() { printf '%s\n' "$*" | tee -a "$summary"; }

note "## $label"
note '```'
note "kernel: $(uname -r), cpus: $(nproc)"

hogs=()
start_hogs() {
  for _ in $(seq 1 "$(nproc)"); do
    (while :; do :; done) &
    hogs+=($!)
  done
}
stop_hogs() {
  kill "${hogs[@]}" 2>/dev/null
  wait "${hogs[@]}" 2>/dev/null
  hogs=()
}

start_hogs
probes=()
for i in 1 2 3 4; do
  python3 .github/diag/exit_window_probe.py "$window_iterations" >"$out/window-$i.txt" 2>&1 &
  probes+=($!)
done
wait "${probes[@]}"
for i in 1 2 3 4; do
  note "window probe $i: $(head -1 "$out/window-$i.txt")"
done
grep -h '^  ' "$out"/window-*.txt | sort | uniq -c | sort -rn | head -8 | while IFS= read -r line; do note "  $line"; done

copies=()
for i in $(seq 1 "$process_copies"); do
  QOL_DIAG_STRESS_ITERATIONS="$process_iterations" "$process_bin" --exact \
    bounded_output::guarded::tests::diag_fast_root_stress --nocapture >"$out/process-$i.log" 2>&1 &
  copies+=($!)
done
failed_copies=0
for pid in "${copies[@]}"; do
  wait "$pid" || failed_copies=$((failed_copies + 1))
done
process_failures=$(grep -h '^error kind\|^completed\|^timed out' "$out"/process-*.log | wc -l)
note "qol-process guarded runs of sh -c 'echo ok': $((process_copies * process_iterations)) runs, $process_failures failed ($failed_copies of $process_copies copies reported failures)"
grep -h '^error kind\|^completed\|^timed out' "$out"/process-*.log \
  | sed -E 's/PID [0-9]+/PID N/g; s/qol-process-v1-[^ ")]*/qol-process-v1-X/g; s/flags=0x[0-9a-f]+/flags=F/g' \
  | sort | uniq -c | sort -rn | head -8 | while IFS= read -r line; do note "  $line"; done
grep -h -m1 '^error kind' "$out"/process-*.log | head -3 | while IFS= read -r line; do note "  example: $line"; done

stop=$out/stop-hang
hangers=()
for _ in 1 2; do
  (
    while [ ! -e "$stop" ]; do
      "$tray_bin" --exact \
        features::gpu_driver_sync::platform::linux::tests::a_hanging_modinfo_probe_is_bounded_and_leaves_no_child \
        --nocapture >/dev/null 2>&1
    done
  ) &
  hangers+=($!)
done
started=$(date +%s)
seq 1 "$tray_iterations" | xargs -P "$tray_parallel" -n 1 .github/diag/run_tray_once.sh "$tray_bin" \
  features::gpu_driver_sync::platform::linux::tests::a_working_modinfo_probe_returns_the_version "$out/tray"
elapsed=$(($(date +%s) - started))
touch "$stop"
wait "${hangers[@]}"
stop_hogs
tray_failures=$(find "$out/tray" -name '*.log' | wc -l)
note "qol-tray a_working_modinfo_probe_returns_the_version: $tray_iterations runs in ${elapsed}s at -P $tray_parallel, $tray_failures failed"
cat "$out"/tray/*.log 2>/dev/null | grep -h 'panicked\|modinfo probe failed\|left:\|right:' \
  | sed -E 's/PID [0-9]+/PID N/g; s/\([0-9]+\) panicked/(T) panicked/; s/qol-process-v1-[^ ")]*/qol-process-v1-X/g; s/flags=0x[0-9a-f]+/flags=F/g' \
  | sort | uniq -c | sort -rn | head -10 | while IFS= read -r line; do note "  $line"; done
find "$out/tray" -name '*.log' | head -2 | while IFS= read -r log; do
  grep -h 'modinfo probe failed' "$log" | head -1 | while IFS= read -r line; do note "  example: $line"; done
done
note '```'
cat "$summary" >>"$GITHUB_STEP_SUMMARY"
