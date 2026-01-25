# GPUI Test

Minimal gpui spike to validate it works on Linux.

## Linux Dependencies (Ubuntu/Debian)

```bash
sudo apt install gcc g++ libasound2-dev libfontconfig-dev libwayland-dev \
    libx11-xcb-dev libxkbcommon-x11-dev libssl-dev libzstd-dev libvulkan1 \
    libgit2-dev make cmake clang mold libstdc++-14-dev
```

Adjust `libstdc++-14-dev` based on your Ubuntu version:
- Ubuntu 24.04+: `libstdc++-14-dev`
- Ubuntu 22.04: `libstdc++-12-dev`
- Ubuntu 20.04: `libstdc++-10-dev`

## Build & Run

```bash
cargo run
```

## What This Tests

1. 42px tall borderless window (proves no min height constraint)
2. Catppuccin-style dark background
3. Escape key to quit
4. Basic text rendering

## Expected Result

A small floating bar appears center-top of screen with "Type to search..." text.
Press Escape to close.
