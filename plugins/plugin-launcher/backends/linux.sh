#!/bin/bash
set -euo pipefail

query="$1"
[[ -z "$query" ]] && exit 0

cache_dir="$HOME/.cache/qol-launcher-dbs"
mkdir -p "$cache_dir"

db_args=()
for dir in /media/*/; do
    [[ -d "$dir" ]] || continue
    db_path="$cache_dir/$(echo -n "$dir" | md5sum | cut -d' ' -f1).db"
    [[ -f "$db_path" ]] && db_args+=("-d" "$db_path")
done

search_plocate() {
    local pattern="$1" limit="$2"
    { [[ ${#db_args[@]} -gt 0 ]] && plocate -i -l "$limit" "${db_args[@]}" "$pattern" 2>/dev/null || true
      plocate -i -l "$limit" "$pattern" 2>/dev/null || true
    } | { grep -v -E "^/timeshift/|/app-install/|^/mnt/" || true; } | awk '!seen[$0]++'
}

search_desktop_dirs() {
    local dirs=(
        "/usr/share/applications"
        "/usr/lib"
        "$HOME/.local/share/applications"
        "/var/lib/flatpak/exports/share/applications"
    )
    for d in "${dirs[@]}"; do
        find "$d" -maxdepth 3 -iname "*${query}*.desktop" 2>/dev/null || true
    done
}

{
    search_desktop_dirs
    search_plocate "*${query}*.desktop" 30
    search_plocate "*$query*" 200
} | awk '!seen[$0]++' | head -n 50
