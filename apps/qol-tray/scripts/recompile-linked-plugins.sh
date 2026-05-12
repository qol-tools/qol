#!/usr/bin/env bash
set -uo pipefail

# Build every sibling plugin / qol-* crate that exposes a plugin.toml so the
# tray finds a fresh target binary on launch. Mirrors the sibling glob used by
# `make clean-all` so a wipe + dev cycle leaves no missing-binary holes.
#
# Behaviour:
# - Crate without plugin.toml (lib crate, support tool): skip, it builds
#   transitively when plugins need it.
# - plugin.toml without runtime.command and without daemon.command: skip,
#   nothing executable to provide.
# - target/debug/<command> already present: skip, cargo would no-op anyway.
# - cargo build failure: report once at the end, exit 0 so the user can still
#   reach the GUI Recompile pane.

# Each plugin has its own target/ dir, so common deps (libc, objc2, serde)
# would normally recompile per plugin on a cold build. sccache caches by
# content hash across every target/, so deps compile once per machine.
if [ -z "${RUSTC_WRAPPER:-}" ] && command -v sccache >/dev/null 2>&1; then
    export RUSTC_WRAPPER=sccache
    printf 'recompile-linked: using sccache as RUSTC_WRAPPER\n'
elif [ -z "${RUSTC_WRAPPER:-}" ]; then
    printf 'recompile-linked: sccache not found - install via `brew install sccache` to dedupe deps across plugins.\n' >&2
fi

script_dir=$(cd "$(dirname "$0")" && pwd)
workspace_root=$(cd "$script_dir/../.." && pwd)

shopt -s nullglob
siblings=()
for d in "$workspace_root"/plugin-* "$workspace_root"/qol-*; do
    [ -d "$d" ] || continue
    [ -f "$d/Cargo.toml" ] || continue
    [ "$(basename "$d")" != "qol-tray" ] || continue
    siblings+=("$d")
done
shopt -u nullglob

if [ "${#siblings[@]}" -eq 0 ]; then
    printf 'recompile-linked: no siblings under %s\n' "$workspace_root"
    exit 0
fi

extract_command() {
    # Read the value of `command = "..."` from a named TOML section.
    # Stops at the next `[section]` header so we do not bleed across tables.
    local file=$1 section=$2
    awk -v want="[$section]" '
        $0 == want { in_sec=1; next }
        in_sec && /^\[/ { exit }
        in_sec && /^[[:space:]]*command[[:space:]]*=/ {
            sub(/^[^=]*=[[:space:]]*/, "", $0)
            gsub(/^"|"$/, "", $0)
            print
            exit
        }
    ' "$file"
}

failed=()
built=0
skipped=0

for plugin_dir in "${siblings[@]}"; do
    name=$(basename "$plugin_dir")
    manifest="$plugin_dir/plugin.toml"

    if [ ! -f "$manifest" ]; then
        skipped=$((skipped + 1))
        continue
    fi

    runtime_cmd=$(extract_command "$manifest" runtime)
    daemon_cmd=$(extract_command "$manifest" daemon)
    command=${runtime_cmd:-$daemon_cmd}

    if [ -z "$command" ]; then
        skipped=$((skipped + 1))
        continue
    fi

    if [ -x "$plugin_dir/target/debug/$command" ] || [ -x "$plugin_dir/target/release/$command" ]; then
        skipped=$((skipped + 1))
        continue
    fi

    printf '==> build %s (missing target/debug/%s)\n' "$name" "$command"
    if ! ( cd "$plugin_dir" && cargo build ); then
        failed+=("$name")
        continue
    fi
    built=$((built + 1))
done

printf 'recompile-linked: built %d, skipped %d, failed %d\n' \
    "$built" "$skipped" "${#failed[@]}"

if [ "${#failed[@]}" -gt 0 ]; then
    printf 'recompile-linked: failed plugins: %s\n' "${failed[*]}" >&2
    printf 'recompile-linked: continuing - recover via qol-tray GUI Recompile pane.\n' >&2
fi

exit 0
