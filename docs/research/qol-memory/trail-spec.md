# Trail: a focus that walks a line

Enter on a memory row opens a vertical trail.
Down walks backwards through where the fact came from.
The brass leaves the circle you are on, travels the line, and drains into the next one.
At rest exactly one circle is brass and every line is grey.

Accepted from the interactive prototype.
This spec is the implementation contract; it replaces nothing in `flow-ui-spec.md`, which stays the design note for the generic field ledger and is not built here.

## What is built

Three disjoint pieces.

1. `qol_gpui::trail::Trail`, a shared gpui element that knows about items, dots, lines and focus, and knows nothing about memory or the launcher.
2. The launcher dive: enter opens the trail over the selected flow row, esc ascends, up and down move the focus.
3. qol-memory emits a `trail` array on each flow row.

Nothing here changes the launcher-flows wire format, the plugin manifest, or any contract type in `qol-config`.

## Constraint that shapes everything

gpui has no CSS transitions and no compositor.
Motion is one `with_animations` chain that recomputes geometry from a delta each frame, and the whole thing is driven by arithmetic over fixed row heights.

Verified in `gpui-0.2.2/src/elements/animation.rs`:

- `with_animations(id, vec![a, b], |el, ix, delta| ...)` runs each `Animation` in order, advancing `animation_ix` as each one-shot finishes, and calls `window.request_animation_frame()` every frame until the last one is done.
- `AnimationState { start }` lives in element state keyed by the `GlobalElementId`, so a one-shot restarts only when the `ElementId` changes.

Both facts are load-bearing.
The id carries a move counter, so every keypress restarts the chain, and the phase boundary between the two animations *is* the arrival moment, which is why this design needs no timers at all.

Fixed row height is the second constraint.
Measuring node positions to place the line would need a prepaint pass; instead every node occupies one fixed slot and every dot centre is arithmetic.
The body text is clamped to three lines to fit that slot.

## Lane A: `libs/qol-gpui/src/trail/`

Owned paths: `libs/qol-gpui/src/trail/mod.rs`, `libs/qol-gpui/src/trail/model.rs`, `libs/qol-gpui/src/trail/motion.rs`, `libs/qol-gpui/src/lib.rs`.

### `model.rs`

```rust
use gpui::SharedString;

#[derive(Clone, Debug, PartialEq)]
pub struct TrailItem {
    pub at: SharedString,
    pub tag: SharedString,
    pub text: SharedString,
    pub struck: bool,
}

impl TrailItem {
    pub fn new(at: impl Into<SharedString>, tag: impl Into<SharedString>, text: impl Into<SharedString>) -> Self;
    pub fn struck(mut self, struck: bool) -> Self;
}
```

`at` and `tag` arrive already formatted.
The component performs no date parsing, no truncation of `at`, and no interpretation of `tag`.

### `motion.rs`

Pure, no gpui imports beyond `Pixels` arithmetic done as `f32`, fully unit-testable.

```rust
pub const ROW_H: f32 = 68.0;
pub const VISIBLE: usize = 3;
pub const DOT_OFFSET: f32 = 11.5;
pub const TRAVEL_MS: u64 = 420;
pub const DRAIN_MS: u64 = 230;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    Travel,
    Drain,
}

pub fn viewport_height() -> f32;

pub fn dot_center(index: usize) -> f32;

pub fn track_offset(selected: usize, len: usize) -> f32;

pub fn segment(from: usize, to: usize, phase: Phase, delta: f32) -> (f32, f32);

pub fn head_center(from: usize, to: usize, phase: Phase, delta: f32) -> Option<f32>;

pub fn lit_index(from: usize, to: usize, phase: Phase) -> usize;

pub fn slide(from: usize, to: usize, len: usize, phase: Phase, delta: f32) -> f32;
```

`viewport_height` is `VISIBLE as f32 * ROW_H`.

`dot_center(i)` is `i as f32 * ROW_H + DOT_OFFSET`.

`track_offset(selected, len)` puts the selected node in the middle slot and clamps at both ends:
the scrolled index is `selected.saturating_sub(1).min(len.saturating_sub(VISIBLE))`, and the result is that index negated times `ROW_H`.
A trail shorter than `VISIBLE` never scrolls.

`segment` returns `(top, height)` in track-local pixels, with `a = dot_center(from)` and `b = dot_center(to)`:

| direction | phase | top | height |
|---|---|---|---|
| `b > a` | Travel | `a` | `(b - a) * delta` |
| `b > a` | Drain | `a + (b - a) * delta` | `(b - a) * (1 - delta)` |
| `b < a` | Travel | `a - (a - b) * delta` | `(a - b) * delta` |
| `b < a` | Drain | `b` | `(a - b) * (1 - delta)` |
| `b == a` | either | `a` | `0` |

The line therefore grows out of the circle being left and empties into the circle being reached, and its height is zero at rest.

`head_center` is `Some(a + (b - a) * delta)` in `Travel` and `None` in `Drain`; the travelling head vanishes as the destination circle fills.

`lit_index` is `from` in `Travel` and `to` in `Drain`.
This is the whole reason the design needs no timers: the colour is on the line during travel and in the circle from the instant the drain begins.

`slide` interpolates `track_offset(from, len)` to `track_offset(to, len)` across `Travel` and holds `track_offset(to, len)` during `Drain`.

