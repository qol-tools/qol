# Session Handoff

## Current State

Screen recording plugin for qol-tray using ffmpeg.

## Known Issues / TODO

1. **Wayland support** - Uses X11-only tools:
   - `xrandr` for monitor detection
   - `slop` for region selection

   Wayland alternatives:
   - Monitor detection: `wlr-output-management` or `xdg-output` protocols
   - Region selection: `slurp` (Wayland equivalent of slop)
   - Screen capture: PipeWire with `xdg-desktop-portal`

## Dependencies

- ffmpeg
- slop
- xrandr
- jq
