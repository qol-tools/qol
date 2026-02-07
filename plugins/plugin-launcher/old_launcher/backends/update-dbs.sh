#!/bin/bash
set -euo pipefail

cache_dir="$HOME/.cache/qol-launcher-dbs"
mkdir -p "$cache_dir"

for dir in /media/*/; do
    [[ -d "$dir" ]] || continue
    hash=$(echo -n "$dir" | md5sum | cut -d' ' -f1)
    db_path="$cache_dir/$hash.db"
    echo "Indexing $dir -> $db_path"
    updatedb -l 0 -U "$dir" -o "$db_path" 2>/dev/null || true
done

echo "Done. Databases in $cache_dir:"
ls -la "$cache_dir"
