# Session Handoff

## Current Issue: move-monitor-left/right broken for snapped windows

### Problem
`move-monitor-left.sh` and `move-monitor-right.sh` in `plugin-window-actions` have issues:
1. **Snapped windows don't move correctly** - When a window is "snapped" (tiled) to left/right side of a monitor using Cinnamon's tiling, moving it to another monitor fails visually
2. **Works fine for**: centered windows, maximized windows, freely positioned windows
3. **Fails for**: windows snapped to left or right edge

### User's Setup
- **OS**: Linux Mint Cinnamon
- **Monitors**: 
  - DP-0: 1920x1080 at +0+360 (left, smaller)
  - HDMI-0: 2560x1440 at +1920+0 (right, larger, primary)

### What Was Fixed This Session
1. **Y-axis drift** - Fixed by using `_NET_FRAME_EXTENTS` to account for window decorations
2. **Monitor sort order** - Changed from `-k3` to `-k2` to sort by X position, not Y
3. **Proportional positioning** - Center-based positioning works correctly
4. **Proportional sizing** - Window scales proportionally between monitors

### What Was Tried (All Failed for Snapped Windows)

| Approach | Result |
|----------|--------|
| `wmctrl -ir $win -b remove,maximized_vert,maximized_horz` before move | No effect |
| `xprop -id $win -remove _NET_WM_STATE` to clear all states | No effect |
| Clear state BEFORE reading geometry, with 20ms sleep | No effect |
| `xdotool windowmove --sync` | Made it worse |
| `wmctrl -e` for atomic move+resize | Same issue |
| Move to far position first (3000,500) then to target | Made it much worse |
| Various orderings of resize/move operations | No improvement |

### Current Script Logic (move-monitor-right.sh)
```bash
1. Get active window
2. Clear tiled state (wmctrl + xprop)
3. Sleep 20ms
4. Read frame extents (_NET_FRAME_EXTENTS)
5. Read geometry (xwininfo)
6. Calculate visual position (subtract frame offsets)
7. Find current monitor by center point
8. Calculate proportional new position and size
9. xdotool windowsize + windowmove
```

### Key Files
- `~/.config/qol-tray/plugins/plugin-window-actions/scripts/move-monitor-left.sh`
- `~/.config/qol-tray/plugins/plugin-window-actions/scripts/move-monitor-right.sh`
- Git repo: `/media/kmrh47/WD_SN850X/Git/qol-tools/plugin-window-actions/`

### Theories Not Yet Tested
1. **Cinnamon-specific tiling API** - Maybe there's a dbus call or Cinnamon-specific way to untile
2. **Window constraints** - Cinnamon might be constraining window to monitor when tiled
3. **Muffin (Cinnamon's WM) specific behavior** - May need Muffin-specific commands
4. **Read geometry from wmctrl -lG** - Might report different coords when tiled
5. **Use xdotool getwindowgeometry instead of xwininfo** - Different coordinate reporting

### What Works Perfectly
- `minimize.sh` - Instant, uses `xdotool windowminimize`
- `restore.sh` - Instant, uses `wmctrl -ia`
- `snap-left.sh`, `snap-right.sh` - Work fine
- `center.sh` - Works fine
- `maximize.sh` - Works fine
- Moving **centered** windows between monitors - Works perfectly with size scaling
- Moving **maximized** windows between monitors - Works fine

### Debug Command
Add this to scripts to see what's happening:
```bash
echo "DEBUG: visual_x=$visual_x visual_y=$visual_y visual_w=$visual_w visual_h=$visual_h" >&2
echo "DEBUG: monitors=${monitors[*]}" >&2
echo "DEBUG: current_idx=$current_idx target_idx=$target_idx" >&2
echo "DEBUG: new_x=$new_x new_y=$new_y new_w=$new_w new_h=$new_h" >&2
```

---

## Previous Session Summary

### 1. README Update
Updated README.md to reflect current architecture.

### 2. Created plugin-window-actions
New plugin for window management with 9 actions.
Repo: https://github.com/qol-tools/plugin-window-actions

### 3. Plugin Manifest Fetching Fix
`github.rs` now tries both `main` and `master` branches.

### 4. Hotkey Execution Fix
`src/hotkeys/mod.rs` now passes action ID as first argument to `run.sh`.

### 5. Hotkey Modal UX Improvements
Major refactor of `ui/views/hotkeys.js`.

## Notes
- Window actions use `xdotool`, `wmctrl`, `xrandr`, `xprop` — X11 only
- Cinnamon uses Muffin as its window manager (fork of Mutter)
- The issue is specifically with Cinnamon's tiling/snapping feature
