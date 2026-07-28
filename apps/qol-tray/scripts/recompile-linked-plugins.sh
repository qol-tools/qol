#!/usr/bin/env bash
set -uo pipefail

script_dir=$(cd "$(dirname "$0")" && pwd)
workspace_root=$(cd "$script_dir/../.." && pwd)

shopt -s nullglob
siblings=()
for d in "$workspace_root"/*; do
    [ -d "$d" ] || continue
    [ -f "$d/Cargo.toml" ] || continue
    [ -f "$d/plugin.toml" ] || continue
    siblings+=("$d")
done
shopt -u nullglob

if [ "${#siblings[@]}" -eq 0 ]; then
    printf 'recompile-linked: no siblings under %s\n' "$workspace_root"
    exit 0
fi

extract_command() {
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

extract_platforms() {
    local file=$1
    awk '
        $0 == "[plugin]" { in_sec=1; next }
        in_sec && /^\[/ { exit }
        in_sec && /^[[:space:]]*platforms[[:space:]]*=/ {
            sub(/^[^=]*=[[:space:]]*/, "", $0)
            gsub(/[][",]/, " ", $0)
            print
            exit
        }
    ' "$file"
}

case "$(uname -s)" in
    Linux)  host_os=linux ;;
    Darwin) host_os=macos ;;
    MINGW*|MSYS*|CYGWIN*) host_os=windows ;;
    *) host_os=unknown ;;
esac

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

    platforms=$(extract_platforms "$manifest")
    if [ -n "$platforms" ] && ! printf ' %s ' $platforms | grep -q " $host_os "; then
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
