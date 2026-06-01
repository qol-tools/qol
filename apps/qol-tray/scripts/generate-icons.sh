#!/usr/bin/env bash
set -euo pipefail

SRC="assets/icon.png"
OUT="assets/icons"

resize() {
    local src="$1" size="$2" dst="$3"
    if command -v convert >/dev/null 2>&1; then
        convert "$src" -filter point -resize "${size}x${size}" "$dst"
    else
        python3 - "$src" "$size" "$dst" <<'EOF'
import sys
from PIL import Image
img = Image.open(sys.argv[1]).convert("RGBA")
img = img.resize((int(sys.argv[2]), int(sys.argv[2])), Image.NEAREST)
img.save(sys.argv[3])
EOF
    fi
}

mkdir -p "$OUT"

resize "$SRC" 64  "$OUT/64.png"
resize "$SRC" 128 "$OUT/128.png"
resize "$SRC" 256 "$OUT/256.png"

ICNS_DIR=$(mktemp -d)
trap 'rm -rf "$ICNS_DIR"' EXIT

for size in 32 128 256 512; do
    resize "$SRC" "$size" "$ICNS_DIR/${size}.png"
done

if command -v png2icns >/dev/null 2>&1; then
    png2icns assets/qol-tray.icns \
        "$ICNS_DIR/32.png" \
        "$ICNS_DIR/128.png" \
        "$ICNS_DIR/256.png" \
        "$ICNS_DIR/512.png"
elif command -v icnsutil >/dev/null 2>&1; then
    icnsutil compose assets/qol-tray.icns \
        "$ICNS_DIR/32.png" \
        "$ICNS_DIR/128.png" \
        "$ICNS_DIR/256.png" \
        "$ICNS_DIR/512.png"
else
    echo "Warning: neither png2icns nor icnsutil found, skipping .icns generation"
    echo "Install: sudo apt install libicns-utils OR pipx install icnsutil"
fi
