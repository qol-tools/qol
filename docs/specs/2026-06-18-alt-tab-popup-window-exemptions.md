# Alt-tab popup-window exemptions (graduated, opt-in)

Status: research / design handoff. No code written.
Date: 2026-06-18.
Deeper audit: 2026-06-18, after checking the current discovery, activation,
config, Linux, and host pid-tracking paths.

## Problem

Alt-tab is a keyboard-first window switcher, but it hard-drops every window
that is not on the normal window layer. Always-on-top companion panels (the
`cli-sessions` panel, and any future qol panel pinned above other
windows) therefore can never be reached from the keyboard - only with the
mouse, which defeats the point. We want a way to let a *curated, user-owned*
set of such windows into the switcher without re-admitting the flood of system
chrome that the layer filter exists to block.

## Ground truth (macOS, measured)

The cli-sessions plugin process (one pid) presents three windows to
`CGWindowListCopyWindowInfo`:

| layer | owner | what it is |
|------|-------|-----------|
| 0   | `cli-sessions` | keepalive window (tiny; dropped by the 100px min-dimension filter) |
| 101 | `cli-sessions` | the visible panel (~360x400, alpha 1) |
| 101 | `cli-sessions` | hidden keepalive PopUp (alpha 0) |

`101` = `NSPopUpMenuWindowLevel`, set by
`qol_gpui::popup_window::configure_popup_window`
(`libs/qol-gpui/src/popup_window/platform/macos.rs:188`) so the panel stays
always-on-top. CGWindow's `kCGWindowOwnerName` is the executable basename
(`cli-sessions`), available without Screen Recording permission.

## The single blocking filter

For an **on-screen** window, the only gate the panel fails is the first
predicate in `parse_cg_entry`:

```rust
// plugins/alt-tab/src/discovery/macos/mod.rs:149
let layer = ffi::dict_get_i32(dict, keys.layer)?;
if layer != K_CG_WINDOW_LAYER_NORMAL {   // 0; returns None with NO probe/log
    return None;
}
```

Everything downstream already passes for the panel:

- size 360x400 > `MIN_WINDOW_DIM` (100) - `window_enum.rs:357`
- activation policy `Accessory` is allowed; only `Prohibited` is rejected -
  `process.rs` `policy_is_switchable`
- the AX subrole filter (`AXStandardWindow`/`AXDialog`) only runs for pids with
  multiple windows (AX is prefetched via `pids_with_multiple_windows`); the
  panel's pid has one real window, so subrole never gates it

So a single, narrow exemption at the layer branch is sufficient, and the
size/policy checks stay in place as guards. Note the layer drop emits no
telemetry, which is why these windows are silently invisible even in debug.

## Why NOT to lift the layer filter wholesale

