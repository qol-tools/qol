# Session Handoff

## What Was Done This Session

### 1. README Update
Updated README.md to reflect current architecture:
- Tray menu now minimal: just "Plugins" and "Quit"
- All plugin interaction happens in browser UI at `http://127.0.0.1:42700`
- Documented plugin store discovery via `qol-tray-plugin` topic
- Split action types into separate table

### 2. Created plugin-window-actions
New plugin for window management with 9 actions:
- `snap-left`, `snap-right`, `snap-bottom`
- `center` (1152x892, centered on current monitor)
- `maximize`, `minimize`, `restore` (LIFO from WM stacking order)
- `move-monitor-left`, `move-monitor-right` (wraps around)

Structure:
```
plugin-window-actions/
├── plugin.toml
├── run.sh              # dispatcher
└── scripts/
    ├── lib.sh          # shared monitor detection
    └── *.sh            # action scripts
```

All scripts have multi-monitor support with proportional scaling when monitors differ in resolution.

Repo: https://github.com/qol-tools/plugin-window-actions

### 3. Plugin Manifest Fetching Fix
`github.rs` now tries both `main` and `master` branches when fetching `plugin.toml`:
- Fixes plugins that use `master` as default branch
- Falls back gracefully if neither exists

### 4. Hotkey Execution Fix
`src/hotkeys/mod.rs` now passes action ID as first argument to `run.sh`:
- Was: `bash run.sh`
- Now: `bash run.sh <action-id>`

### 5. Hotkey Modal UX Improvements
Major refactor of `ui/views/hotkeys.js`:

**Field order changed**: Plugin → Action → Shortcut (logical flow)

**Modal stays open** after saving new hotkey:
- Shortcut field clears
- Already-assigned actions filtered from dropdown
- Closes automatically when all actions assigned

**Removed enabled toggle**:
- Hotkeys are always enabled if they exist
- Delete to disable
- Removed status column from list view

**Keyboard shortcuts**:
- Enter on shortcut field → starts recording
- Enter on other fields → advances to next field
- Ctrl+Enter anywhere → saves
- Esc → cancels

## Current State

- README: **Updated**
- plugin-window-actions: **Working** (installed, hotkeys bindable)
- Manifest fetching: **Fixed** (main/master fallback)
- Hotkey execution: **Fixed** (action ID passed)
- Hotkey modal UX: **Improved**

## Key Files Changed

| File | Changes |
|------|---------|
| `README.md` | Updated to reflect browser-based UI architecture |
| `src/features/plugin_store/github.rs` | Try main then master for plugin.toml |
| `src/hotkeys/mod.rs` | Pass action ID to run.sh |
| `ui/views/hotkeys.js` | Modal UX overhaul, removed enabled toggle |

## New Plugin Repo

| Repo | Description |
|------|-------------|
| `qol-tools/plugin-window-actions` | Window snapping, centering, multi-monitor |

## Notes

- Window actions use `xdotool`, `wmctrl`, `xrandr`, `xprop` — X11 only
- Monitor detection finds which monitor contains window center point
- Move-to-monitor scales window position/size proportionally
