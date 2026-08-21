# GPUI user-visible strings

Inventory of string literals that reach the user through gpui surfaces: settings panels, toasts, pickers, overlays, and window titles. Generated 2026-08-21 by scanning every crate that imports gpui (`use gpui` / `use qol_gpui`), excluding `examples/`, `tests/`, and `#[cfg(test)]` blocks, then hand-curating out trace probes, X11 atom names, element ids, and log/assert text.

Per-crate counts: qol-gpui 12, qol-shot 30, alt-tab 14, removeapp 14, cli-sessions 8, launcher 8, qol-tray settings surface 3, plus shared key-cap fragments.

## Window and panel titles

| String | Location |
|---|---|
| `{} Settings` (generic plugin settings window) | apps/qol-tray/src/settings_surface/platform/unix_common.rs:561 |
| `qol Settings` | apps/qol-tray/src/settings_surface/platform/unix_common.rs:538 |
| `Alt Tab Settings` | plugins/alt-tab/src/picker/run.rs:181 |
| `Alt Tab` | plugins/alt-tab/src/app/render.rs:189 |
| `Alt Tab · Live Window Grid` | plugins/alt-tab/src/app/render.rs:196 |
| `Launcher Settings` | plugins/launcher/src/ui/run.rs:129 |
| `QoL Shot Settings` | plugins/qol-shot/src/ui/settings_panel.rs:8 |
| `QoL Shot Editor` | plugins/qol-shot/src/ui/editor/mod.rs:224 |
| `CLI SESSIONS` | plugins/cli-sessions/src/ui/render.rs:190 |

## qol-gpui shared kit

| String | Location | Used for |
|---|---|---|
| `Save` | settings_panel/view.rs:2947 | panel footer button |
| `+ Add` | settings_panel/view.rs:2692 | list add button |
| `On` / `Off` | settings_panel/view.rs:1872 | toggle row |
| `No matching results.` | settings_panel/view.rs:2429 | options filter empty state |
| `Unsupported: {reason}` | settings_panel/view.rs:1812 | unsupported field row |
| `{} found` / `{visible}/{}` | settings_panel/view.rs:1691,1694 | list count footer |
| `{count} items` | settings_panel/view.rs:3844 | list count |
| `{primary} +{}` | settings_panel/view.rs:3794 | shortcut display |
| `action `{action}` is unavailable` | settings_panel/mod.rs:225 | settings action error |
| `{edit}_` / `{text}_` | settings_panel/view.rs:1620,2881 | in-edit text cursor suffix |
| `{step}` / `{value:.0}` / `#{}` / `#{value}` | settings_panel/view.rs:3610,3668,3678,246 | slider readouts |
| `{}` | settings_panel/view.rs:3670 | slider value fallback |
| `px` / `ms` / `{:.0}%` | settings_panel/view.rs:3694,3697,3734 | unit suffixes |
| `true` / `false` | settings_panel/view.rs:3762 | read-only toggle value |
| `{r:02x}{g:02x}{b:02x}` / `#{:06x}` | color_wheel.rs:454, settings_panel/view.rs:1570 | hex color field |
| `{:.2}` / `{:+.2}` | gamepad/view.rs:302,364 | stick/trigger values |
| `{} of {} · Enter to switch` / `Live native input` | gamepad/view.rs:102,107 | device selector |
| `Waiting for movement` | gamepad/view.rs:215 | calibration hint |
| `ACTIVE INPUTS` | gamepad/view.rs:234 | panel section |
| `Wake a controller` / `Controller input unavailable` | gamepad/view.rs:418,420 | empty state |
| `Left ` / `L ` / `Right ` / `R ` / `D-pad ` / `D ` | gamepad/view.rs:369-371 | button name prefix + abbreviation |

## alt-tab

| String | Location |
|---|---|
| `W close · Q quit · R minimize · ↑↓←→ navigate · ⏎ switch · esc close` | app/render.rs:190 hint bar |
| `Scanning windows...` | app/render.rs:326 empty state |
| `{app} · {title}` / `[{}] {}` | app/render.rs:540,531 window card label |
| `...` | app/render.rs:647 truncated label |

## cli-sessions overview

