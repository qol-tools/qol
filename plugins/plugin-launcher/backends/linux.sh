#!/bin/bash
set -euo pipefail

query="$1"
limit=50
cache_dir="$HOME/.cache/qol-launcher-dbs"

if [[ -z "$query" ]]; then
    exit 0
fi

mkdir -p "$cache_dir"

read -ra words <<< "$query"
first_word="${words[0]}"

db_args=()
for dir in /media/*/; do
    [[ -d "$dir" ]] || continue
    hash=$(echo -n "$dir" | md5sum | cut -d' ' -f1)
    db_path="$cache_dir/$hash.db"
    [[ -f "$db_path" ]] && db_args+=("-d" "$db_path")
done

search_and_filter() {
    local results
    results=$(
        {
            [[ ${#db_args[@]} -gt 0 ]] && plocate --ignore-case --limit 200 "${db_args[@]}" "$first_word" 2>/dev/null || true
            plocate --ignore-case --limit 200 "$first_word" 2>/dev/null || true
        } | awk '!seen[$0]++'
    )

    if [[ ${#words[@]} -le 1 ]]; then
        echo "$results"
        return
    fi

    for word in "${words[@]:1}"; do
        results=$(echo "$results" | grep -i "$word" || true)
    done
    echo "$results"
}

prioritize_results() {
    local desktop=()
    local executable=()
    local other=()

    while IFS= read -r path; do
        [[ -z "$path" ]] && continue
        if [[ "$path" == *.desktop ]]; then
            desktop+=("$path")
        elif [[ -x "$path" && -f "$path" ]]; then
            executable+=("$path")
        else
            other+=("$path")
        fi
    done

    printf '%s\n' "${desktop[@]}" "${executable[@]}" "${other[@]}"
}

search_and_filter | prioritize_results | head -n "$limit"
