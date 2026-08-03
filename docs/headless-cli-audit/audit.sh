#!/usr/bin/env bash
# Headless-CLI coverage audit for the qol-monorepo.
#
# For every release unit it reports whether the unit is headless-CLI at its
# base, per the qol-arch-code "headless-first feature shape" contract:
#
#   - standalone binary with a [runtime] command declared in its manifest
#   - a qol-headless CLI surface (help / doctor / --json)
#   - host actions mapped to CLI command argv
#   - optional daemon mode layered on top, never replacing the CLI
#
# Exit 0 when coverage is 100%, 1 otherwise (CI-gateable).
#
# Usage: docs/headless-cli-audit/audit.sh [--json]
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel)"
cd "$ROOT"

JSON=0
[ "${1:-}" = "--json" ] && JSON=1

failures=0
units=0

report_unit() {
    # report_unit <kind> <name> <headless> <detail>
    local kind="$1" name="$2" headless="$3" detail="$4"
    units=$((units + 1))
    if [ "$headless" != "yes" ]; then
        failures=$((failures + 1))
    fi
    if [ "$JSON" = "1" ]; then
        printf '{"kind":"%s","name":"%s","headless":"%s","detail":"%s"}\n' \
            "$kind" "$name" "$headless" "$detail"
    else
        printf '%-8s %-28s %-4s %s\n' "$kind" "$name" "$headless" "$detail"
    fi
}

check_plugin() {
    local dir="$1" id
    id="$(basename "$dir")"
    local toml="$dir/plugin.toml"
    [ -f "$toml" ] || { report_unit plugin "$id" no "missing plugin.toml"; return; }

    local runtime doctor daemon cli_headless qh
    runtime="$(grep -c '^\[runtime\]' "$toml" || true)"
    doctor="$(grep -c '^doctor = true' "$toml" || true)"
    daemon="$(grep -c '^\[daemon\]' "$toml" || true)"
    qh="$(grep -c 'qol-headless' "$dir/Cargo.toml" || true)"
    cli_headless="no"
    if [ -f "$dir/src/cli.rs" ] || [ -d "$dir/src/cli" ] || \
       [ -f "$dir/src/runtime/cli/mod.rs" ] || \
       grep -rq 'qol_headless::HeadlessApp' "$dir/src" 2>/dev/null; then
        cli_headless="yes"
    fi

    local detail="runtime=$runtime doctor=$doctor daemon=$daemon headless_app=$qh"
    if [ "$runtime" -ge 1 ] && [ "$doctor" -ge 1 ] && [ "$cli_headless" = "yes" ]; then
        report_unit plugin "$id" yes "$detail"
    else
        report_unit plugin "$id" no "$detail"
    fi
}

check_bin() {
    # check_bin <kind> <name> <crate-dir>
    local kind="$1" name="$2" dir="$3"
    local qh
    qh="$(grep -c 'qol-headless' "$dir/Cargo.toml" || true)"
    if [ "$qh" -ge 1 ]; then
        report_unit "$kind" "$name" yes "qol-headless"
    else
        report_unit "$kind" "$name" no "no qol-headless dep"
    fi
}

for p in plugins/*/; do
    check_plugin "$p"
done

check_bin tool qol "$(pwd)/tools/qol-cli"
check_bin tool qol-guest-runner "$(pwd)/tools/qol-guest-runner"

for bin in qol-tray qol-tray-install qol-tray-doctor qol-tray-migrate; do
    check_bin app "$bin" "$(pwd)/apps/qol-tray"
done

if [ "$JSON" = "1" ]; then
    printf '{"summary":{"units":%d,"failures":%d}}\n' "$units" "$failures" >&2
else
    printf '%d units, %d not headless\n' "$units" "$failures"
fi

[ "$failures" -eq 0 ]