Tests to write in `motion.rs`, as a dense parameterised set:

- `segment` has zero height at `delta = 0` in Travel and at `delta = 1` in Drain, in both directions.
- `segment` spans exactly `dot_center(from)` to `dot_center(to)` at the phase boundary, in both directions.
- the segment's far edge never leaves the closed interval between the two dot centres, for a swept `delta`.
- `track_offset` is `0` while `selected <= 1`, and stops at `-(len - VISIBLE) * ROW_H` at the end.
- `track_offset` is `0` for any `selected` when `len <= VISIBLE`.
- `lit_index` flips exactly at the phase boundary.
- `slide` is continuous across the boundary.

### `mod.rs`

```rust
#[derive(IntoElement)]
pub struct Trail {
    id: ElementId,
    items: Vec<TrailItem>,
    from: usize,
    to: usize,
    seq: u64,
    palette: qol_theme::SystemPalette,
}

impl Trail {
    pub fn new(id: impl Into<ElementId>, items: Vec<TrailItem>) -> Self;
    pub fn focus(mut self, from: usize, to: usize) -> Self;
    pub fn seq(mut self, seq: u64) -> Self;
    pub fn palette(mut self, palette: qol_theme::SystemPalette) -> Self;
}

impl RenderOnce for Trail { ... }
```

`from` and `to` default to `0`; `seq` defaults to `0`.
The caller bumps `seq` on every focus change and passes the previous index as `from`.

Render shape:

- an outer `div` of height `motion::viewport_height()` with `overflow_hidden`.
- inside it one animated element carrying the whole track, built with:

```rust
.with_animations(
    (self.id.clone(), self.seq),
    vec![
        Animation::new(Duration::from_millis(motion::TRAVEL_MS)).with_easing(ease_in_out),
        Animation::new(Duration::from_millis(motion::DRAIN_MS)).with_easing(ease_out_quint()),
    ],
    move |track, ix, delta| { ... },
)
```

The `ix` is `0` for `Phase::Travel` and `1` for `Phase::Drain`.
The animator rebuilds the track each frame from `motion::slide`, `motion::segment`, `motion::head_center` and `motion::lit_index`.

Because the id carries `seq`, a focus change mints a fresh `GlobalElementId`, the stored `AnimationState` is absent, and the chain restarts from zero.
Because it is a one-shot chain, a settled trail stops requesting frames and idles at `delta = 1` of the drain, which is the correct resting state: empty line, one filled circle.

Colours come from the passed `SystemPalette`, never from a literal:
the spine is `slate` at the palette's line weight, the filled dot and the segment are the accent, and the struck body uses the muted tone.
Nothing in this module names a bone or midnight hex value.

Drawing details:

- the spine is one absolutely positioned `div`, `1.5px` wide, running from `dot_center(0)` to `dot_center(len - 1)`.
- the segment is a second absolutely positioned `div` at the `(top, height)` `motion::segment` returns.
- the head is a `7px` circle centred on `motion::head_center`, omitted when that is `None`.
- each node is a fixed `ROW_H` slot: a meta line of `at` and `tag`, then the body clamped with `.line_clamp(3)`.
- the focused node's body takes the primary text colour, every other node takes the muted colour, and a struck node draws its body with a strikethrough.
- element ids for the nodes are `(id, index)` so no two siblings collide.

The module ships one integration test in `libs/qol-gpui/tests/` only if a headless gpui harness already exists in this workspace; if it does not, `motion.rs` unit tests are the whole test surface and that is stated in the lane report rather than worked around.

`lib.rs` gains `pub mod trail;` in alphabetical position and `pub use trail::Trail;` beside the existing `Spinner` re-export.

## Lane B: launcher state and input

Owned paths: `plugins/launcher/src/ui/state.rs`, `plugins/launcher/src/ui/input.rs`, `plugins/launcher/src/ui/layout.rs`.

### `state.rs`

`FlowSession` gains one field:

```rust
pub view: FlowView,
```

initialised to `FlowView::List` in `enter_flow`.

```rust
pub enum FlowView {
    List,
    Trail(TrailFocus),
}

pub struct TrailFocus {
    pub row_index: usize,
    pub len: usize,
    pub from: usize,
    pub to: usize,
    pub seq: u64,
}
```

`TrailFocus` holds only indices; the item text is read from `FlowRow.raw` at render time.
It carries `len` so navigation can clamp without reaching back into the row.

Methods on `LauncherState`:

```rust
pub fn dive_flow_row(&mut self, len: usize) -> bool;
pub fn ascend_flow(&mut self) -> bool;
pub fn trail_move(&mut self, delta: i32) -> bool;
pub fn trail_focus(&self) -> Option<&TrailFocus>;
```

`dive_flow_row` takes the trail length, sets `view` to `Trail(TrailFocus { row_index: self.scroll_list.selected, len, from: 0, to: 0, seq: 0 })`, and returns false when there is no flow session or `len == 0`.

`ascend_flow` sets `view` back to `List` and returns whether it changed anything.

`trail_move` sets `from = to`, moves `to` by `delta` clamped to `0 .. len - 1`, bumps `seq`, and returns false when the index did not move, so a keypress at either end paints nothing.