| String | Location |
|---|---|
| `No CLI sessions found` / `Open a CLI in kitty, then open this panel again.` | ui/render.rs:233,239 empty state |
| `{summary} \u{2713}` | ui/render.rs:366 completed marker |
| `\u{21C4} {count}` / `\u{21C4}` | ui/render.rs:416,447 attention marker |
| `\u{25B2}` | ui/render.rs:587 needs-you marker |
| `{secs}s` / `{}m` / `{}h` | ui/render.rs:39-43 session age |

## launcher

| String | Location |
|---|---|
| `Type to search...` | ui/view.rs:116 placeholder |
| `+{boost}` | ui/view.rs:394 frecency boost chip |
| `{selected_index}/{result_count}` | ui/view.rs:299 position readout |
| `[{}{}]` | ui/view.rs:358 mode/key chips |
| `Open in Terminal` / `Open Folder` / `Copy Path` | lib.rs:46-48 result actions |
| `desktop` | discovery/mod.rs:156 desktop entry kind |

## qol-shot

Toasts (app/mod.rs unless noted):

| String | Location |
|---|---|
| `Could not capture the selected area` | app/mod.rs:253 |
| `Could not save the selected area` | app/mod.rs:282 |
| `Screenshot failed` | app/mod.rs:312 |
| `Recording starts in {seconds}` / `Get ready` | app/mod.rs:379,380 countdown |
| `Recording stopped` | app/mod.rs:632 |
| `Saving recording…` | app/mod.rs:633 |
| `Recording saved` | app/mod.rs:642 |
| `Save delayed` / `The recorder is still finalizing the file` | app/mod.rs:650,651 |
| `Recording cancelled` / `No video was captured` | app/mod.rs:697,698 |
| `Recording not started` / `The countdown could not close safely` | app/mod.rs:716,717 |
| `Screenshot updated` | ui/editor/mod.rs:537 |
| `Copying edited screenshot…` / `Copying screenshot path…` / `Opening screenshot folder…` | ui/editor/mod.rs:118-120 |
| `Could not save screenshot` / `Could not copy edited screenshot` / `Could not copy screenshot path` / `Could not open screenshot folder` | ui/editor/mod.rs:126-129 |
| `Could not open screenshot editor` | ui/preview.rs:895,909 |
| `failed to save frozen screenshot: {}` | capture/frozen_frame.rs:373 |
| `failed to read screenshot dimensions: {}` / `failed to read image dimensions: {}` / `failed to read preview image: {}` / `failed to prepare preview image: {}` | ui/editor/mod.rs:203, ui/preview.rs:608,635,639 |

Overlay hints:

| String | Location |
|---|---|
| `Detected window` / `Full monitor` | ui/region_selector/mod.rs:62,64 target chip |
| `Release mouse to capture` | ui/region_selector/mod.rs:914 |
| `No window detected` | ui/region_selector/mod.rs:920 |
| `Press Esc to cancel` | ui/region_selector/mod.rs:925 |
| `Click to capture or drag to select area` | ui/region_selector/mod.rs:928 |
| `Drag to select area or press Esc to cancel` | ui/region_selector/mod.rs:930 |
| `Capture area` | ui/region_selector/mod.rs:1000,1005 |
| `{label} · {free} free · ~{}` / `{label} · {free} free` | ui/region_selector/mod.rs:969,970 |
| `Drag to draw · Ctrl+Z undo · Ctrl+C copies & closes · Esc cancels` | ui/editor/render.rs:125 |
| `Edit screenshot` | ui/editor/render.rs:147 |
| `{:.1} GB` / `{} MB` / `{} KB` | ui/region_selector/mod.rs:1368-1372 |
| `{} h {} min` / `{} min` / `{} sec` | ui/region_selector/mod.rs:1378-1382 |
| `{:.0},{:.0}` | ui/preview.rs:312 cursor coords |

Windows doctor text (plugins/qol-shot/src/platform/windows/mod.rs): full-sentence messages at 181-206, short `qol-shot: ...` errors at 38-127.

## removeapp

