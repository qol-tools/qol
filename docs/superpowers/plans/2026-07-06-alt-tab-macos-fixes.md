# Alt-Tab macOS Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix four alt-tab annoyances on macOS: grid overshooting the monitor with many windows, occasional duplicate window cards, visible progressive UI build-in after open, and small cards when only a few windows are open (dynamic sizing, default on).

**Architecture:** Issues 1+4 are one layout change: `picker_layout` becomes the single resolver of an *effective card scale* (shrink-to-fit always; grow-to-fill when the new `dynamic_card_scale` config is on) and exposes the resolved `CardMetrics` so render, navigation, and the preview plane can never disagree. Issues 2+3 are evidence-first debugging tasks with a leading hypothesis each; they end in a fix only if the evidence confirms it.

**Tech Stack:** Rust, GPUI, macOS CoreGraphics/AX. Crate: `plugins/plugin-alt-tab` (package name `alt-tab`).

## Global Constraints

- Comment-free codebase. No code comments except where a constraint cannot be expressed in code (existing file style wins).
- Conventional commits, one-liners, no co-authors, no push.
- Gates before every commit: `cargo fmt --all --check`, `cargo clippy --all-targets --all-features --keep-going -- -D warnings`, `cargo build`, `cargo test` (run from `plugins/plugin-alt-tab/`).
- Every bug fix starts with a failing test (plugin CLAUDE.md).
- Debug logging only under `#[cfg(debug_assertions)]`, prefixed `[alt-tab/...]`.
- No `#[cfg(target_os)]` in business logic; platform code stays in per-OS modules.
- A resident daemon serves the old binary until restarted: after building, restart the daemon (qol-tray Recompile or kill + relaunch) before judging behavior by hand.
- The show path must never capture screenshots synchronously (plugin CLAUDE.md non-negotiable #4).
- Read `plugins/plugin-alt-tab/CLAUDE.md` before starting any task.

---

### Task 1: Effective-scale layout solver in `shared/layout.rs`

Fixes the overshoot (issue 1) and implements dynamic sizing (issue 4) in the layout core. `picker_layout` gains a `dynamic_card_scale: bool` parameter and resolves an effective scale; `PickerLayout` gains the resolved `metrics`. Callers are updated in Task 2 - to keep this task compiling on its own, update the four call sites mechanically here with `false` and leave behavior-visible adoption to Task 2.

**Files:**
- Modify: `plugins/plugin-alt-tab/src/shared/layout.rs`
- Modify (mechanical, pass `false`): `plugins/plugin-alt-tab/src/app/render.rs:135`, `plugins/plugin-alt-tab/src/app/mod.rs:435`, `plugins/plugin-alt-tab/src/app/input.rs:139`, `plugins/plugin-alt-tab/src/picker/create.rs:90`
- Test: `plugins/plugin-alt-tab/src/shared/layout.rs` (inline `mod tests`)

**Interfaces:**
- Produces: `pub fn picker_layout(window_count: usize, max_columns: usize, monitor_size: Option<(f32, f32)>, show_hotkey_hints: bool, card_scale: f32, card_padding: f32, dynamic_card_scale: bool) -> PickerLayout`
- Produces: `pub struct PickerLayout { pub width: f32, pub height: f32, pub columns: usize, pub metrics: CardMetrics }`

Geometry background for the solver. Card height is affine in scale `s` for fixed padding `p`:

```
preview_w = BASE_CARD_WIDTH * s - 2p
preview_h = preview_w * 9/16
card_h    = preview_h + BASE_LABEL_STRIP_HEIGHT * s + 2p
          = s * (BASE_CARD_WIDTH * 9/16 + BASE_LABEL_STRIP_HEIGHT) + 2p * (1 - 9/16)
```

So the largest scale that fits `cols` columns and `rows` rows in a `(max_w, max_h)` budget is the min of a width solve and a height solve, both closed-form.

- [ ] **Step 1: Write the failing tests** (append inside `mod tests` in `layout.rs`)

```rust
    fn content_height(layout: &PickerLayout, count: usize, hints: bool) -> f32 {
        let rows = count.max(1).div_ceil(layout.columns.max(1));
        let hints_h = if hints { HOTKEY_HINTS_HEIGHT } else { 0.0 };
        RENDER_PAD_Y
            + rows as f32 * layout.metrics.card_height
            + rows.saturating_sub(1) as f32 * RENDER_GAP
            + hints_h
    }

    #[test]
    fn grid_content_always_fits_monitor_budget() {
        let budgets = [(1280.0, 800.0), (1920.0, 1080.0), (3440.0, 1440.0)];
        let scales = [0.5, 1.5, 2.5];
        let counts = [1, 2, 3, 6, 12, 30];
        for dynamic in [false, true] {
            for (bw, bh) in budgets {
                for scale in scales {
                    for count in counts {
                        let layout = picker_layout(
                            count,
                            6,
                            Some((bw, bh)),
                            true,
                            scale,
                            DEFAULT_CARD_PADDING,
                            dynamic,
                        );
                        let fits = content_height(&layout, count, true) <= bh * 0.9 + 0.01
                            && layout.width <= bw * 0.9 + 0.01;
                        let at_floor = (layout.metrics.scale - MIN_CARD_SCALE).abs() < 0.0001;
                        assert!(
                            fits || at_floor,
                            "dynamic={dynamic} budget={bw}x{bh} scale={scale} count={count}: \
                             content overflows without hitting the scale floor"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn dynamic_scale_grows_for_few_windows() {
        let cases = [(1, 1), (2, 2), (3, 3)];
        for (count, expected_columns) in cases {
            let layout = picker_layout(
                count,
                6,
                Some((3440.0, 1440.0)),
                true,
                DEFAULT_CARD_SCALE,
                DEFAULT_CARD_PADDING,
                true,
            );
            assert_eq!(layout.columns, expected_columns, "count={count}");
            assert!(
                (layout.metrics.scale - MAX_CARD_SCALE).abs() < 0.0001,
                "count={count}: huge monitor must max out card scale, got {}",
                layout.metrics.scale
            );
        }
    }

    #[test]
    fn dynamic_prefers_one_row_over_stacking_at_equal_scale() {
        let layout = picker_layout(
            2,
            6,
            Some((3440.0, 1440.0)),
            false,
            DEFAULT_CARD_SCALE,
            DEFAULT_CARD_PADDING,
            true,
        );
        assert_eq!(layout.columns, 2, "two windows must sit side by side");
    }

    #[test]
    fn fixed_mode_never_exceeds_configured_scale() {
        let counts = [1, 2, 3, 12, 30];
        for count in counts {
            let layout = picker_layout(
                count,
                6,
                Some((3440.0, 1440.0)),
                true,
                1.0,
                DEFAULT_CARD_PADDING,
                false,
            );
            assert!(
                layout.metrics.scale <= 1.0 + 0.0001,
                "count={count}: fixed mode grew past configured scale"
            );
        }
    }

    #[test]
    fn fixed_mode_shrinks_to_fit_instead_of_overshooting() {
        let layout = picker_layout(
            30,
            6,
            Some((1280.0, 800.0)),
            true,
            2.5,
            DEFAULT_CARD_PADDING,
            false,
        );
        assert!(
            layout.metrics.scale < 2.5,
            "30 windows at scale 2.5 on 1280x800 must shrink, got {}",
            layout.metrics.scale
        );
        assert!(content_height(&layout, 30, true) <= 800.0 * 0.9 + 0.01);
    }
```

Also update every existing test call of `picker_layout(...)` in this file to pass a trailing `false`.

- [ ] **Step 2: Run tests to verify the new ones fail to compile (missing param/field), existing pass after the mechanical arg**

Run: `cd plugins/plugin-alt-tab && cargo test --lib shared::layout`
Expected: compile error `this function takes 6 arguments but 7 were supplied` until Step 3.

- [ ] **Step 3: Implement the solver**

Replace `PickerLayout` and `picker_layout` in `plugins/plugin-alt-tab/src/shared/layout.rs`:

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PickerLayout {
    pub width: f32,
    pub height: f32,
    pub columns: usize,
    pub metrics: CardMetrics,
}

pub fn picker_layout(
    window_count: usize,
    max_columns: usize,
    monitor_size: Option<(f32, f32)>,
    show_hotkey_hints: bool,
    card_scale: f32,
    card_padding: f32,
    dynamic_card_scale: bool,
) -> PickerLayout {
    let count = window_count.max(1);
    let padding = clamp_card_padding(card_padding);
    let (max_w, max_h) = monitor_size
        .map(|(w, h)| (w * 0.9, h * 0.9))
        .unwrap_or((1820.0, 980.0));
    let hints_height = if show_hotkey_hints {
        HOTKEY_HINTS_HEIGHT
    } else {
        0.0
    };
    let grid_max_h = max_h - hints_height;

    let (columns, scale) = if dynamic_card_scale {
        best_dynamic_fit(count, max_columns, max_w, grid_max_h, padding)
    } else {
        fixed_fit(count, max_columns, max_w, grid_max_h, padding, card_scale)
    };

    let metrics = CardMetrics::from_config(scale, padding);
    let width = (width_for_cols(columns, &metrics) + WIDTH_SLACK).min(max_w);
    let height = (picker_height_for(count, columns, &metrics) + hints_height).min(max_h);
    PickerLayout {
        width,
        height,
        columns,
        metrics,
    }
}

fn best_dynamic_fit(
    count: usize,
    max_columns: usize,
    max_w: f32,
    max_h: f32,
    padding: f32,
) -> (usize, f32) {
    let cap = count.min(max_columns.max(2)).max(1);
    let mut best = (1usize, MIN_CARD_SCALE);
    for cols in 1..=cap {
        let rows = count.div_ceil(cols);
        let fit = fit_scale(cols, rows, max_w, max_h, padding).min(MAX_CARD_SCALE);
        if fit >= best.1 {
            best = (cols, fit);
        }
    }
    (best.0, best.1.max(MIN_CARD_SCALE))
}

fn fixed_fit(
    count: usize,
    max_columns: usize,
    max_w: f32,
    max_h: f32,
    padding: f32,
    card_scale: f32,
) -> (usize, f32) {
    let configured = clamp_card_scale(card_scale);
    let metrics = CardMetrics::from_config(configured, padding);
    let preferred = preferred_column_count(count, max_columns);
    let columns = if width_for_cols(preferred, &metrics) <= max_w {
        preferred
    } else {
        cols_for_width(max_w, count, &metrics)
    };
    let rows = count.div_ceil(columns.max(1));
    let fit = fit_scale(columns, rows, max_w, max_h, padding);
    (columns, configured.min(fit).max(MIN_CARD_SCALE))
}

fn fit_scale(cols: usize, rows: usize, max_w: f32, max_h: f32, padding: f32) -> f32 {
    let cols_f = cols.max(1) as f32;
    let rows_f = rows.max(1) as f32;
    let aspect = PREVIEW_ASPECT_H / PREVIEW_ASPECT_W;
    let width_budget = max_w - RENDER_PAD_X - WIDTH_SLACK - (cols_f - 1.0) * RENDER_GAP;
    let scale_w = width_budget / (cols_f * BASE_CARD_WIDTH);
    let row_budget = (max_h - RENDER_PAD_Y - (rows_f - 1.0) * RENDER_GAP) / rows_f;
    let height_slope = BASE_CARD_WIDTH * aspect + BASE_LABEL_STRIP_HEIGHT;
    let height_intercept = 2.0 * padding * (1.0 - aspect);
    let scale_h = (row_budget - height_intercept) / height_slope;
    scale_w.min(scale_h)
}
```

Then make the four external call sites compile by appending `false` as the last argument (exact adoption happens in Task 2):

- `plugins/plugin-alt-tab/src/app/render.rs:135`
- `plugins/plugin-alt-tab/src/app/mod.rs:435`
- `plugins/plugin-alt-tab/src/app/input.rs:139`
- `plugins/plugin-alt-tab/src/picker/create.rs:90`

- [ ] **Step 4: Run the full plugin gate**

Run: `cd plugins/plugin-alt-tab && cargo fmt --all --check && cargo clippy --all-targets --all-features --keep-going -- -D warnings && cargo test`
Expected: all tests pass, including the five new ones.

- [ ] **Step 5: Commit**

```bash
git add plugins/plugin-alt-tab/src
git commit -m "feat(alt-tab): resolve effective card scale in picker layout"
```

---

### Task 2: Config flag + state plumbing + metrics adoption

Adds `display.dynamic_card_scale` (default **true**), threads it along the exact same path `card_scale` takes, and switches every consumer to the layout-resolved `metrics` so render/nav/preview-plane share one geometry.

**Files:**
- Modify: `plugins/plugin-alt-tab/src/config.rs` (DisplayConfig struct + `Default` impl at lines 8-27; the debug log line at 149-152 may include the new flag)
- Modify: `plugins/plugin-alt-tab/qol-config.toml` (new `[field.display_dynamic_card_scale]`)
- Modify: `plugins/plugin-alt-tab/src/picker/create.rs` (PickerInit field at :114, init at :149, layout call at :90-97)
- Modify: `plugins/plugin-alt-tab/src/picker/mod.rs` (state field at :549, init copy at :581, per-show reload at :724, test fixtures at :968 and :1133)
- Modify: `plugins/plugin-alt-tab/src/app/render.rs` (RenderSnap metrics at :123, layout call at :135-141)
- Modify: `plugins/plugin-alt-tab/src/app/mod.rs` (layout call at :435-442, metrics at :443)
- Modify: `plugins/plugin-alt-tab/src/app/input.rs` (layout calls at :139-145 and around :160)

**Interfaces:**
- Consumes: Task 1's `picker_layout(..., dynamic_card_scale)` and `PickerLayout.metrics`.
- Produces: `DisplayConfig.dynamic_card_scale: bool`, state field `dynamic_card_scale: bool` alongside every existing `card_scale` field.

- [ ] **Step 1: Write the failing config test** (in `config.rs` tests module, mirroring the existing contract-default tests there)

```rust
    #[test]
    fn dynamic_card_scale_defaults_on() {
        assert!(DisplayConfig::default().dynamic_card_scale);
        let defaults = contract_defaults();
        assert!(defaults.display.dynamic_card_scale);
    }
```

If `config.rs` has no `contract_defaults()` helper, mirror how its existing tests obtain contract defaults (`qol_config::typed_defaults_from_contract(CONFIG_CONTRACT)` - see `CONFIG_CONTRACT` at config.rs:142).

- [ ] **Step 2: Run it, expect compile failure on the missing field**

Run: `cd plugins/plugin-alt-tab && cargo test --lib config`

- [ ] **Step 3: Implement config + contract**

`config.rs`: add `pub dynamic_card_scale: bool,` to `DisplayConfig` (line 8 block) and `dynamic_card_scale: true,` to its `Default` (line 26 block).

`qol-config.toml`, next to `[field.display_card_scale]` (line 43):

```toml
[field.display_dynamic_card_scale]
type = "boolean"
config_key = "display.dynamic_card_scale"
label = "Dynamic card size"
description = "Grow window cards to fill free space when few windows are open; shrink to fit when many are."
section = "layout"
default = true
```

Copy the exact `section` value from the neighboring `display_card_scale` field if it differs from `layout`.

- [ ] **Step 4: Thread the flag through state (compile-driven)**

Add `dynamic_card_scale: bool` beside every `card_scale` listed in Files above (create.rs:114/149, picker/mod.rs:549/581/724/968/1133 - fixtures use `true`). Then change the four `picker_layout` call sites from the Task 1 placeholder `false` to the threaded value (`d.dynamic_card_scale`, `state.dynamic_card_scale`, `req.config.display.dynamic_card_scale`). Grep to guarantee nothing is missed:

Run: `grep -rn "card_scale" plugins/plugin-alt-tab/src --include='*.rs' | grep -v layout.rs`
Expected: every hit has a `dynamic_card_scale` sibling or is the layout call itself.

- [ ] **Step 5: Adopt layout metrics as the single geometry source**

- `render.rs`: compute `let layout = picker_layout(...)` *before* building `RenderSnap`, and set `metrics: layout.metrics` (replacing `CardMetrics::from_config(d.card_scale, d.card_padding)` at :123).
- `app/mod.rs:443`: replace `CardMetrics::from_config(state.card_scale, state.card_padding)` with `layout.metrics` from the call directly above.
- `input.rs`: only consumes `layout.columns`; just thread the flag.
- `create.rs`: only consumes width/height; just thread the flag.

Run: `grep -rn "CardMetrics::from_config" plugins/plugin-alt-tab/src --include='*.rs' | grep -v layout.rs`
Expected: zero hits outside `layout.rs` (the picker consumers all go through `PickerLayout.metrics`).

- [ ] **Step 6: Full gate + manual verification**

Run: `cd plugins/plugin-alt-tab && cargo fmt --all --check && cargo clippy --all-targets --all-features --keep-going -- -D warnings && cargo build && cargo test`
Expected: green.

Manual (requires daemon restart first - see Global Constraints): with 2 windows open, cards render noticeably larger than before; with ~20 windows on a laptop screen, the grid stays inside the monitor; toggling `display.dynamic_card_scale = false` in the plugin config restores configured-size cards.

- [ ] **Step 7: Commit**

```bash
git add plugins/plugin-alt-tab
git commit -m "feat(alt-tab): dynamic card sizing on by default with shrink-to-fit"
```

---

### Task 3: Duplicate window cards - evidence first, then the budget-slack fix

Leading hypothesis: `should_keep` in `plugins/plugin-alt-tab/src/discovery/macos/ax.rs:426-444` lets a second CG entry of the same logical window through whenever the AX budget exceeds the AX id list (`budget = accepted.max(id_map.len())` at ax.rs:386, and the `ax_ids.len() >= dedup.budget` guard at ax.rs:435 goes inert when `accepted > id_map.len()`). This degrades exactly when AX is slow or the `_AXWindowID` attribute is missing - timing-dependent, matching "sometimes".

**Files:**
- Modify: `plugins/plugin-alt-tab/src/discovery/macos/ax.rs`
- Test: inline tests in `ax.rs` (or the existing dedup test module if one exists - grep `mod.*tests` in the file first)

**Interfaces:**
- Consumes: `PidDedup { ax_ids, ax_meta, budget }`, `emit_deduped`, `should_keep` (all private to `ax.rs`).
- Produces: no signature changes; behavior change inside `should_keep` only.

- [ ] **Step 1: Reproduce and capture evidence (do not skip)**

Build debug (`cargo build`), restart the daemon, and when a duplicate appears capture stderr: the `[alt-tab/enum] pre-dedup:` / `post-dedup:` lines and any `[alt-tab/ax] DEDUP` lines for the affected pid. Record: app name, whether both cards carry the same title, whether both activate the same window.

**Decision gate:** if the evidence shows the duplicate ids appear in *pre-dedup with different pids*, or only one card activates the real window, STOP - the hypothesis below is wrong; report findings to the user instead of fixing.

- [ ] **Step 2: Write the failing test**

```rust
    #[test]
    fn budget_slack_never_readmits_windows_missing_from_ax_ids() {
        let win = |id: u32| CgWindow {
            id,
            pid: 100,
            layer: 0,
            app_name: "foo".to_string(),
            title: "bar".to_string(),
            has_title: true,
            is_onscreen: true,
            is_cross_space: false,
            x: 0.0,
            y: 0.0,
            w: 800.0,
            h: 600.0,
        };
        let mut ax_meta = HashMap::new();
        ax_meta.insert(
            10u32,
            AxWindowMeta {
                title: "bar".to_string(),
                is_minimized: false,
            },
        );
        let dedup = PidDedup {
            ax_ids: ax_meta.keys().copied().collect(),
            ax_meta,
            budget: 2,
        };
        let mut info = HashMap::new();
        info.insert(100, dedup);
        let out = emit_deduped(vec![win(10), win(11)], &info);
        let ids: Vec<u32> = out.iter().map(|w| w.id).collect();
        assert_eq!(
            ids,
            vec![10],
            "AX listed exactly one window; the second CG entry is an overlay and must be dropped"
        );
    }
```

`AxWindowMeta` is `{ title: String, is_minimized: bool }` (ax.rs:54). `ax.rs` has no dedup test module yet - add `#[cfg(test)] mod dedup_tests` at the end of the file with the imports `use super::*;` and `use std::collections::HashMap;`.

- [ ] **Step 3: Run it, expect failure**

Run: `cd plugins/plugin-alt-tab && cargo test budget_slack_never_readmits`
Expected: FAIL with `ids == [10, 11]`.

- [ ] **Step 4: Implement the fix in `should_keep`**

```rust
fn should_keep(win: &CgWindow, dedup: &PidDedup, emitted: usize) -> bool {
    if emitted >= dedup.budget {
        return false;
    }
    if !dedup.ax_ids.is_empty() && !dedup.ax_ids.contains(&win.id) {
        return false;
    }
    true
}
```

Keep the existing `#[cfg(debug_assertions)]` eprintln blocks inside the branches (restore them around the new conditions with the same messages). This makes the AX id list authoritative whenever it is non-empty; the budget remains the only guard when AX returned no ids (timeout path, unchanged).

**Risk to verify in Step 5:** windows legitimately absent from AX (`_AXWindowID` unsupported apps) would vanish. The existing test suite has cases for AX-absent apps - if any existing dedup test fails after this change, the safer variant is to drop a non-AX window only when another kept window of the same pid has an identical title; implement that instead and rerun.

- [ ] **Step 5: Full gate**

Run: `cd plugins/plugin-alt-tab && cargo fmt --all --check && cargo clippy --all-targets --all-features --keep-going -- -D warnings && cargo test`
Expected: green, including all pre-existing dedup tests.

- [ ] **Step 6: Manual soak**

Restart the daemon; open/close the picker across the apps recorded in Step 1 over a normal work session. No duplicates, and no previously-listed window missing.

- [ ] **Step 7: Commit**

```bash
git add plugins/plugin-alt-tab/src/discovery/macos/ax.rs
git commit -m "fix(alt-tab): drop CG entries missing from a non-empty AX window list"
```

---

### Task 4: Stale-frame reveal - paint the new grid before flipping alpha

**Status note (2026-07-06):** a previous run stopped "Task 4" citing SCK/live-frame trace evidence - that rationale belongs to Task 5 (the fill-batching hypothesis from an earlier plan revision), not to this task. This task is still open: its Step 1 gate (`REUSE_SHOW_WINDOW` → `SHOW_PAINTED` gap) has not been evaluated, and the capture backend is irrelevant to reveal ordering.

Root cause (traced 2026-07-06, supersedes the earlier cold-cache hypothesis as the primary fix): on the reuse path, `try_reuse` (`plugins/plugin-alt-tab/src/picker/reuse.rs:32-113`) applies the new window list synchronously (`apply_reuse`), then calls `super::platform::show_picker_window(&title, req.all_titles)` at reuse.rs:79 - an instant alpha flip. The window's surface still holds the last frame presented at the previous dismissal; GPUI paints the new state one or more frames later (the existing `SHOW_PAINTED` / `SHOW_PAINT_TIMEOUT` probes at reuse.rs:87-109 measure exactly this gap). Whenever the window set changed while the picker was hidden, the user sees the stale grid snap into the new one - "sometimes" - and dynamic card sizing (Task 1) amplifies it because a count change now also resizes every card.

Fix: reveal inside the already-registered `on_next_frame` callback (paint-then-reveal), with the existing 120ms timer as a reveal fallback so a stalled paint can never leave the picker invisible.

**Files:**
- Modify: `plugins/plugin-alt-tab/src/picker/reuse.rs`
- Investigate afterward if pop-in persists: `plugins/plugin-alt-tab/src/picker/gather.rs` (see Task 5)

**Interfaces:**
- Consumes: `super::platform::show_picker_window(&str, &[String])`, `window.on_next_frame`, `AltTabApp::focus_for_keys`.
- Produces: no signature changes; reveal-ordering change inside `try_reuse` only.

- [ ] **Step 1: Confirm the stale frame with the existing probes**

Debug build, restart daemon, change the visible window set (open/close an app window) while the picker is hidden, then open it. Capture the trace: the gap between `REUSE_SHOW_WINDOW` and `SHOW_PAINTED` is the stale-frame window. If `SHOW_PAINTED` lands at 0-1ms consistently AND the jank still reproduces visually, STOP - the reveal ordering is not the cause; report findings.

- [ ] **Step 2: Reorder reveal after first paint**

In `try_reuse`, remove the `show_picker_window` call at reuse.rs:79 and the `focus_for_keys("reuse-after-show", ...)` at reuse.rs:86, and fold them into the `on_next_frame` closure (which must move `title`, `all_titles` clone, and an `Entity<AltTabApp>`/`WindowHandle` capture as needed):

```rust
            let painted = Arc::new(AtomicBool::new(false));
            let painted_for_frame = painted.clone();
            let show_id = req.show_id;
            let reveal_title = title.clone();
            let reveal_all_titles = req.all_titles.to_vec();
            window.on_next_frame(move |window, cx| {
                painted_for_frame.store(true, Ordering::Release);
                super::platform::show_picker_window(&reveal_title, &reveal_all_titles);
                qol_runtime::probe!(
                    "SHOW_PAINTED",
                    "show_id={show_id} frame={}ms revealed=after_paint",
                    t_show.elapsed().as_millis()
                );
                let _ = (window, cx);
            });
```

Adjust the closure signature to the real `on_next_frame` callback types (read its definition in the vendored gpui first; the current code at reuse.rs:90 shows the arity). If `focus_for_keys` needs `&mut Window`/`Context`, route the after-show focus through the same mechanism the closure provides, or keep only the before-show focus if the after-show one was compensating for the early reveal - verify by keyboard-cycling immediately after open.

Then extend the existing 120ms watchdog (reuse.rs:98-109) into a reveal fallback:

```rust
            let painted_for_timeout = painted.clone();
            let timeout_title = title.clone();
            let timeout_all_titles = req.all_titles.to_vec();
            cx.spawn(move |_, cx: &mut AsyncApp| {
                let cx = cx.clone();
                async move {
                    cx.background_executor()
                        .timer(Duration::from_millis(120))
                        .await;
                    if !painted_for_timeout.load(Ordering::Acquire) {
                        super::platform::show_picker_window(&timeout_title, &timeout_all_titles);
                        qol_runtime::probe!("SHOW_PAINT_TIMEOUT", "show_id={show_id} after=120ms revealed=fallback");
                    }
                }
            })
            .detach();
```

`show_picker_window` must be idempotent for the double-call race (paint lands between the timeout check and the fallback call) - read its implementation in `picker/platform` and confirm; if it is not, guard with a second `AtomicBool` swap.

- [ ] **Step 3: Force a paint**

`apply_reuse` ends in `cx.notify()` via its delegate updates; verify a frame is actually scheduled while the window is still alpha-0 (macOS may throttle occluded windows - the alpha-0 ghost pattern in this codebase explicitly relies on continued rendering, see the ghost popup notes in the plugin CLAUDE.md). If `SHOW_PAINTED` never fires while hidden, call `window.refresh()` (or the equivalent explicit request used elsewhere in this plugin) right before registering `on_next_frame`.

- [ ] **Step 4: Full gate**

Run: `cd plugins/plugin-alt-tab && cargo fmt --all --check && cargo clippy --all-targets --all-features --keep-going -- -D warnings && cargo test`
Expected: green.

- [ ] **Step 5: Manual verification**

Restart the daemon. Repeatedly: change the window set while the picker is hidden, open the picker. The grid must appear fully formed - no card sliding/resizing after reveal. Time-to-correct-content is unchanged by design (the ghost window was already going to paint that same frame; we only stop flashing the stale one first), so if opens feel slower something is wrong - check `SHOW_PAINTED` timings against a pre-change trace. Also verify Alt+release immediately after Alt+Tab still activates the selected window.

- [ ] **Step 6: Commit**

```bash
git add plugins/plugin-alt-tab/src/picker/reuse.rs
git commit -m "fix(alt-tab): reveal reused picker only after the new frame paints"
```

---

### Task 5: SCK live-frame refill pop-in on every open

Re-aimed on 2026-07-06 trace evidence: `PREVIEW_GATHER missed=5` on shows 2 AND 3 with `PREVIEW_CAPTURE source=fill_sck` - the ScreenCaptureKit live-frame path re-captures the same windows on every open and commits them after reveal. Two distinct suspects, in order:

1. **Retention:** live frames should survive between shows (the whole point of the warm cache), but the show-path repeatedly reports the same windows uncovered. Investigate `plugins/plugin-alt-tab/src/shared/live_lanes.rs` (lane eviction) and `snapshot_live_frame_keys` in `plugins/plugin-alt-tab/src/picker/gather.rs:443` (it reads `view.delegate.live_frames` - confirm dismissal does not drain that map, and confirm `PREVIEW_GATHER`'s `missed` is not just accounting that ignores live frames).
2. **Commit batching:** `commit_live_frames_foreground` (`gather.rs:454`) inserts frames the moment each capture lands, after reveal - only the background warmer path goes through `FirstFillGate`. If retention is working as designed and refills are expected, batch this commit through a `FirstFillGate<capture::LiveFrame>` exactly like `app/live_preview.rs` does (read it fully first; follow its ownership pattern).

The old CG-path batching hypothesis (`commit_previews_foreground`) is dead for this machine: the trace shows the SCK path, not CG cache writes. Keep it in mind only for machines where `capture::live_shots_available()` is false.

**Files:**
- Investigate: `plugins/plugin-alt-tab/src/shared/live_lanes.rs`, `plugins/plugin-alt-tab/src/picker/gather.rs:383-472`, `plugins/plugin-alt-tab/src/app/live_preview.rs`
- Modify (per evidence): `plugins/plugin-alt-tab/src/picker/gather.rs` or `plugins/plugin-alt-tab/src/shared/live_lanes.rs`

**Interfaces:**
- Consumes: `FirstFillGate<T>` (`new(first_fill: bool)`, `admit(frames, visible) -> Option<Vec<(u32, T)>>`, `note_failure(wid)`, `take_pending()`), `fill_live_frames`, `commit_live_frames_foreground`.

- [ ] **Step 1: Determine retention vs. expected-refill (do not skip)**

Debug build. Open the picker twice with an unchanged window set, capture for the second open: `PREVIEW_GATHER` entries, `PREVIEW_CAPTURE source=fill_sck targets=N ids=[...]`, and the contents of `live_frames` at gather time (the `snapshot_live_frame_keys` set - add a temporary `#[cfg(debug_assertions)]` probe if needed). Decide:
- `live_frames` empty/shrunken at second show → retention bug; fix eviction/drain (suspect 1) and re-trace.
- `live_frames` warm but targets still selected → selection bug in `select_capture_targets_with_focus` coverage set; trace which id set it was given.
- Refill is by-design (e.g. frontmost refresh only) and covers exactly `refresh_frontmost`/`refresh_previous_frontmost` → pop-in is limited to idx 0/1; batch the commit (suspect 2) only if the visual pop-in is broader than those two cards.

- [ ] **Step 2: Write the failing test**

`FirstFillGate` is generic and pure - test the intended gather-side batching contract directly (add to `first_fill.rs` tests if absent):

```rust
    #[test]
    fn admits_nothing_until_all_visible_covered_then_everything_at_once() {
        let mut gate: FirstFillGate<u8> = FirstFillGate::new(true);
        assert_eq!(gate.admit(vec![(1, 10)], &[1, 2]), None);
        let out = gate.admit(vec![(2, 20)], &[1, 2]);
        assert_eq!(out, Some(vec![(1, 10), (2, 20)]));
    }
```

If this exact case already exists in `first_fill.rs`, skip to Step 3 - the test work for this task is then only the gather-side wiring being compile-checked.

- [ ] **Step 3: Wire the gate into the CG preview fill**

In `fill_previews` (`gather.rs:311`), batch the commit: instead of committing whatever subset was captured, only commit when the captured set plus already-cached ids covers every non-minimized visible window; otherwise hold via a `FirstFillGate<Arc<RenderImage>>` owned by the fill request path, and flush with `take_pending()` when the final wave lands. Mirror how `app/live_preview.rs` constructs and drives its gate (read it fully first; follow its ownership pattern exactly). Icons: apply the same batching in `fill_missing_icons` only if Step 1's evidence shows icons (not previews) popping - do not change both paths speculatively.

- [ ] **Step 4: Full gate**

Run: `cd plugins/plugin-alt-tab && cargo fmt --all --check && cargo clippy --all-targets --all-features --keep-going -- -D warnings && cargo test`
Expected: green.

- [ ] **Step 5: Manual verification**

Kill the daemon, relaunch, open the picker with 8+ windows: cards should appear fully populated in one paint (placeholder first paint is acceptable; per-card popping is not).

- [ ] **Step 6: Commit**

```bash
git add plugins/plugin-alt-tab/src
git commit -m "fix(alt-tab): batch cold-cache preview fill into one commit"
```

---

## Residual risks and out-of-scope

- Extreme window counts (60+ on a small monitor) still overflow at the `MIN_CARD_SCALE` floor; scrolling/pagination is explicitly out of scope.
- Tasks 3, 4, and 5 carry evidence gates that can end in "report findings" rather than a fix. That is a valid outcome - do not force the fix if evidence disagrees. Task 5 only runs if pop-in survives Task 4.
- Tasks 1-2 were implemented on 2026-07-06 (uncommitted at the time of writing); their steps remain for reference and for the commit.
- Preview capture ceiling (`PREVIEW_MAX_WIDTH/HEIGHT`) already covers `MAX_CARD_SCALE`; dynamic sizing needs no capture changes.