`exit_flow` already replaces the whole session, so the trail dies with it and `reset_for_show` clears it on every window show.
No new teardown is needed.

### `input.rs`

`InputEffect` gains two variants:

```rust
FlowDive,
FlowAscend,
```

`apply_flow_key` forks at the top on the session's view.
When the view is `Trail`:

- `escape` returns `FlowAscend`.
- `up` and `down` call `trail_move(-1)` / `trail_move(1)` and return `Navigate` when it moved and `Ignore` when it did not.
- every other key returns `Ignore`.

The last rule is load-bearing.
The query string is still live underneath, and letting a keystroke reach `insert_char` while dived would mutate the query and schedule a fetch behind the user's back.

When the view is `List`, the existing arms are unchanged except `enter`, which now returns `FlowDive`.
`FlowActivate` stays in the enum and stays reachable from the row-action path in Lane C.

`flow_tab_is_ignored` in the existing test module keeps asserting that tab is ignored; add a sibling test asserting that a printable key while dived is `Ignore` and leaves `self.query` untouched.

### `layout.rs`

Add:

```rust
pub fn window_height_for_trail() -> f32;
```

returning `HEADER_HEIGHT + qol_gpui::trail::motion::viewport_height() + qol_gpui::theme::HEIGHT_HINT_BAR`.

## Lane C: launcher controller, render and view

Owned paths: `plugins/launcher/src/ui/controller.rs`, `plugins/launcher/src/ui/render.rs`, `plugins/launcher/src/ui/view.rs`, `plugins/launcher/src/flow/mod.rs`.

### `flow/mod.rs`

`trail` becomes the fourth reserved presentation key, beside `title`, `subtitle` and `copy`.

```rust
pub struct TrailNode {
    pub at: String,
    pub tag: String,
    pub text: String,
    pub struck: bool,
}

pub fn trail_of(raw: &serde_json::Value) -> Vec<TrailNode>;
```

`trail_of` reads `raw["trail"]` as an array of objects, keeps entries that carry a non-empty string `text`, and reads `at`, `tag` and `struck` with empty-string and `false` defaults.
A row with no `trail` key, a non-array `trail`, or an array with no usable entry yields a single node built from the row's own `title` and `subtitle`, so every row is divable and the launcher never has a dead enter key.

`parse_rows` is unchanged; `FlowRow.raw` already carries the whole object.

Unit tests: a well-formed trail parses in order; a missing `trail` yields exactly one node from title and subtitle; a `trail` holding a string, a number and one valid object yields exactly the valid object.

### `controller.rs`

Two new arms in `handle_key`:

- `InputEffect::FlowDive` calls a new `dive_flow_row(cx)` which resolves the selected `FlowRow`, computes `flow::trail_of(&row.raw)`, calls `state.dive_flow_row(nodes.len())`, resizes the window to `layout::window_height_for_trail()`, traces `trace::flow(self, "dived")` and notifies.
- `InputEffect::FlowAscend` calls `state.ascend_flow()`, resizes back to the list height, traces `"ascended"` and notifies.

`activate_flow_row` is unchanged and stays bound to the row-action path.

`start_flow_fetch`'s completion handler gains one guard: when the session view is `Trail`, the arriving rows still replace `session.rows`, but the view is left alone.
A fetch already in flight at dive time is the common case, since enter usually lands inside the 200ms debounce, and the dived frame must not be torn down under the user.
`TrailFocus` holds `row_index` only for the hint bar and the trace, never to re-resolve the row's content, so a replaced row set cannot corrupt the open trail.

### `render.rs`

`render` forks once, at the point where it currently builds flow rows.
When the view is `Trail`, it skips the row list entirely and renders `view::trail_body`.
The wheel handler, the visible-range computation and the nav-cue machinery are all list-only and stay behind that fork; they are never reached while dived.

`result_count` while dived is the trail length, so the existing `sync_result_window` call and the search bar counter both stay coherent.

The esc interception in the root key handler is unchanged: flow-active esc already routes through `handle_key`, which is where `FlowAscend` now lives.
Do not add a third branch to that handler.

`trace::RenderSignature` gains the focused trail index so render dedupe does not collapse two distinct dived frames.

### `view.rs`

```rust
pub fn trail_body(
    kit: &qol_gpui::kit::Kit,
    nodes: &[crate::flow::TrailNode],
    focus: &crate::ui::state::TrailFocus,
) -> impl IntoElement;
```

It maps `TrailNode` to `qol_gpui::trail::TrailItem`, builds `Trail::new(("flow-trail", focus.row_index), items).focus(focus.from, focus.to).seq(focus.seq).palette(kit.palette)`, and returns it.
This function contains no motion logic and no colour literals.

The search bar counter shows `to + 1 / len` while dived.
The hint bar while dived reads `down back in time`, `up forward`, `ctrl-y copy`, `esc back`.

## Lane D: qol-memory emits the trail

Owned path: `plugins/qol-memory/src/ask/rows.rs`.

`FlowRow` gains one field:

```rust
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub trail: Vec<TrailEntry>,

pub struct TrailEntry {
    pub at: String,
    pub tag: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub struck: bool,
}
```

`at` is the already-formatted date the existing `date_of` helper produces, so the launcher formats nothing.

