# CLI Sessions - UI Upgrade + Architecture Docs

Date: 2026-06-20
Plugin: `plugins/plugin-cli-sessions`

## Context

`plugin-cli-sessions` renders an always-on-top gpui panel listing live CLI
sessions discovered in a terminal host. Each row shows project/name, branch, a
tool tag (Claude/Codex), a status-colored summary, and elapsed time. Status
drives both color and sort order.

The panel works but is visually thin: keyboard shortcuts are undiscoverable, the
header shows only a bare count, status is conveyed by color alone, and a tracked
field (`running_since`) is never shown. Separately, the plugin's two-axis
"strategy" architecture (terminal host vs. tool flavor) is undocumented, and
three declared config fields are silently ignored by the code.

The visual direction was validated against live HTML mockups during
brainstorming. The palette and row look are explicitly **kept as-is** - this
upgrade adds information and affordances, it does not restyle.

## Goals

1. Layer five information/affordance improvements into the panel without
   changing the existing palette or row layout.
2. Make `running_since` meaningful for every tool (today only the generic CLI
   strategy populates it).
3. Honor the `corner` config field, which currently does nothing.
4. Document the strategy architecture in an in-plugin `ARCHITECTURE.md`, keeping
   the code comment-free per repo rule.

## Non-goals

- No color/palette/contrast changes. The status colors and row tints are fixed.
- No wiring of the `poll_secs` or `host` config fields (also dead today, but out
  of scope - noted in ARCHITECTURE.md as known gaps).
- No multi-monitor "follow the active monitor" behavior; place once on open.

## Part 1 - Panel visual upgrade (`src/ui/render.rs`)

All five changes live in the render layer plus small supporting pure functions.
The row's two-line body (identity line + status line) is preserved.

### 1.1 Jump-number gutter

A narrow left gutter (`~22px`, right-bordered `#21262d`) shows each row's 1-based
index. The `1`-`9` jump shortcut already exists in the key handler; this makes it
discoverable.

- Number color `#6e7681`; on the selected row it brightens to the selection blue
  `#58a6ff`.
- Rows at index 10+ render a blank gutter (the shortcut only covers `1`-`9`).
- The existing 2px selection border on the row's left edge is retained.

### 1.2 Header status summary

Replace the bare count on the right of the header with one colored
disc + count per **non-empty** status group, in sort-rank order:

| Group | Disc color | Statuses folded in |
|-------|-----------|--------------------|
| Needs you | `#f85149` | `NeedsYou` |
| Your turn | `#d29922` | `YourTurn` |
| Working | `#3fb950` | `Working` |
| Idle | `#6e7681` | `Unknown`, `Acknowledged` |

Groups with zero count are omitted. A pure helper
`status_summary(rows) -> Vec<(Status group, usize)>` is unit-tested.

### 1.3 Status glyphs

Add a non-color channel so state reads without relying on hue. A small glyph
precedes the summary on the status line:

- `NeedsYou` -> `!` (`#f85149`)
- `YourTurn` -> stays the existing clickable acknowledge pill (`your turn ✓`)
- `Working` -> `◐` (`#3fb950`), animated if feasible (see Open details)
- `Unknown`/`Acknowledged` -> `·` (`#6e7681`)

### 1.4 Working duration (+ centralized `running_since`)

For `Working` rows the status line's right value shows run duration as
`running {dur}` (e.g. `running 12m`), computed from `running_since` with the
existing `format_elapsed` bucketing. Non-working rows keep showing
last-activity elapsed.

`running_since` is currently set by the default `Cli` strategy but hardcoded to
`None` by `Claude` and `Codex`, so today no agent row ever has a duration. Fix
by **centralizing** the computation:

- Remove `running_since` from `strategy::Reading`.
- Add `running_since_for(prev_running: Option<u64>, phase: Phase, now: u64) -> Option<u64>`:
  `Busy` -> `Some(prev_running.unwrap_or(now))`, every other phase -> `None`.
- Call it in `reconcile::apply` from `reading.phase`, so all tools behave
  identically and no strategy can forget it.

This is the "make invalid states unrepresentable" fix: duration tracking is a
property of the phase, not of each tool's `read`.

### 1.5 Keybind footer