| String | Location |
|---|---|
| `Type to search apps` | ui/mod.rs:360 placeholder |
| `Uninstall with {}` | ui/mod.rs:437 footer |
| `Move to Trash` / `PERMANENTLY DELETE` | ui/mod.rs:440,441 footer buttons |
| `uninstall with Homebrew` / `uninstall with APT` / `uninstall with Flatpak` | ui/mod.rs:451-453 detail |
| `Checking package manager\u{2026}` | ui/mod.rs:515 |
| `still running - couldn't quit; press Q to retry` | ui/mod.rs:522 |
| `is running - press Q to quit first` | ui/mod.rs:524 |
| `{}-managed - Enter uninstalls through {}` | ui/mod.rs:539 |
| `couldn't confirm package ownership - {reason}` | ui/mod.rs:550 |
| `Removal failed` | ui/mod.rs:578 |
| `Removed {removed} item(s)` | ui/mod.rs:612 |
| `Freed {}` | ui/mod.rs:618 |
| `{failed} failed` | ui/mod.rs:625 |
| `Enter to continue \u{00b7} Esc to quit` | ui/mod.rs:590,632 |
| `protected` | ui/mod.rs:730 chip |
| `Remove {title}` | ui/mod.rs:751 |
| `{:.1} GB` / `{:.1} MB` / `{:.0} KB` / `{bytes} B` | ui/mod.rs:819-825 |

## Consistency issues worth fixing

Status as of 2026-08-21, after the cleanup pass (commits 35a95cf1b, d2239d9fa, 193142952).

**Fixed**

1. Ellipsis style. Every user-visible string now uses the real ellipsis escape. The first pass covered only the two sites listed in this inventory; a follow-up caught the shared settings kit (`qol-gpui/src/settings_panel/view.rs` loading / Waiting / working) and qol-shot's platform recording notifications, which this inventory had missed.
2. Escape key cap. Named keys read `Esc` everywhere. Two more sites turned up beyond the ones listed here: alt-tab's second (debug-overlay) header bar, and removeapp's `("esc", "back")` hint, so the claim that removeapp was already capitalized was wrong. Single-letter caps (`d`, `T`, `a`) keep their case: it tells you which key to press.
3. `Capture area` duplication, folded into `CAPTURE_AREA_LABEL`.
4. `Could not open screenshot editor` duplication, folded into `EDITOR_OPEN_FAILED_TOAST`.
5. The `Enter to continue` / `Esc to quit` hint duplication, folded into `CONTINUE_OR_QUIT_HINT`.
7. Size formatting. This was not a precision difference: qol-shot was decimal, removeapp was 1024-based while labelling the result GB/MB/KB, so removeapp reported 90.4 GB for what Finder calls 97.0 GB. Both now call one `qol_gpui::format_bytes`, decimal, which also rolls 999_999 bytes up to `1.0 MB` instead of showing `1000 KB`. `qol-dev-build` has its own binary formatter, correctly labelled GiB/MiB/KiB; that is a different unit system and stays.

**Rejected**

6. Not a mismatch. `Uninstall with {}` is a standalone button label; `uninstall with Homebrew` is a hint description sitting beside `quit app` and `confirm`, which are lowercase too.
9. Shared key-cap constants. The literals are match-arm patterns across roughly 13 files, no two of which disagree about a key mapping. Replacing them changes nothing the user sees, prevents no bug, and a lowercase const in a pattern silently becomes a catch-all binding rather than a comparison.

**Left alone deliberately**

8. `QoL Shot Settings` versus `Alt Tab Settings`: plugin title casing follows the plugin name, and qol-tray's generic path produces `{} Settings`. Nothing to change; keep the pattern when new surfaces appear.
10. The repeated `Run QoL Shot on Linux or macOS.` and the mechanical `is not implemented on Windows` series could come from one template, but it is Windows-only text in a plugin that does not run on Windows.
11. Bare `screenshot` / `preview` / `record` in qol-shot `app/mod.rs` are trace strings, not user-visible.

## Excluded from inventory

- Trace probes and `qol_runtime::probe!` messages (`title={title} phase=...`, `context={context} stage=...`, GHOSTDUMP lines).
- X11 atom names (`_NET_WM_STATE`, `_NET_WM_WINDOW_TYPE`, `_MOTIF_WM_HINTS`, `WM_PROTOCOLS`).
- Element/state ids (`qol-toast-host-{}-{sequence}`, `qol-shot-pin-{}-{seq}`, `show#{}`, `reuse`, `superseded`, `picker_visible`, `gap`, `toggle`, `ghost`, `danger`, `primary`, `accent`).
- Log/assert/error text (`invalid plugin ID...`, `failed to launch...`, `couldn't confirm package ownership` reason, alt-tab state-machine strings, keepalive/ghost internals).
- `#[cfg(test)]` blocks and tests/ directories (inventory covers only shipped surfaces).