For the answer row, the trail is, in order: the answer itself tagged `true now`, then each entry of `answer.superseded` tagged `superseded` with `struck` true, oldest last.
For a capture or recalled unit row, the trail is that unit tagged with its own kind.
When there is no history the trail is a single entry, which the launcher renders as a one-dot trail.

Emitting a trail is additive: `FlowRow` keeps `title`, `subtitle`, `copy`, `key` and `kind` exactly as they are, and a consumer that ignores `trail` sees no change.

Tests: an answer carrying two superseded entries produces three trail entries in recency order with the two older ones struck; a row with no history produces exactly one entry; the field is absent from the serialised JSON when empty.

## Sequencing

Lane A first and alone.
Lanes B, C and D are disjoint and run together in the second round, against A's landed signatures.

## Acceptance

1. `qol_gpui::trail` compiles with no dependency added to `libs/qol-gpui/Cargo.toml` and names no plugin, no memory concept and no theme hex value.
2. `motion.rs` tests cover the table above and pass.
3. In the launcher, enter on a memory row opens the trail, esc returns to the list, a second esc leaves the flow.
4. Down moves the focus: the brass leaves the current circle, travels the line, and fills the next one, with the line empty at rest and exactly one circle filled.
5. Typing while dived changes nothing and schedules no fetch.
6. A row with no `trail` key still opens, as a single node.
7. The full gate is green: `cargo fmt --all -- --check`; `cargo +1.98.0 clippy -p qol-gpui -p launcher -p qol-memory --features qol-tray/dev --all-targets -- -D warnings`; `cargo test -p qol-gpui -p launcher -p qol-memory`; `env -u QOL_TRAY_HTTP_TOKEN cargo run -q -p qol -- check`.
8. The motion is verified in a guest, not on the host session, per the runtime-behavior rule.

## Non-goals

The generic field ledger from `flow-ui-spec.md`.
Mouse support.
Row actions while dived beyond the existing copy binding.
Variable row heights.
Any change to `RowActionSpec`, the manifest, or `launcher-flows.json`.

## Revision 2: the flow list is the trail

The per-row dive is withdrawn.
In flow mode the query's answers are the trail: rows arrive, they render as one node each on the vertical trail, the selected row is the lit circle, down and up move the focus with the travel and drain motion, enter runs the row action, esc leaves the flow.
There is no `FlowView`, no dive, no ascend, no `FlowDive` or `FlowAscend` effect, and no ctrl-y binding.
Lane A (`libs/qol-gpui/src/trail`) is unchanged.
Lane D (`plugins/qol-memory`) is unchanged.

### Order

`parse_rows` in `plugins/launcher/src/flow/mod.rs` stable-sorts the parsed rows newest first by the `at` of the first `trail` entry, taken as a plain string compare on the `YYYY-MM-DD` date; rows whose first trail entry is missing or has an empty `at` keep their relative order and go last.
Down therefore moves back in time, matching the hint bar.

### Node

One node per row: `trail_of(&row.raw)[0]`, which always exists because `trail_of` falls back to title and subtitle.
`TrailItem::new(node.at, node.tag, node.text).struck(node.struck)`.

### Lane B2: `plugins/launcher/src/ui/{state.rs,input.rs,layout.rs}`