Of 328 live windows, only a handful are layer 0. Dropping `layer != 0` admits
the menu bar, Dock, menu extras, context menus, tooltips, notification
banners, status-item windows, Spotlight, and every qol ghost overlay
(the launcher and alt-tab's own picker). The filter is correct; we want
exceptions, not its removal.

## Graduated design

### Tier 1 (do this first): user-owned allowlist, OS-specific

Not a hardcoded plugin list - a **config-driven allowlist the user maintains**,
read fresh per show. Alt-tab already calls `load_alt_tab_config()` inside
`dispatch_show` before it asks the platform for windows, so the important
property is: config is loaded, converted to discovery options, and immediately
used for that show. No restart, no daemon-global allowlist that survives the
next config read, and no built-in knowledge of any specific plugin.

The runtime config is JSON loaded by
`qol_config::load_plugin_config_from_env("plugin-alt-tab")`. The settings schema
file is `qol-config.toml`, but the plugin's consumed config shape is JSON/serde.
Default must be empty:

```jsonc
{
  "switchable_popups": {
    "macos": [],
    "linux": []
  }
}
```

A user opt-in then looks like:

```jsonc
{
  "switchable_popups": {
    "macos": [
      { "owner": "cli-sessions", "title": "cli-sessions-panel" }
    ],
    "linux": [
      { "wm_class": "plugin-cli-sessions", "title": "cli-sessions-panel" }
    ]
  }
}
```

Matching is **per-OS** because the identity keys differ:

- **macOS**: match on `kCGWindowOwnerName` (permission-free) and/or
  `kCGWindowName` (title; needs Screen Recording, which alt-tab already has
  since it shows titles). `owner` alone is permission-safe; `title` narrows to
  a specific window.
- **Linux/X11**: the equivalent exclusion is by window type
  (`_NET_WM_WINDOW_TYPE_DOCK`/`UTILITY`/etc) rather than CG layer, so the match
  keys are `WM_CLASS` and `_NET_WM_NAME` (title). The current Linux path reads
  both, but only after `_NET_WM_WINDOW_TYPE_NORMAL` filtering, so Linux needs a
  small identity prepass before the type gate.

Rule semantics should be deliberately narrow:

- each entry must contain at least one non-empty key
- all keys present in the entry must match exactly after trimming
- matching is case-sensitive initially; add case folding only if a real window
  manager forces it
- no wildcard, regex, substring, or plugin-id alias in Tier 1
- unknown fields are ignored by serde, but malformed entries should not match

Why this is not coupling: alt-tab ships zero entries and no special cases. It
only exposes a generic user-editable predicate. The user, a setup script, or a
future plugin install step can add an entry; alt-tab itself never names
`cli-sessions`.

### Tier 1 plumbing

The current trait is too narrow:

```rust
fn visible_windows(&self, include_minimized: bool) -> Result<Vec<WindowInfo>, DiscoveryError>;
```

It only threads `display.show_minimized`. Use an explicit options object because
the show path already has a fresh config value:

```rust
pub struct DiscoveryOptions {
    pub include_minimized: bool,
    pub switchable_popups: SwitchablePopupsConfig,
}
```

Then call:

```rust
Platform
    .visible_windows(DiscoveryOptions::from_config(&config))
    .unwrap_or_default()
```

from these existing call sites:

- `picker/run.rs::dispatch_show` - the user-visible path; this is the required
  "read fresh per show" contract
- `picker/monitor_listener.rs::refresh_data` - background refresh already reloads
  config, so use the same conversion
- `picker/platform/{macos,linux}.rs::pre_create` - boot/topology sizing; including
  allowlisted windows here is harmless because it only affects the initial
  window-count estimate and ghost sizing

Avoid a platform-side singleton for the allowlist. It would be easy to update in
`dispatch_show` but forget in `refresh_data` or `pre_create`, creating exactly
the stale-config behavior non-negotiable #6 is meant to prevent.

### Tier 1 macOS injection point

Current `parse_cg_entry` drops before it reads owner/title:

```rust
let layer = ffi::dict_get_i32(dict, keys.layer)?;
if layer != K_CG_WINDOW_LAYER_NORMAL {
    return None;
}
```

For the exemption, `parse_cg_entry` needs the discovery options and must read the
minimum identity fields before the layer branch:

```rust
let layer = ffi::dict_get_i32(dict, keys.layer)?;
let pid = ffi::dict_get_i32(dict, keys.pid)?;
if pid == own_pid {
    return None;
}
let owner = read_trimmed_string(dict, keys.owner);
let raw_title = read_trimmed_string(dict, keys.name);
let is_exempt_popup = layer != K_CG_WINDOW_LAYER_NORMAL
    && options.switchable_popups.macos.matches(&owner, &raw_title);

if layer != K_CG_WINDOW_LAYER_NORMAL && !is_exempt_popup {
    return None;
}
if is_exempt_popup && !popup_candidate_is_visible(dict, keys, &raw_title) {
    return None;
}
```

After that branch, keep the existing `id`, system-process, bounds,
`is_onscreen`, size, policy, AX, dedupe, and order logic. The exemption should
only bypass the layer predicate. It should not bypass `own_pid`, `is_system_process`,
`MIN_WINDOW_DIM`, app policy, AX de-dup, or stable ordering.

Add `kCGWindowAlpha` to `CgKeys` and a float CFNumber reader in
`discovery/macos/ffi.rs`; `kCGWindowAlpha` is not currently read. Debug builds
should also get a probe on the rejected non-normal-layer branch, e.g.
`reason=non_normal_layer`, `reason=popup_alpha_zero`,
`reason=popup_title_empty`, `reason=popup_allowlist_miss`. Right now the layer
drop is silent, which made this failure hard to see.

### Tier 1 guards (must-haves to avoid phantoms)

The same owner has a hidden keepalive PopUp (alpha 0) and a tiny keepalive.
An exemption keyed on owner alone would re-admit them. So for exempted
windows additionally require:

- `kCGWindowAlpha > 0` (skip hidden ghosts) - not currently read in
  `parse_cg_entry`; would need adding
- non-empty real title (skip the untitled keepalives)
- keep `MIN_WINDOW_DIM` and policy checks

Pairing `owner` + `title` in the allowlist already discriminates the real
panel from the keepalives; the alpha guard is belt-and-suspenders.

Guard order matters:

1. `own_pid` still drops alt-tab's picker/keepalive windows before matching.
2. `alpha > 0` drops the hidden PopUp from any allowlisted owner.
3. non-empty `raw_title` drops untitled keepalives before the fallback title is
   replaced with `app_name`.
4. `MIN_WINDOW_DIM` drops the 1x1 or tiny keepalive even if it is layer 0.
5. app policy still rejects `Prohibited` apps.

Owner-only entries are allowed but risky. They should still require
`alpha > 0`, non-empty title, and size/policy guards; they can include multiple
real panels from the same executable if the user chooses that broad rule.

### Tier 1 Linux injection point

Current Linux flow:

```rust
let ids = fetch_window_ids(...);
let filtered = filter_normal_windows(&session.conn, &ids, &session.atoms);
let mut windows = collect_window_info(&session.conn, session.root, &filtered, ...);
```

`filter_normal_windows` reads `_NET_WM_WINDOW_TYPE` and accepts missing or empty
types as normal. It rejects explicit non-normal types because it only accepts
`_NET_WM_WINDOW_TYPE_NORMAL`. `qol_gpui::popup_window::configure_popup_window`
sets qol popups to `_NET_WM_WINDOW_TYPE_DOCK` and adds `_QOL_GHOST`,
`_NET_WM_STATE_ABOVE`, `SKIP_TASKBAR`, and `SKIP_PAGER`, so those popups fail
right here.

To implement Linux allowlisting, do not wait until `collect_window_info`; by then
the candidate has already been removed. Instead, refactor the type filter into a
small per-id decision:

- batch-read `_NET_WM_WINDOW_TYPE`, `_NET_WM_NAME`/`WM_NAME`, `WM_CLASS`, state,
  opacity if available, and geometry for all client-list ids
- accept if the type is normal by the existing rules
- otherwise accept only if an allowlist entry matches `WM_CLASS` and/or title
- for exempted non-normal windows, require non-empty title, non-zero geometry,
  and non-zero `_NET_WM_WINDOW_OPACITY` when that property is present
- then pass the accepted ids through the existing `collect_window_info`/icon path

This makes Linux symmetrical with macOS: one narrow exception before the
non-normal-type gate, then the normal downstream guards.

### Tier 2 (target): declarative opt-in via manifest + host pid set

The plugin declares intent in `plugin.toml`, e.g.:

```toml
[capabilities.switchable-popup]
enabled = true
```

Capabilities already support forward-compatible extras, so a nested capability
table is the lowest-friction manifest shape. A first-class schema field can come
later if this graduates.

qol-tray already parses every plugin manifest and writes daemon pid files under
`/tmp/qol-tray/pids` (`runtime_pids_dir`). That mechanism is useful, but it is
not already the Tier 2 source of truth: today `persist_daemon_pids` writes every
daemon pid by plugin id for lifecycle/orphan cleanup, not a filtered list of
plugins that opted into keyboard switching.

So Tier 2 should publish a filtered runtime artifact, for example:

```text
/tmp/qol-tray/cache/switchable-popup-pids.json
```

containing only pids for manifests with `[capabilities.switchable-popup]`. That
keeps the channel file-based and avoids a new socket/API, but it is still a new
artifact with a narrower contract than the lifecycle pid files. Alt-tab can then
read that file fresh per discovery and exempt **by pid** at the same predicate
used by Tier 1.

Advantages over Tier 1: opt-in lives with the plugin that owns the window;
pid-keyed so it is rename-proof and needs no string matching or Screen
Recording; scales to any number of plugins with zero alt-tab edits per plugin.
It also avoids letting a stale owner/title entry match a different future
executable with the same basename.

Open Tier 2 detail: daemon pids are easy, because `Plugin::daemon_pid()` is
already tracked. Runtime-action panels may need action-process tracking added to
the filtered export if they are not daemons. The export should include only live
pids and be rewritten on plugin reload/recompile, daemon start/stop, and action
process start/exit.

Migration: Tier 1's allowlist and Tier 2's pid set feed the same exemption
branch. Tier 2 just swaps the lookup source. Build Tier 1 so the predicate is
isolated (`fn is_exempt_popup(window) -> bool`) and the source is pluggable.

### Tier 3 (rejected)

Allowing a whole custom window level, or a title-sentinel convention parsed
from every window, either re-floods the list or is fragile/hacky.

## Activation caveat: inclusion is not enough

Showing the panel in the list is necessary but maybe not sufficient. Alt-tab's
macOS activation path is tuned for regular layer-0 app windows:

1. `cg_window_pid_and_title(window_id)` looks up the target in the full
   `K_CG_WINDOW_LIST_EXCLUDE_DESKTOP_ELEMENTS` list, with no layer filter. This
   part should find a popup-level target.
2. `ax_find_window(pid, window_id, title)` tries AX window id, title, then the
   single-window fallback; `ax_find_window_brute_force` tries remote AX tokens.
   This might find an accessory popup, but it is not proven.
3. activation runs `AXMinimized=false`, `AXRaise`, `AXMain`, `AXFocused`,
   SkyLight `set_front`/synthetic key events, then app-level `AXFrontmost`.
4. the reassert loop stops only when `target_effectively_front(window_id, pid)`
   says the target is in front.

The current settle check has a popup-specific bug:

```rust
if ffi::dict_get_i32(dict, key_layer) != Some(K_CG_WINDOW_LAYER_NORMAL) {
    continue;
}
```

That means an exempt popup can never be reported as settled by
`target_effectively_front`, even if the popup actually becomes key. Selecting an
allowlisted popup would run the whole reassert schedule and log
`ACTIVATE_STACK phase=stuck` simply because the verifier ignores its layer.

Before trusting this feature, split activation verification by target kind:

- normal target: keep the current layer-0 `front_before_other_apps` check
- exempt popup target: success must be key/focused, not merely app-frontmost;
  use `ax_focused_window_id(pid) == Some(window_id)` as the primary debug signal
  and consider a popup-aware CG stack check only as secondary telemetry

If `AXFocusedWindow` never reports the popup but typing goes to it, add a
separate AppKit/CG key-window probe for the target app before changing the
activation contract. If typing does not go to it, do not ship inclusion alone;
it would create a selectable item that cannot actually be reached.

### Live activation test

Use `cli-sessions` specifically because it is the measured failure
case: one accessory process, one visible layer-101 panel, one hidden layer-101
keepalive, and one tiny layer-0 keepalive.

1. Add a macOS allowlist entry for owner + exact panel title.
2. Open the panel and confirm discovery emits exactly one accepted exempt popup;
   the hidden popup must be rejected by alpha and the tiny keepalive by size.
3. From at least Terminal, Finder, and a browser, invoke alt-tab and select the
   panel.
4. Confirm probes: `ACTIVATE_WIN`, either `ACTIVATE_KEY_FOCUS` or the new
   popup-key equivalent, no persistent `phase=stuck` for a successful focus.
5. Type a key or shortcut the panel handles. The panel must receive it.
6. Switch back to a normal app and confirm the existing layer-0 activation path
   still settles and still reasserts through picker teardown.

## Caveats to verify before calling it done

1. **Screen Recording dependency** for title matching on macOS. Owner-only
   matching avoids it, but title-specific rules should fail closed when the
   title is unavailable.
2. **Linux hidden popup state.** X11 has no CG-style alpha key. Confirm whether
   hidden qol popups stay in `_NET_CLIENT_LIST` with opacity 0 or disappear via
   unmap on the target window manager, then keep the non-normal exemption from
   re-admitting hidden ghosts.
3. **Tier 2 host export.** The host has pid files, but not yet a filtered
   switchable-popup pid set. Treat that export as new host behavior.
4. **Activation/keyness.** A popup appearing in the picker is not a completed
   feature until selecting it makes the popup key and user input reaches it.

## Implementation checklist

- add config structs for `switchable_popups.macos[]` and `.linux[]`, defaulting
  to empty
- replace `visible_windows(bool)` with `visible_windows(DiscoveryOptions)` and
  update all call sites
- add pure tests for allowlist matching: empty entry, owner-only, title-only,
  owner+title, mismatch, trim behavior
- add macOS parser tests around the layer branch if the code is refactored into
  a pure predicate: normal layer passes unchanged, non-normal miss rejects,
  non-normal match + alpha 0 rejects, non-normal match + empty title rejects,
  non-normal match + visible titled popup passes to downstream guards
- add Linux pure tests for the type decision: missing/empty type stays normal,
  explicit normal passes, explicit dock rejects without allowlist, explicit dock
  passes with matching identity and visible guard
- update activation settle logic for exempt-popup targets before considering
  the feature done
- run `cargo test -p alt-tab` or the plugin-local `cargo test` after the code
  change, plus a live macOS focus test

## Files referenced

- `plugins/alt-tab/src/discovery/macos/mod.rs` - `parse_cg_entry`,
  `fetch_cg_windows`, `discover_live_windows`
- `plugins/alt-tab/src/discovery/macos/window_enum.rs` -
  `collect_on_screen_windows`, `filter_visible`, `MIN_WINDOW_DIM`
- `plugins/alt-tab/src/discovery/macos/process.rs` -
  `is_switchable_app`, `policy_is_switchable`
- `plugins/alt-tab/src/discovery/macos/ffi.rs` - CF/CG dictionary readers
- `plugins/alt-tab/src/discovery/linux.rs` - `_NET_WM_WINDOW_TYPE_NORMAL`
  filtering and `WM_CLASS`/title collection
- `plugins/alt-tab/src/actions/platform/macos.rs` - activation,
  reassert loop, `target_effectively_front`
- `plugins/alt-tab/src/config.rs` - serde config loaded per show
- `plugins/alt-tab/src/picker/run.rs` - `dispatch_show` config reload and
  live discovery call
- `libs/qol-gpui/src/popup_window/platform/macos.rs:188` -
  `configure_popup_window` (sets `NSPopUpMenuWindowLevel`)
- `libs/qol-gpui/src/popup_window/platform/linux.rs` -
  `configure_popup_window` (sets `_NET_WM_WINDOW_TYPE_DOCK`, `_QOL_GHOST`,
  and above/skip-taskbar/skip-pager state)
- `apps/qol-tray/src/paths.rs` - `runtime_pids_dir`, `runtime_cache_dir`
- `apps/qol-tray/src/plugins/manager/runtime.rs` - `persist_daemon_pids`
- `libs/qol-plugin-api/src/manifest/schema.rs` - capabilities extras