A thin footer bar (top-bordered `#21262d`, bg `#0d1117`) lists the live keys:
`↑↓ move · ⏎ jump · a ack · esc close`. Keys rendered as subtle `kbd` chips
(`#c9d1d9` on `rgba(255,255,255,.06)`), labels in `#6e7681`. Keyboard-first is a
repo hard rule; this surfaces the existing flow.

## Part 2 - Honor the `corner` config (`src/ui/run.rs`)

Today `panel_window_options` calls `Bounds::centered(...)` and never reads
config, so the "Screen corner" setting is inert.

- Add `src/config.rs` with `CliSessionsConfig { corner: String }`
  (`#[serde(default)]`, default `"top-right"`), loaded via
  `qol_config::load_plugin_config_from_env(PLUGIN_ID)` (mirrors
  `plugin-launcher`).
- Parse into a `Corner` enum (`TopLeft|TopRight|BottomLeft|BottomRight`) at the
  boundary; unknown strings fall back to `TopRight`.
- Add pure `corner_bounds(monitor: Bounds, win: Size, corner: Corner, margin: f32) -> Bounds`
  computing the inset origin per corner. Unit-tested (table-driven).
- In `run.rs`, resolve the monitor via `MonitorTracker`/`ActiveMonitor::bounds()`
  (fall back to `Bounds::centered` when no monitor is known) and place the window
  with `corner_bounds`.

Margin is a small constant (e.g. `16px`) accounting for menubar/panel insets.

## Part 3 - `ARCHITECTURE.md` (plugin root)

A single in-plugin doc. Code stays comment-free. Sections:

1. **Overview** - what the panel is and the data flow:
   `host.discover -> classify tool -> strategy.read -> Phase -> status_for -> Status -> UI`.
2. **Two strategy axes** with an ASCII diagram:
   - **Terminal host** (`host::TerminalHost`): `discover` / `get_text` / `focus`.
     Kitty is the only impl; nothing else references kitty. Selected by the
     `host` config (currently always kitty).
   - **Tool strategy** (`strategy::Strategy`): default `Cli` = standard terminal
     behavior; `Claude` / `Codex` override `read`/`label`/`wants_screen` to detect
     the same phases from tool-specific on-screen tells and metadata.
3. **Phase vs Status** - the seam. `Phase {Busy,Blocked,Done,Idle}` is the
   universal terminal reading; `status_for(prev, phase)` folds in memory
   (`Acknowledged` stickiness) to yield the user-facing
   `Status {Working,YourTurn,NeedsYou,Unknown,Acknowledged}`. Include the mapping
   table.
4. **Signals** - `signal::screen` / `signal::title` are the shared primitives
   (prompt markers, input requests, tool working/done detectors) strategies
   compose from.
5. **Recipes**:
   - *Add a terminal host*: implement `TerminalHost`, wire selection in `run.rs`.
   - *Add a tool strategy*: add a `Tool` variant + `classify` rule, implement
     `Strategy`, register in `for_tool`.
6. **Known gaps** - `poll_secs` and `host` config fields are declared but not yet
   consumed.

## Testing

Per the qol-gpui rule, do **not** create live popup windows in tests. Cover the
pure logic; verify the rendered panel visually.

- `corner_bounds` - table-driven: each corner -> expected origin for a known
  monitor + window size + margin.
- `running_since_for` - table-driven: `Busy`+`None`->`Some(now)`,
  `Busy`+`Some(t)`->`Some(t)`, each non-`Busy` phase -> `None`.
- `status_summary` - table-driven: row sets -> expected ordered non-empty groups
  with counts (incl. `Acknowledged` folding into the idle group).
- `Corner` parse - table-driven: valid strings + unknown -> `TopRight`.
- No test for the `render` impl or footer (thin view over tested helpers).

## Out of scope

Color/contrast restyle; `poll_secs`/`host` wiring; multi-monitor following;
perfecting spinner animation if gpui makes it awkward.

## Open implementation details (decide at build time)

- **Spinner**: prefer a real gpui animation for `◐`; acceptable fallback is a
  static glyph (the panel already re-renders on each reconcile tick).
- **Gutter past 9**: blank gutter for index 10+, confirmed above.