`state.rs`: delete `FlowView`, `dive_flow_row`, `ascend_flow`, `trail_move`, `trail_focus`, and the `view` field.
`TrailFocus` becomes:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrailFocus {
    pub from: usize,
    pub to: usize,
    pub seq: u64,
}
```

`FlowSession` gains `trail_from: usize`, `trail_last: usize`, `trail_seq: u64`, all zero in `enter_flow`.
One new method replaces the deleted ones:

```rust
pub fn flow_trail_focus(&mut self) -> Option<TrailFocus>
```

It returns `None` without a flow.
With a flow it compares `self.scroll_list.selected` with `trail_last`; when they differ it sets `trail_from = trail_last`, `trail_last = selected`, `trail_seq += 1`.
It then returns `TrailFocus { from: trail_from, to: trail_last, seq: trail_seq }`.
Called once per render after `sync_result_window`, so every selection path (keys, wheel, query reset) drives the motion without per-call bookkeeping.
Tests: a first call returns `from 0 to 0 seq 0`; after `scroll_list.selected` moves to 2 the next call returns `from 0 to 2 seq 1` and a repeat call returns the same value unchanged; without a flow the call returns `None`.

`input.rs`: restore `apply_flow_key` to its shape before the trail landed: no dived branch, `enter` returns `FlowActivate`, remove the `FlowDive` and `FlowAscend` variants, restore the test name `flow_escape_exits_and_enter_activates` asserting `FlowActivate`, and delete the tests `printable_key_while_dived_is_ignored_and_leaves_query_untouched`, `dived_trail_moves_with_up_down_and_escape_ascends`, and `dived_trail_ctrl_y_activates_and_plain_y_is_ignored`.

`layout.rs`: `window_height_for_trail()` stays as is.

### Lane C2: `plugins/launcher/src/ui/{controller.rs,render.rs,view.rs,trace.rs}` and `plugins/launcher/src/flow/mod.rs`

`controller.rs`: delete `dive_flow_row`, `ascend_flow`, the two match arms, the `FlowView` import, the `px`, `size`, layout imports if nothing else uses them, and restore the completion handler in `start_flow_fetch` to `if let Some(message) = failure { view.state.set_launch_error(message); }` with no view guard.

`render.rs`: delete `trail_focus` and `dived`.
`result_count` is `flow_result_count()` in flow mode as before.
After `sync_result_window`, `let trail_focus = self.state.flow_trail_focus();`.
`target_height` in flow mode is `window_height_for_trail()` when `result_count > 0` and `window_height_for(0, FLOW_ROW_HEIGHT)` otherwise; the non-flow branch is unchanged.
`rows` is `Vec::new()` in flow mode.
The results child in flow mode is a `div().id("launcher-results").h(px(results_height)).w_full().overflow_hidden().bg(view::bg_color())` carrying an `on_scroll_wheel` listener that computes `qol_gpui::scroll_list::wheel_rows(&event.delta, qol_gpui::trail::motion::ROW_H)` and moves the selection that many rows, `scroll_list.move_down(result_count)` per positive row and `scroll_list.move_up()` per negative row, then `cx.notify()`, with the single child `view::trail_body(&kit, &session.rows, focus)` where `session` is the flow session and `focus` the `TrailFocus`.
The non-flow results child is unchanged.
The search bar keeps `self.state.scroll_list.selected`.
The hint bar in flow mode is `view::hint_bar_flow(entry)`; there is no `hint_bar_trail`.

`view.rs`: `trail_body(kit: &Kit, rows: &[FlowRow], focus: TrailFocus) -> Trail` builds items as in Node above and returns `Trail::new("flow-trail", items).focus(focus.from, focus.to).seq(focus.seq).palette(kit.palette)`.
`hint_bar_flow` shows, in order, enter with the row action label or `copy`, down `back in time`, up `forward`, esc `back`, then the entry chip and the flex spacer.
Delete `hint_bar_trail`.

`trace.rs`: `effect_label` loses the `flow_dive` and `flow_ascend` arms; `RenderSignature.trail_index` is `view.state.scroll_list.selected` when a flow is active and `0` otherwise.

`flow/mod.rs`: keep `TrailNode`, `trail_of`, and their tests; add the sort to `parse_rows` per Order above with one test: three rows dated `2026-08-01`, `2026-08-12`, undated, plus one dated `2026-08-05`, parse in the order `08-12`, `08-05`, `08-01`, undated.

### Acceptance

1. `cargo test -p launcher` green, `clippy -D warnings` green on `qol-gpui`, `launcher`, `qol-memory`.
2. Typing in the qol memory flow renders the answers as the trail directly, newest at the top, three visible.
3. Down and up move the brass along the line into the next circle; the line is empty at rest and exactly one circle is lit.
4. Enter copies or runs the row action as before; esc leaves the flow.

### Lane A2: `libs/qol-gpui/src/trail/{mod.rs,motion.rs}` match the accepted prototype

The prototype (artifact 7a7821a3, "The Provenance Trail") is the visual contract; the shipped component drifted from it.
The launcher font stays; everything else follows the prototype.

`motion.rs`:

```rust
pub const ROW_H: f32 = 82.0;
pub const VISIBLE: usize = 3;
pub const PAD_TOP: f32 = 18.0;
pub const DOT_OFFSET: f32 = PAD_TOP + 11.5;
```

`dot_center`, `track_offset`, `segment`, `head_center`, `slide`, `viewport_height` keep their signatures and formulas (`viewport_height` is now 246).
`lit_index` is replaced by three functions:

```rust
pub fn lit(from: usize, to: usize, phase: Phase) -> Option<usize>;
pub fn here(from: usize, to: usize, phase: Phase) -> usize;
pub fn fill(phase: Phase, delta: f32) -> f32;
```

`lit` is `None` in `Travel` (the brass has left the circle and nothing is lit while it travels) and `Some(to)` in `Drain`.
`here` is `from` in `Travel` and `to` in `Drain`; it drives the text state.
`fill` is `0.0` in `Travel` and `delta` in `Drain`: how far the destination circle has filled.
Replace the `lit_index` test with one that asserts all three for `(0, 4)`, `(4, 0)`, `(2, 2)`.
The other tests stay and must still pass with the new constants.

`mod.rs` geometry, all in track-local pixels:

```rust
const PAD_X: f32 = 14.0;
const DOT_SIZE: f32 = 11.0;
const DOT_LIT_SIZE: f32 = 14.0;
const GLOW_PAD: f32 = 4.0;
const HEAD_SIZE: f32 = 7.0;
const LINE_WIDTH: f32 = 1.5;
const LINE_CX: f32 = PAD_X + 6.0;
const TEXT_LEFT: f32 = PAD_X + 24.0;
const META_LINE_HEIGHT: f32 = 15.0;
const BODY_LINE_HEIGHT: f32 = 18.0;
const META_GAP: f32 = 8.0;
const META_BODY_GAP: f32 = 2.0;
```

Layout: the node column is `flex_col` with `pt(px(motion::PAD_TOP))`; each node is `h(px(motion::ROW_H))`, `relative`, `w_full`; its dot sits at `top 6px` inside the node so its centre lands on `motion::dot_center(index)`.
Text column: `left TEXT_LEFT`, `right PAD_X`, `top_0`.

Dot per node `n`, given `lit: Option<usize>` and `fill: f32`:
- lit is `Some(n)`: size `DOT_SIZE + (DOT_LIT_SIZE - DOT_SIZE) * fill`, centred on the dot centre (`left LINE_CX - size / 2`, `top 11.5 - size / 2`), `rounded_full`, `bg rgb(palette.accent)`; behind it a glow ring of size `size + 2 * GLOW_PAD`, centred the same way, `rounded_full`, `bg` = `Hsla::from(rgb(palette.accent)).opacity(0.22 * fill)`.
- otherwise: size `DOT_SIZE`, `rounded_full`, `bg rgb(palette.surface_elevated)`, `.border(px(1.5)).border_color(rgb(palette.border_subtle))`, a hollow ring.

Text per node `n`, given `here: usize`:
- `at`: `TEXT_NANO`, `text_muted`; when `n == here`, `text_secondary`.
- `tag`: rendered uppercase (`to_uppercase()` on the string), `TEXT_NANO`, `text_muted`; when `n == here`, `accent_ink`.
- body: `TEXT_MICRO`, line height `BODY_LINE_HEIGHT`, `line_clamp(3)`; `n == here` gives `text_primary`; `n < here` gives `text_secondary`; `n > here` gives `text_muted` with `.opacity(0.72)`; `struck` items always `text_muted` plus `line_through`.

Spine, segment, and head are unchanged in role: the spine is `border_subtle` from `dot_center(0)` to `dot_center(len - 1)`, the segment is the accent segment from `motion::segment`, the head is a `HEAD_SIZE` accent circle at `motion::head_center` and only exists in `Travel`; all three sit on `LINE_CX`.
The animator reads `motion::lit`, `motion::here`, and `motion::fill` each frame instead of `lit_index`.
No hex literals; every colour comes from `SystemPalette`.
Verified gpui 0.2.2 facts: `.border(px(1.5))` and `.border_color(..)` are valid on `Div`; `Hsla: From<Rgba>` and `Hsla::opacity(f32)` exist; `.opacity(f32)` exists on `Styled`; there is no text transform, so uppercase the string.

## Revision 3: faster motion, interruptible travel, and a detail view on enter

Three changes: the motion gets quicker, a keypress during a move continues from where the brass actually is instead of snapping, and enter on a row opens that memory's full text and metadata instead of copying.

### Lane A3: `libs/qol-gpui/src/trail/{motion.rs,mod.rs}`

`motion.rs` owns the easing so the launcher can ask where the brass is mid-flight:

```rust
pub const TRAVEL_MS: u64 = 260;
pub const DRAIN_MS: u64 = 140;

