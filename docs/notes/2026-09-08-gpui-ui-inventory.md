# GPUI UI inventory and duplication audit

Investigation: 2026-09-08, HEAD `9222cfd38`, clean worktree.
Scope: every crate that compiles GPUI UI code — `libs/gpui` (crate `qol-gpui`) and its six consumers
`apps/qol-tray`, `plugins/{alt-tab,cli-sessions,launcher,removeapp,shot}`.
Method: five read-only research lanes (no edits, no builds), each inventorying one slice against the
shared kit's public surface, then an architect pass that re-verified every high-severity claim in
source. Full lane reports: `/tmp/qol-gpui-inventory/{kit,pickers,shot,panels,shapes}.md`.
Markers: **[V]** = architect-verified in source for this report, **[L]** = lane-reported with
file:line evidence, not independently re-read here.

Baseline: `cargo clippy --workspace --all-targets` exits 0 with no warnings (only the upstream
`proc-macro-error2` future-incompat note). Nothing in this audit is a lint-level defect.

## 0. Delivery status

Branch `gpui-dedup` (worktree `/media/kmrh47/WD_SN850X/Git/worktrees/gpui-dedup/qol-monorepo`) fixes
the items below. Every commit passed `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets`, the workspace nextest suite (6150 tests) and `qol check` (9/9 stages).

