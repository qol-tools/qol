#!/usr/bin/env bash
set -euo pipefail

BIN_DIR="gpui-prototype/gpui-test/src/bin"

bins=()
while IFS= read -r f; do
    bins+=("$(basename "$f" .rs)")
done < <(ls -1 "$BIN_DIR"/*.rs 2>/dev/null | sort)

if [ ${#bins[@]} -eq 0 ]; then
    echo "No bin tests found in $BIN_DIR"
    exit 1
fi

printf '\n  \033[1;36mGPUI Bin Tests\033[0m\n\n'
for i in "${!bins[@]}"; do
    printf '  \033[33m%2d\033[0m) %s\n' "$((i + 1))" "${bins[$i]}"
done

printf '\n  Pick [1-%d]: ' "${#bins[@]}"
read -r choice

if ! [[ "$choice" =~ ^[0-9]+$ ]] || [ "$choice" -lt 1 ] || [ "$choice" -gt "${#bins[@]}" ]; then
    echo "Invalid choice: $choice"
    exit 1
fi

selected="${bins[$((choice - 1))]}"
printf '\n  \033[1;32m▶\033[0m Running \033[1m%s\033[0m\n\n' "$selected"
cd gpui-prototype/gpui-test && exec cargo run --bin "$selected"