pub fn ease_travel(delta: f32) -> f32 {
    if delta < 0.5 {
        2.0 * delta * delta
    } else {
        let x = -2.0 * delta + 2.0;
        1.0 - x * x / 2.0
    }
}

pub fn ease_drain(delta: f32) -> f32 {
    1.0 - (1.0 - delta).powi(5)
}
```

The departure point becomes fractional, because an interrupted move restarts from the pixel the head had reached:

```rust
pub fn dot_center(index: f32) -> f32;
pub fn track_offset(pos: f32, len: usize) -> f32;
pub fn segment(from: f32, to: usize, phase: Phase, delta: f32) -> (f32, f32);
pub fn head_center(from: f32, to: usize, phase: Phase, delta: f32) -> Option<f32>;
pub fn slide(from: f32, to: usize, len: usize, phase: Phase, delta: f32) -> f32;
pub fn lit(to: usize, phase: Phase) -> Option<usize>;
pub fn here(from_index: usize, to: usize, phase: Phase) -> usize;
pub fn fill(phase: Phase, delta: f32) -> f32;
pub fn position_at(from: f32, to: usize, elapsed_ms: u64) -> f32;
```

`dot_center` is `index * ROW_H + DOT_OFFSET`, unchanged apart from the type.
`track_offset` is `-(pos - 1.0).clamp(0.0, len.saturating_sub(VISIBLE) as f32) * ROW_H`, which reduces to the old integer behaviour at whole positions.
`segment`, `head_center` and `slide` keep their formulas with `a = dot_center(from)` and `b = dot_center(to as f32)`.
`lit` drops its unused `from` parameter: `None` in `Travel`, `Some(to)` in `Drain`.
`here` and `fill` are unchanged.
`position_at` answers where the head is after `elapsed_ms` of a move: `from + (to as f32 - from) * ease_travel(elapsed_ms as f32 / TRAVEL_MS as f32)` while `elapsed_ms < TRAVEL_MS`, and `to as f32` at or after it.

Tests: keep the existing ones, adapting the call sites to `f32`, and add three.
`position_at` returns `from` at 0 ms, `to` at `TRAVEL_MS` and beyond, and a value strictly between the two at `TRAVEL_MS / 2`.
`track_offset` at a fractional position lies strictly between its two neighbouring whole positions.
`segment` and `head_center` from a fractional `from` still stay inside the closed interval between the two dot centres.

`mod.rs` passes `motion::ease_travel` and `motion::ease_drain` to the two `Animation`s (`.with_easing(motion::ease_travel)`), so the delta the animator receives is the same curve the launcher uses to locate an interrupted head.
The `Trail` builder takes the fractional departure:

```rust
pub fn focus(mut self, from: f32, from_index: usize, to: usize) -> Self;
```

`from` feeds the geometry, `from_index` feeds `motion::here`, `to` feeds both.
Every other part of the element is unchanged.

### Lane B3: `plugins/launcher/src/ui/{state.rs,input.rs,layout.rs}`

`state.rs`:

```rust
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrailFocus {
    pub from: f32,
    pub from_index: usize,
    pub to: usize,
    pub seq: u64,
}
```

`FlowSession` replaces `trail_from`, `trail_last` and `trail_seq` with `trail_from: f32`, `trail_from_index: usize`, `trail_to: usize`, `trail_seq: u64`, `trail_started: Option<std::time::Instant>`, and gains `detail: bool`; `enter_flow` sets them to `0.0`, `0`, `0`, `0`, `None`, `false`.

`flow_trail_focus` becomes interrupt-aware:

```rust
pub fn flow_trail_focus(&mut self) -> Option<TrailFocus> {
    let selected = self.scroll_list.selected;
    let session = self.flow.as_mut()?;
    if session.trail_to != selected {
        let elapsed = session
            .trail_started
            .map_or(u64::MAX, |start| start.elapsed().as_millis() as u64);
        session.trail_from =
            qol_gpui::trail::motion::position_at(session.trail_from, session.trail_to, elapsed);
        session.trail_from_index = session.trail_to;
        session.trail_to = selected;
        session.trail_seq += 1;
        session.trail_started = Some(std::time::Instant::now());
    }
    Some(TrailFocus { .. })
}
```

A move that lands while the previous one is still travelling therefore starts from the pixel the head had reached, never from the circle it had not arrived at.

Three detail methods, all returning whether anything changed:

```rust
pub fn open_flow_detail(&mut self) -> bool;
pub fn close_flow_detail(&mut self) -> bool;
pub fn flow_detail_open(&self) -> bool;
```

`open_flow_detail` returns false without a flow or when already open; `close_flow_detail` mirrors it; `exit_flow` needs no change since the whole session is dropped.

Tests: the existing `flow_trail_focus` tests keep asserting `from_index`, `to` and `seq` (time-independent); add one asserting that a second move without an intervening delay leaves `from` at or before the new `from_index` as a float, and three asserting the detail methods' open, close and idempotence.

`input.rs`: `InputEffect` gains `FlowDetail` and `FlowDetailClose`.
In `apply_flow_key`, before the existing match, when `self.flow_detail_open()`:

```rust
"escape" | "esc" => InputEffect::FlowDetailClose,
"enter" => InputEffect::FlowActivate,
"up" => { self.move_up(); InputEffect::Navigate }
"down" => { self.move_down(result_count); InputEffect::Navigate }
_ => InputEffect::Ignore,
```

so the detail follows the trail while it is open and typing cannot disturb the query behind it.
In the list branch, `"enter"` returns `InputEffect::FlowDetail` instead of `FlowActivate`; every other arm is unchanged.
Tests: enter in the flow list yields `FlowDetail`; with the detail open escape yields `FlowDetailClose`, enter yields `FlowActivate`, a printable key yields `Ignore` and leaves the query untouched, and down yields `Navigate`.

`layout.rs`:

```rust
pub const DETAIL_HEIGHT: f32 = 380.0;
pub fn window_height_for_detail() -> f32 {
    HEADER_HEIGHT + DETAIL_HEIGHT + qol_gpui::theme::HEIGHT_HINT_BAR
}
```

### Lane C3: `plugins/launcher/src/ui/{controller.rs,render.rs,view.rs,trace.rs}` and `plugins/launcher/src/flow/mod.rs`

`flow/mod.rs`: `pub fn detail_of(raw: &serde_json::Value) -> Vec<(String, String)>` reads the row's `detail` array, keeping entries whose `label` and `value` are both non-empty strings, in order, and returning an empty vector when the key is missing or unusable.
One test covers a well-formed array, a missing key, and an array holding a string, a number and one valid object.

`controller.rs`: two handlers, both resizing the window and tracing:
`open_flow_detail` returns early unless `self.state.open_flow_detail()`, then resizes to `window_height_for_detail()` and traces `detail_open`;
`close_flow_detail` returns early unless `self.state.close_flow_detail()`, then resizes to `window_height_for_trail()` when the flow has rows and `window_height_for(0, FLOW_ROW_HEIGHT)` when it does not, and traces `detail_close`.
Two new match arms dispatch to them.
`activate_flow_row` is unchanged and stays bound to `FlowActivate`.

`render.rs`: `let detail = self.state.flow_detail_open();`.
When `detail` is true and the flow session has a row at `scroll_list.selected`, `target_height` is `window_height_for_detail()`, the results child is `view::detail_body(&kit, row, results_height)` with no wheel listener, and the hint bar is `view::hint_bar_detail()`.
Otherwise the flow branch is exactly as it is today.

`view.rs`:

```rust
pub fn detail_body(kit: &qol_gpui::kit::Kit, row: &FlowRow, height: f32) -> Div;
pub fn hint_bar_detail() -> Div;
```

`detail_body` is a `flex_col` of height `height`, `overflow_hidden`, padded `14px` on each side and `16px` top, `gap 14px`:
first the full text from `row.copy` falling back to `row.title`, at `TEXT_MICRO`, line height `18px`, `text_primary`, `line_clamp(12)`;
then, when `crate::flow::detail_of(&row.raw)` is non-empty, a `flex_col` with `gap 5px` of one row per field, each a `flex` with `gap 10px` holding the uppercase label at `TEXT_NANO` in `text_muted` with a fixed `w(px(92.0))`, and the value at `TEXT_NANO` in `text_secondary`, `flex_1`, `truncate`.
No colour literals; every tone comes from `kit.palette`.
`hint_bar_detail` shows enter `copy`, up and down `move`, esc `back`, then a flex spacer.

`trace.rs`: `effect_label` gains `InputEffect::FlowDetail => "flow_detail"` and `InputEffect::FlowDetailClose => "flow_detail_close"`.

### Lane D3: `plugins/qol-memory/src/ask/rows.rs`

`FlowRow` gains one field beside `trail`:

```rust
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub detail: Vec<DetailField>,

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DetailField {
    pub label: String,
    pub value: String,
}
```

Fields are emitted in the order below, and any whose value is absent or empty is skipped, so the launcher renders whatever is there without knowing the row kinds.

Answer row: `verdict` (`output.verdict`), `confidence` (`output.confidence`), `layer` (`answer.layer`), `class` (`answer.cls`), `source` (`answer.source_kind`), `when` (`answer.source_ts`, the full timestamp, not `date_of`), `score` (two decimals), `margin` (two decimals), `session`, `key`.
Capture and unit rows: `kind` (`unit.kind`), `when` (`unit.ts`), `session`, `cwd`, `score` (two decimals), `key`.
Recalled rows: `kind`, `when` (`recall.source_ts`), `score`, `key`.
Skill rows: `skill` (`hit.id`), `section`, `status`, `head`, `dirty`.

Tests: an answer row carries verdict, confidence and key in that relative order and omits an absent margin; a capture row carries its kind, timestamp and key and omits an absent cwd; `detail` is absent from the serialised JSON when empty.

### Acceptance

1. `cargo fmt --all -- --check`, `clippy -D warnings` and the tests are green for `qol-gpui`, `launcher` and `qol-memory`, and `qol check` passes.
2. Holding down through the trail is continuous: the brass never jumps backwards to a circle it had not reached, and the track never snaps.
3. A single press completes in about 400 ms rather than 650 ms.
4. Enter on a row opens that memory's full text with its metadata; esc returns to the trail; up and down move while the detail is open; enter there copies and hides.

## Revision 4: the trail does not replay its animation when it comes back

Opening a memory's detail and pressing esc replays the travel and drain as though the focus had just moved.
The cause is not the launcher's focus bookkeeping, which is stable across the trip: it is that gpui keys `AnimationState` by `GlobalElementId` and drops the state of any element not painted in a frame.
While the detail is open the `Trail` element is not painted at all, so on return the animation element is constructed with no prior state, takes the `start: Instant::now(), animation_ix: 0` branch, and runs the whole chain again.
The same defect makes the trail animate once when a query's rows first arrive.

The fix is to stop animating at all once the move is over, so there is no animation state left to lose.

### Lane A4: `libs/qol-gpui/src/trail/{motion.rs,mod.rs}`

`motion.rs` gains one constant:

```rust
pub const SETTLE_MS: u64 = TRAVEL_MS + DRAIN_MS;
```

`mod.rs` grows a builder and a resting path:

```rust
pub fn settled(mut self, settled: bool) -> Self;
```

Extract the closure body that builds the track into a free function so both paths share it verbatim:

```rust
fn track(
    track: Div,
    items: &[TrailItem],
    node_ids: &[ElementId],
    from: f32,
    from_index: usize,
    to: usize,
    len: usize,
    phase: motion::Phase,
    delta: f32,
    palette: SystemPalette,
) -> Div;
```

The animator calls it with the phase and delta gpui hands it.
When `self.settled` is true the element skips `with_animations` entirely and calls `track(..)` once with `motion::Phase::Drain` and delta `1.0`, which is exactly the frame the chain ends on: an empty segment, no travelling head, `lit` on `to`, `fill` at `1.0`, and the track parked at `track_offset(to)`.
A settled trail therefore paints one static frame, requests no animation frames, and holds no animation state to lose.

### Lane B4: `plugins/launcher/src/ui/state.rs`

`TrailFocus` gains `pub settled: bool`.
`flow_trail_focus` computes it from the same stamp it already keeps, after the interrupt block:

```rust
let settled = session
    .trail_started
    .is_none_or(|start| start.elapsed().as_millis() as u64 >= motion::SETTLE_MS);
```

so a session that has never moved is settled, a move in flight is not, and a move whose full travel and drain have elapsed is settled again.
Tests: a fresh flow reports `settled` true; the focus returned immediately after a selection change reports it false.

### Lane C4: `plugins/launcher/src/ui/view.rs`

`trail_body` passes the flag through: `.settled(focus.settled)` after `.seq(focus.seq)`.

### Acceptance

1. `cargo fmt --all -- --check`, `clippy -D warnings` and the tests are green for `qol-gpui`, `launcher` and `qol-memory`, and `qol check` passes.
2. Entering a memory's detail and pressing esc returns to the trail with the focused circle already filled and nothing moving.
3. A query's first rows appear with the top circle already filled, with no travel from nowhere.
4. Pressing down still travels and drains exactly as before.