| Finding | Fix | Commit |
|---|---|---|
| A1 `WindowOptions` at 7 sites | `PopupWindowOptions` builder + 6 sites migrated (cli-sessions' site removed by A2) | `a8df5be15`, `6e1886a25` |
| A2 cli-sessions hand-rolled panel | `SurfaceKind::OverlayPanel` + `OpenedSurface::update_view`, cli-sessions moved onto `Surface` | `c0ca88c3f` |
| A7 `RenderImage` construction x5 | `qol_gpui::image::{render_image,render_image_rgba}` + 5 sites migrated | `a8df5be15`, `6e1886a25` |
| B2 cli-sessions toast 380x76 | sends `style = "compact"`; host derives 340x76 | `5e3f781ad` |
| B4 `pinned::scroll_steps` copy | calls `scroll_list::accumulate_steps` | `5e3f781ad` |
| B13 canvas origin-offset copies | `qol_gpui::canvas` + gamepad/shot editor migrated | `1373daab5` |
| B17 bluetooth dead `qol-gpui` dep | both target legs removed; `gpui = true` kept (host-rendered settings) | `5e3f781ad` |
| A3 shot reveal copies | public `surface::reveal` (`schedule_fresh_frame`, `await_reveal_readiness`, `RevealProof`); `Surface` refactored onto it; preview and pinned migrated; selector exempt (pre-created reused full-screen window never emits a per-reveal bounds epoch) | `e0d3a61af` |
| A6 blur guard | `ghost::BlurGuard` owns the arm/expiry state machine; per-surface durations stay local | `9ea31bb1d` |
| A7 atlas registry | `RenderImage` atlas registry promoted from alt-tab to `qol_gpui::image_registry`; alt-tab drops `image`/`smallvec` | `253890b4c` |
| A8 Linux ghost title | alt-tab Linux picker title calls `ghost::ghost_window_title` | `5234402e5` |
| B3 native_tools form nav | `settings_panel::form_nav` exported; hosted panel and native_tools share it | `4151f2364` |
| B6 removeapp palette | removeapp reads only `RemoveAppPalette` | `9d12e0676` |
| B10 alt-tab hint fitting | hint bar built through `fit_hints` | `5234402e5` |
| B15 hex validation | `qol_color::normalize_hex`; tray depends on `qol-color` directly (not the linux/macos-only `qol-gpui` leg) | `9d12e0676` |
| B18 dead kit surface | 15 uncalled `Kit` builders, `pinned_order` and 3 unused height constants removed; `ToastPresenter` crate-private, `tile_tone`/`focus_ring_for` private | `253890b4c` |
| B19 alt-tab keepalive id | passes `config::PLUGIN_ID` | `5234402e5` |
| A5 text input | `text_edit::TextField` owns cursor, anchor, motion, delete and paste; `TextFieldElement` renders the caret/selection window; launcher, removeapp and native_tools hold one field each, and the editable chrome is a `SettingsTextField` recipe | `e11bd0310`, `de0726a24` |
| B5 settings opener name | `qol_apps::desktop_integration::open_plugin_settings_via_tray` at ~18 call sites, so it no longer shares a name with the native panel opener | `f6b1b08a1` |
| B8 mirrored geometry | six consumer constants read their theme token (`RENDER_GAP`, `SEARCH_PAD`, `SEARCH_H`, `FAILBAR_H`, `EDGE`, `CHIP_TOP`); `RENDER_PAD_X/Y` and `ROW_H` stay local because no equal-valued token owns their meaning | `659b6b511` |
| B11 text width helpers | `qol_gpui::text::{shaped_width, truncate_to_width}`; launcher and alt-tab migrated | `e15179367` |
| B14 action-ring copy | shot editor and preview share one ring builder | `e15179367` |
| B21 hint-key copies | shot editor and launcher hint keys derive from the binding tables their handlers read | `e15179367` |
| A4 active-monitor ownership | `monitor.rs` owns the only active-monitor cache and names the three precedence policies (active-first, focus-first, cached-first); ghost and alt-tab keep intent, not a second answer, and every call site keeps the policy it had | `186c1a179` |
| B1 cli-sessions selection | `Selection` wraps `ScrollList`; the id anchor only restores position through one function | `8ae81aa93` |
| B9 two ghost-hide mechanisms | stopped, not approximated: the title-keyed native hide and the handle-keyed view hide cannot share one API without erasing the typed handle or threading `&mut App` into title-only callers | — |
| B12 shot warm pools | one `WarmWindowPool` serves the pin and selector caches, keyed by window kind + monitor topology + size, dropping entries whose topology no longer matches the live set | `d7accc4f8` |

### Residual, deliberately not changed

| Item | Decision |
|---|---|
| A4 multi-monitor magnitude | Ownership is unified and the single-monitor paths are guest-verified; whether focus-first and active-first should ever disagree is a precedence product question, and the available guest is single-monitor. |
| A9 shot cross-monitor move | Deliberate sticky-placement policy; a contract question, not duplication. |
| B7 spacing literals | The 41 literals and 8 off-ladder values are a design-system decision: snapping them changes pixels, extending the ladder changes the system. |
| B16 gpui re-export | One dependency line per crate against an import rewrite across six crates. |
| B20 alt-tab scrollbar | A UX addition, not duplication. |
| B22 launcher examples palette | Teaching material. |

Guest verification of A2 (`linux/mint-cinnamon`, artifact-backed lane, debug bundle):
`SURFACE_REVEAL phase=opened hidden=true` -> `phase=frame-ready expected=observed=rendered=360x400` ->
`SHOW_WIN_STATE presentation=Overlay` -> `phase=state-restored overlay_configured=true` ->
`phase=ready focus=true`; `xprop` shows `_NET_WM_STATE_ABOVE, _NET_WM_STATE_SKIP_TASKBAR,
_NET_WM_STATE_SKIP_PAGER` with `_MOTIF_WM_HINTS` decorations off and `WM_NORMAL_HINTS` 360x400
min=max; Escape gives `CLI_SESSIONS_DISMISS hidden=true` with the window unmapped and focus returned;
reopen repeats the full reveal on a new window; collapse/expand give 360x52 and 360x400 with the
overlay state preserved.

Guest verification of A3 (`linux/mint-cinnamon`, artifact-backed lane, debug bundle): preview
`SHOT_PREVIEW_REVEAL state=proof ready=true ... expected=396x329 observed=396x329 rendered=396x329`
-> `state=presented preview_ms=108`; pinned `SHOT_PIN_REVEAL state=proof ready=true ... 360x240` ->
`action_ms=98 state=presented` -> `SHOT_PIN_TRANSITION focused=true`; the refactored `Surface` on
cli-sessions and removeapp gives `phase=frame-ready ... content_rendered=true` -> `phase=revealed`
-> `phase=ready focus=true` (removeapp `first_paint_latency_ms=1`); the exempt selector still gives
`SHOT_SELECT_REVEAL state=presented`, `SHOT_SELECT_VIEWPORT aligned=true`, and a 600x400 capture.

Guest verification of A5 (`linux/mint-cinnamon`, artifact-backed lane, debug bundle): launcher typing
through the tray action route gives `LAUNCHER_INPUT ... q="fire" q_len=4 cursor=4` with result counts
falling 22 -> 15 -> 8; `shift+left` twice gives `selection=3-4` then `selection=2-4`; `ctrl+a` gives
`selection=0-4`; `backspace` clears to `q="" q_len=0 cursor=0`; typing `ch` then `left` leaves
`q="ch" cursor=1`. removeapp takes the same field through open, two typed characters, `backspace`,
`ctrl+a` and `backspace` with zero panics in the guest log and a rendered window.

Guest verification of A4 and B12 (`linux/mint-cinnamon`, artifact-backed lane, debug bundle):
launcher open/type/dismiss keeps `SHOW_GHOST`/`HIDE_WIN`/`FOCUS_RETURN` and reports
`GHOSTDUMP ... active_mon=Some(ActiveMonitor { inner: MonitorBounds { x: 0.0, y: 0.0, width: 1280.0,
height: 800.0 } })` through the new owner; the shot selector reuses its warm window
(`SHOT_SELECT_WINDOW state=reuse`, `SHOT_SELECT_OPEN result=reuse`), the pin reuses a pooled window
(`SHOT_PIN_OPEN path=reuse ms=6`, `SHOT_PIN_REVEAL ... action_ms=77 state=presented`) and returns it
(`SHOT_PIN_RECYCLE result=ok`), and no `SHOT_WARM_INVALIDATE` fires while the topology is unchanged.
cli-sessions selection is covered by the workspace tests, which pass unchanged.

Line counts in scope: `libs/gpui` 33,259 (70 files); GPUI-touching consumer code 7,832 (launcher),
8,755 (alt-tab), 10,943 (shot), 6,999 (qol-tray), 2,197 (cli-sessions), 1,141 (removeapp).

## 1. What exists

### 1.1 The shared kit (`libs/gpui`, 33 modules)

| Capability | Owner | Consumers |
|---|---|---|
| Window/surface lifecycle + dismissal | `surface/` (`Surface`, `SurfaceKind`, `SurfaceDismisser`, `OpenedSurface`, `PanelDragArea`) | tray, shot, removeapp |
| Retained settings host + contract panel | `settings_panel/` (`SettingsPanel`, `SettingsWindowHost`, `SettingsRuntime`, rows/nav/persistence) | tray, shot, removeapp, cli-sessions (hosted) |
| Popup/ghost native control | `popup_window/` (`HiddenWindowsBarrier`, `reassert_focus_until_held`, title-keyed hide/show) | all 6 |
| Per-monitor ghost sets + placement | `window/`, `ghost/` (`ActiveWindows`, `MonitorKey`, `PopupPlacement`, `reconcile*`, `dismiss_to_ghost*`) | alt-tab, launcher, shot, cli-sessions |
| Monitor/cursor snapshots | `monitor/` (`MonitorTracker`, `ActiveMonitor`, `CursorAnchor`) | all 6 (20 files) |
| Toast + push-notification layout | `toast/` (`Toast`, `ToastHost`, `ToastLayout`, `ToastTone`) | tray, shot, cli-sessions (push) |
| Spinner/status/motion | `spinner/`, `status_indicator/`, `activity_animation/`, `deck/`, `trail/` | launcher, removeapp, shot, alt-tab, cli-sessions, tray |
| Lists, scroll, dropdown, text spans | `scroll_list/`, `scrollbar/`, `dropdown/`, `text_edit/` | tray, launcher, removeapp, alt-tab, cli-sessions |
| Component recipes | `kit/` (rows, buttons, chips, hints, shadows, keycaps, action circles) | all 6 |
| Canvas/visual extras | `color_wheel/`, `gamepad/`, `vertical_label/`, `artifact/`, `history/`, `hint_bar/` | shot, settings rows, cli-sessions, launcher |
| Platform leaves | `platform/` (modifier state, ghost kind/decorations, reassert driver, window move) | all 6 |
| Runtime plumbing | `command_loop/`, `event_router/`, `keepalive/`, `probe/`, `phantom_nav/`, `pinned_order/`, `runtime_config/` | all 6 (except `pinned_order`) |

### 1.2 Consumer shape

- **alt-tab / launcher** — retained per-monitor ghost popups, sibling architecture, no `surface/`.
  Both go through the shared ghost layer; alt-tab additionally keeps a plugin-local monitor static
  and a cross-monitor move path on macOS/Windows.
- **shot** — four window families (editor, region selector, pinned, preview) plus a settings
  entrypoint. Best shared-owner adoption in the repo: `surface/`, `kit/`, `hint_bar/`, `history/`,
  `color_wheel/`, `toast/`, `scroll_list/` all used. Hand-rolls the canvas itself and three copies
  of the reveal-after-present state machine.
- **cli-sessions** — the only consumer that bypasses `surface/` entirely and hand-rolls its retained
  panel; everything else (selection scroll, phantom nav, activity animation, hint bar, keepalive,
  command loop) is shared.
- **removeapp** — small, correct `Surface`/`ScrollList`/`kit` use; hand-rolls a bare-`String` query
  field and a row div.
- **qol-tray settings_surface** — the intended hosted `SettingsWindowHost` +
  `CustomPanelFactory`/`CustomSettingsBreadcrumbs` shape; deviates only in form navigation and text
  editing, which are private in the shared panel.

## 2. Duplicate / hand-rolled findings

### 2.1 Architectural: one capability, several implementations

**A1. `WindowOptions` hand-assembled at seven sites, no shared builder.** **[V]** for
`plugins/cli-sessions/src/ui/run.rs:163-176` and `plugins/launcher/src/ui/window_host.rs:271-282`;
**[L]** for `plugins/alt-tab/src/picker/create.rs:195`, `plugins/shot/src/ui/pinned.rs:218`,
`plugins/shot/src/ui/preview.rs:637,673`, `plugins/shot/src/ui/region_selector/mod.rs:142`.
Every site repeats `window_bounds` + `titlebar: None` + `ghost_window_decorations` +
`ghost_window_kind` + `is_movable: true` + transparent background + `app_id`, and the two shot
variants already disagree on `show`/`window_background`. `window/` owns the placement math but not
the option skeleton. Severity medium, confidence high.

**A2. cli-sessions hand-rolls the retained panel lifecycle that `surface/` owns.** **[V]**
`plugins/cli-sessions/src/ui/run.rs:154-207,347-402`, `plugins/cli-sessions/src/ui/mod.rs:81-111,315-341`.
It builds `WindowOptions` itself, calls `open_window_with_focus`, `configure_overlay_window`,
`show_window_by_title` + `activate_window` + `focus`, and resizes through
`set_window_fixed_size_by_title` + `sync_window_layout_by_title` instead of
`SurfaceDismisser::resize_window/reposition_window`. removeapp (`ui/run.rs:15,47-53,69`) and shot
(`ui/editor/mod.rs:265-269`) use the shared `Surface`, which owns the bounds/viewport/frame reveal
gate and the fixed-size constraint; cli-sessions silently misses fixes to those paths. The same
file's `panel_bounds` `None` branch uses raw `Bounds::centered` where `window::centered_window_placement`
exists **[V]** (`run.rs:160`). Severity high, confidence high.

**A3. Reveal-after-present state machine copied three times inside shot; the real owner is private.**
**[L]** `plugins/shot/src/ui/region_selector/mod.rs:522-590`, `ui/preview.rs:933-991`,
`ui/pinned.rs:464-533` each implement generation-guarded wait-for-bounds/render/frame before
restoring opacity, because `libs/gpui/src/surface/mod.rs:809,856,975` (`RevealReadiness`,
`FreshFrame`, `await_reveal_readiness`) is private to `Surface::show*`. Three copies of the one
mechanism the gpui-conventions skill calls the correctness bar for X11 reveal. Severity medium-high,
confidence high.

**A4. Active-monitor ownership is split three ways.** **[V]** Three caches answer the same question:
`ghost::ACTIVE_MONITOR` + `resolve_active_monitor` (`libs/gpui/src/ghost.rs:10-40`),
`MonitorTracker::snapshot_monitor` (`libs/gpui/src/monitor.rs:147-196`), and alt-tab's plugin-local
`ACTIVE_PICKER_MONITOR` (`plugins/alt-tab/src/app/mod.rs:20`, re-derived in
`picker/mod.rs:104-163`). The precedence policies differ (`snapshot`: active → cursor → first;
`snapshot_monitor_focus_first`: focus → active → cursor; `resolve_active_monitor`: cache → active).
`dismiss_to_ghost_with` targets `resolve_active_monitor` (`ghost.rs:150`) while `Surface` centers on
`snapshot_monitor` (`surface/mod.rs:331`), and launcher feeds both in one function
(`ui/window_host.rs:113-122`). Wrong-monitor ghost/panel behavior is possible when focus and active
monitor diverge; magnitude unverified. Severity medium-high, confidence high on the paths.

**A5. Text input, caret, and selection are hand-rolled in three places.** **[V]** for
`plugins/removeapp/src/ui/mod.rs:294-330` (bare `String`, `pop`/`push`, no caret) and
`apps/qol-tray/src/settings_surface/platform/native_tools/view.rs:1583-1626` (push chars, paste into
`String`); **[L]** for the richest copy `plugins/launcher/src/ui/input.rs:25-345` (cursor, selection,
word/line motion, caret render, horizontal windowing) and a fourth in `settings_panel/view.rs`
(`ActiveControl::Edit`). `libs/gpui/src/text_edit/mod.rs` publishes only `Span` and word helpers, so
no shared field exists and the hosted settings panel lacks word/line motion. Severity medium,
confidence high.

**A6. The blur-guard policy is re-implemented three times with three durations.** **[V]**
`plugins/alt-tab/src/app/mod.rs:23` (250 ms), `plugins/launcher/src/ui/mod.rs:26` (400 ms),
`plugins/shot/src/ui/preview.rs:41-42,134-138` (400 ms + 5000 ms parked). Each view owns its own
`blur_guard_until` and passes a closure into shared `ghost::track_dismiss*`, so the duration and
re-arm policy have no single owner. Severity medium, confidence high.

**A7. Raw pixel buffer → `Arc<RenderImage>` is written five times, and the atlas refcount registry is plugin-local.** **[V]** for the five construction sites:
`plugins/alt-tab/src/rendering/preview_image.rs:18`, `plugins/alt-tab/src/picker/gather.rs:601`,
`plugins/shot/src/ui/preview.rs:745`, `plugins/shot/src/capture/frozen_frame.rs:276`,
`libs/gpui/src/color_wheel.rs:509`. **[L]** for `plugins/alt-tab/src/rendering/image_registry.rs:1-196`,
the only implementation of the "one registry per `Arc<RenderImage>`" rule the gpui-conventions skill
states as workspace-wide (Metal atlas double-decrement). Severity medium, confidence high.

**A8. alt-tab duplicates the shared ghost window-title format.** **[V]**
`plugins/alt-tab/src/picker/platform/linux/mod.rs:7-12` builds `"{prefix}@{x},{y},{w}x{h}"` by hand;
`libs/gpui/src/ghost.rs:42-47` is that exact format and is called by launcher (6 sites) and shot (3
sites). The title is the ghost set's key, so a format drift orphans hidden windows. Severity medium,
confidence high.

**A9. alt-tab keeps a live cross-monitor ghost move path on macOS and Windows.** **[V]**
`plugins/alt-tab/src/picker/monitor_listener/mod.rs:546-600` branches on
`reuse_picker_across_targets()` (`platform/macos/mod.rs:24-26` true, `platform/windows/mod.rs:24-26`
true, `platform/linux/mod.rs:36-38` false) and runs `recenter_single_ghost`, which moves the single
existing window and re-keys the map. `gpui-conventions` names `reuse_picker_across_targets` as the
path not to reintroduce; the skill's rationale is X11/Muffin-specific and Linux is excluded here.
This is a contract question to resolve (tighten the skill to Linux, or delete the path), not a
proven defect. Severity medium, confidence high.

### 2.2 Medium and low findings

| # | Finding | Location | Severity / confidence |
|---|---|---|---|
| B1 | cli-sessions keeps a local id-anchored selection index beside `ScrollList`'s index model | `plugins/cli-sessions/src/ui/selection.rs:1-48`, `ui/mod.rs:38,160-174` **[L]** | medium / high |
| B2 | cli-sessions pins notification layout `380x76`; `ToastLayout::compact()` is `340x76` | `plugins/cli-sessions/src/ui/notify.rs:29-38` vs `libs/gpui/src/toast.rs:17-18,62-68` **[V]** | low / high |
| B3 | native_tools re-derives settings-form navigation that `settings_panel/view.rs` owns privately (`Intent`, `adjacent_visible_row`, `escape_step`) | `apps/qol-tray/src/settings_surface/platform/native_tools/view.rs:614-700,1561-1581` **[L]** | medium / high |
| B4 | `pinned::scroll_steps` is a verbatim copy of public `scroll_list::accumulate_steps` | `plugins/shot/src/ui/pinned.rs:1208-1226` vs `libs/gpui/src/scroll_list.rs:73-91` **[V]** | medium / high |
| B5 | Two `open_plugin_settings` with different behavior (native panel vs tray-route/browser fallback) | `libs/gpui/src/settings_panel/mod.rs:572` vs `libs/apps/src/desktop_integration/mod.rs:29` **[L]** | medium / high |
| B6 | removeapp mixes `RemoveAppPalette` and `kit.palette` so one semantic token renders two grays; cli-sessions mixes them structurally | `plugins/removeapp/src/ui/mod.rs:36,439,539,570,783`; `plugins/cli-sessions/src/ui/render.rs:246,291,332` **[L]** | low / high |
| B7 | 41 spacing literals in consumer UI, 8 off the theme ladder; the theme guard only polices settings scope | removeapp ×20, launcher ×15, shot ×2, cli-sessions ×2, alt-tab ×2 **[L]** | low / high |
| B8 | 9 consumer geometry constants numerically mirror theme tokens (`SEARCH_H=40` vs `HEIGHT_HINT_BAR`, `RENDER_GAP=16` vs `SPACE_PAD`, `EDGE=8` vs `SPACE_INSET`, …) | `plugins/alt-tab/src/picker/layout.rs:12,24-26`; `plugins/removeapp/src/ui/mod.rs:19-22`; `plugins/shot/src/ui/pinned.rs:21`, `ui/preview.rs:36`, `ui/region_selector/mod.rs:27` **[L]** | low / high |
| B9 | Two ghost-hiding mechanisms (native title path vs `ActiveWindows` handle path); launcher uses both in one flow | `libs/gpui/src/ghost.rs:79,94` vs `libs/gpui/src/window.rs:187`; `plugins/launcher/src/ui/window_host.rs:117,139` **[L]** | medium / high |
| B10 | Hint bar is split across `hint_bar/` (fitting) and `kit` (styling); alt-tab bypasses `fit_hints` entirely | `libs/gpui/src/hint_bar.rs:8-61`, `libs/gpui/src/kit.rs:392-426`; `plugins/alt-tab/src/app/render.rs:276-282` **[L]** | medium / high |
| B11 | Two text-measurement helpers reaching `cx.text_system()` directly; no shared owner | `plugins/alt-tab/src/app/render.rs:578`, `plugins/launcher/src/ui/view.rs:220,241` **[L]** | low / high |
| B12 | Two warm-window pools in shot (`PIN_CACHE`, `SelectorCache`) beside the shared ghost/`ActiveWindows` layer, without topology invalidation | `plugins/shot/src/ui/pinned.rs:31-33,248-323`; `plugins/shot/src/platform/linux/selector_cache.rs:18-48` **[L]** | low-medium / medium |
| B13 | Canvas `bounds.origin` mapping and element-bounds probing have no public owner; the lib has private copies | `plugins/shot/src/ui/editor/render.rs:51-58,304-306`; `libs/gpui/src/gamepad/diagram/mod.rs:281`; `libs/gpui/src/color_wheel.rs:58`, `settings_panel/view.rs:285,3993` **[L]** | low-medium / high |
| B14 | Action-ring navigation duplicated between shot editor and preview | `plugins/shot/src/ui/editor/mod.rs:414-418`; `ui/preview.rs:993-997` **[L]** | low / high |
| B15 | `dev/runtime_gpui.rs` re-implements hex color validation that `qol_color::parse_hex_color` owns (tray does not depend on `qol-color`) | `apps/qol-tray/src/dev/runtime_gpui.rs:31-44` vs `libs/color/src/lib.rs:1-10` **[L]** | low / high |
| B16 | `qol_gpui` does not re-export `gpui`, so all seven manifests pin the framework directly | `libs/gpui/src/lib.rs:35-47` **[L]** | low-medium / high |
| B17 | `plugins/bluetooth` declares `qol-gpui` on both platform legs with zero usage, plus a stale `gpui = true` capability and legacy `__qol-settings-surface` handler that opens a browser | `plugins/bluetooth/Cargo.toml:27,32`; `plugin.toml:77`; `src/lib.rs:9`; `src/settings/mod.rs:3` **[V]** | medium / high |
| B18 | Dead/shadowed public kit surface: 15 uncalled `Kit` builders, `pinned_order`, `ToastPresenter`, two status-dot owners, `focus_ring_for`/`tile_tone` used only internally | `libs/gpui/src/kit.rs:87,144,148,159,187,270,279,294,534,543,629,648,676,768,776,809,874`; `pinned_order.rs:2`; `toast.rs:332` **[V]** | low-medium / high |
| B19 | alt-tab opens its keepalive without an app id, while its own discovery filter expects the app-id form | `plugins/alt-tab/src/picker/run.rs:95` vs `plugins/launcher/src/ui/keepalive.rs:6`, `plugins/alt-tab/src/discovery/platform/macos/mod.rs:99-102` **[L]** | low / high |
| B20 | alt-tab's scrollable card grid draws no scrollbar while launcher attaches the shared `seam_track` | `plugins/alt-tab/src/app/render.rs:322-325` vs `plugins/launcher/src/ui/view.rs:583` **[L]** | low / high |
| B21 | Hint key strings are a second source of truth for the editor key handler (and launcher repeats the pattern) | `plugins/shot/src/ui/editor/render.rs:156-166` vs `editor/mod.rs:649-662`; `plugins/launcher/src/ui/view.rs:648-654` **[L]** | low / high |
| B22 | Launcher examples carry 87 Catppuccin color literals; the only literal colors in consumer code | `plugins/launcher/examples/*.rs` **[V]** | low / high |

## 3. Promotion candidates (one owner deletes the duplicates)

1. **`qol_gpui::window::GhostWindowOptions` builder** — collapses 7 `WindowOptions` sites across 4
   crates (A1) and gives `centered_window_placement` a caller (A2).
2. **Public `reveal_after_present`** — extract the private `RevealReadiness`/`FreshFrame` path; deletes
   three shot copies (A3).
3. **Shared text field** (value, cursor, selection, caret, horizontal windowing) — absorbs launcher's
   state machine and the removeapp/native_tools/settings-panel half-copies (A5).
4. **`qol_gpui::image::render_image` + promoted atlas registry** — deletes 5 construction copies and
   makes the refcount rule reusable (A7).
5. **Blur-guard policy inside `ghost::track_dismiss*`** — removes 3 constants and 3 re-arm paths (A6).
6. **Move cli-sessions onto `Surface`** — removes the hand-rolled panel, reveal, resize, and centering
   fallback (A2).
7. **One active-monitor owner** — delete `ghost::ACTIVE_MONITOR` or make `MonitorTracker` the only
   cache and expose one precedence policy (A4).
8. **Extend the theme guard tests beyond settings scope** — the mechanical fix that stops B7/B8 drift
   from returning.
9. **Delete or wire the dead kit surface** — 15 `Kit` builders, `pinned_order`, `ToastPresenter`
   (B18); decide whether the `settings_panel::components` recipes are the single design-system owner.
10. **`qol_gpui::canvas` geometry helpers** (`at`, `point_in`, `local_in`) plus a tracked-bounds type —
    removes the private lib copies and the shot editor's probe plumbing (B13).

## 4. Corrections and non-findings

- **Correction (architect).** The kit lane reported `scroll_list::{clamp_into_view, shift_window,
  accumulate_steps}` as dead public API. Verified false: each has internal callers
  (`scroll_list.rs:143,122,132`). They are public-but-only-internally-used, a surface-hygiene point,
  not dead code. `PinnedOrder` has no consumers outside its own tests; `focus_ring_for` is
  test-only; `tile_tone` is used internally by `letter_tile`.
- **Correction (architect).** The kit lane's F2 claim survives scrutiny: `.panel(`, `.row(`,
  `.row_tight(`, `.row_described(`, `.row_selected_tinted(`, `.description(`, `.mono(`, `.radio(`,
  `.check(`, `.lamp(`, `.divider(`, `.button_primary(`, `.button_danger(`, `.segmented(` have zero
  consumer call sites; the `.label(` and `.check(` grep hits are unrelated methods.
- **No literal colors in consumer UI.** A naive `rgb(0x…` grep returns hundreds of hits, but every
  one is token-based (`rgb(palette.field)`); the only numeric literals are in launcher examples and
  three transparent/white constants inside `kit.rs`.
- **`surface`/`placement` unused by the pickers is sanctioned** — both are documented ghost-popup
  consumers, and the skill explicitly allows either architecture.
- **removeapp has no settings contract**, and cli-sessions' contract panel is tray-hosted, so the
  "don't hand-roll settings windows" rule is not violated by either; `native_tools` is the intended
  `CustomPanel` shape.
- **`qol-gpui`'s `image` dependency is real** (prior audit): `color_wheel.rs` constructs gpui's
  public `RenderImage` from `image::Frame`.
- **Standards drift (side finding).** `gpui-conventions` and `qol-plugin-gpui-surfaces` still point
  at `libs/qol-gpui`, which no longer exists after `49d3ff07f` renamed the folder to `libs/gpui`
  (crate name remains `qol-gpui`). Both skills should be corrected before the next session follows
  a dead path.

## 5. Lane reports

- `/tmp/qol-gpui-inventory/kit.md` — kit catalog (33 modules), internal duplication, dead surface.
- `/tmp/qol-gpui-inventory/pickers.md` — alt-tab vs launcher, 20 capabilities × 2, ghost contract verification.
- `/tmp/qol-gpui-inventory/shot.md` — 39 capabilities, editor/selector/pinned/preview.
- `/tmp/qol-gpui-inventory/panels.md` — cli-sessions, removeapp, tray settings_surface, 46 capabilities.
- `/tmp/qol-gpui-inventory/shapes.md` — 17 shape greps across all consumers, ownership hygiene.

## 6. What this audit did not do

No code was changed, no fix was applied, and no runtime behavior was exercised in a guest.
A4 (wrong-monitor ghost/panel) and A9 (cross-monitor move) are the only findings whose user-visible
magnitude is unverified; both need a compositor-backed guest lane before being called defects.
