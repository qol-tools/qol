# Monitor management: list, arrange, modes

Implementation spec, 2026-09-10. Extends `docs/specs/2026-08-16-monitor-control.md`
with the deferred management phase. Interfaces in sections 3 through 8 are frozen;
report a deviation instead of inventing one.

## 1. Goal and acceptance criteria

plugin-monitor gains, on Linux/X11 and macOS: connected displays with position,
size, primary flag, current resolution and refresh rate; position and primary
changes (arrange); per-display resolution and refresh changes (modes).

Acceptance criteria:

1. `plugin-monitor layout` and `plugin-monitor modes` report real state;
   `set-mode`, `arrange`, `primary`, and the `apply_layout` daemon action apply
   it atomically.
2. Every read is served by one shared display capability owner; plugin-monitor
   never opens xrandr or CoreGraphics itself.
3. Every write is captured before the first change and restored on daemon exit
   and on unclean recovery when the host is portable; a resident host keeps the
   change, exactly like the existing brightness and gamma mutations.
4. The settings page exposes the capability through the config contract: a
   Layout list, a Modes list, per-display position fields, and an Apply action.
5. Gate: `qol check` green in the worktree; guest verified on
   linux/mint-cinnamon for layout/modes reads, mode set and restore, primary,
   and write refusals. macOS compiles in CI; it cannot be verified locally.

## 2. Layering (single source of truth)

- `libs/windowing` (qol-windowing) is the display capability owner. It already
  owns `DisplayHandle` identity, `MonitorBounds` geometry, and the per-OS
  `DisplayEnumerator`. This spec adds the read snapshot (position, primary,
  current mode) and the write ops (modes, set mode, set layout) inside the same
  module and the same per-OS platform split, matching the existing in-lib
  enumeration precedent.
- plugin-monitor is the policy layer: daemon commands and queries, CLI, config,
  session capture and restore, residency, notifications, doctor.
- Existing private readers (host `desktop_state`, `window-actions` macOS screen
  cache, `shot`, `pointz`, `qol-cli trace_rs`) are not migrated here; section 11
  lists them as follow-ups so the new read model is their migration target.

## 3. Frozen shared API (lane A)

New types and traits in `libs/windowing/src/display/mod.rs`, re-exported from
`libs/windowing/src/lib.rs` exactly like `DisplayHandle` is today:

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DisplayMode {
    pub token: u64,
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DisplaySnapshot {
    pub handle: DisplayHandle,
    pub bounds: MonitorBounds,
    pub primary: bool,
    pub mode: Option<DisplayMode>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DisplayPlacement {
    pub handle: DisplayHandle,
    pub x: i32,
    pub y: i32,
    pub primary: bool,
}

pub trait DisplayEnumerator {
    fn enumerate(&self) -> Result<Vec<DisplayHandle>, DisplayError>;
    fn snapshot(&self) -> Result<Vec<DisplaySnapshot>, DisplayError>;
}

pub trait DisplayOps {
    fn modes(&self, handle: &DisplayHandle) -> Result<Vec<DisplayMode>, DisplayError>;
    fn set_mode(&self, handle: &DisplayHandle, mode: &DisplayMode) -> Result<(), DisplayError>;
    fn set_layout(&self, placements: &[DisplayPlacement]) -> Result<(), DisplayError>;
}

pub fn validate_layout(placements: &[DisplayPlacement]) -> Result<(), DisplayError>;
```

`DisplayError` gains two variants, existing ones unchanged:

```rust
Unsupported { capability: &'static str, reason: String },
LayoutInvalid { reason: String },
```

with `Display` and `std::error::Error` impls for both. `token` is the
platform-native mode id (X11 mode id, CoreGraphics IODisplayModeID as u64).

Binding semantics:

- `snapshot`: one entry per display that currently has geometry, in virtual
  coordinates; `primary` per the platform marker; `mode` is `None` when the
  current mode cannot be read.
- `modes`: every selectable mode including the current one; an unreadable mode
  list is an error, never an empty list.
- `set_mode`: matches `token` only; after the write, read back and compare; a
  mismatch is `LayoutInvalid` with a reason. `Unsupported` on macOS.
- `set_layout`: structural validation via `validate_layout` (non-empty, unique
  handles, exactly one primary). Per OS:
  - Linux: reject origins outside the X11 `i16` range; reject overlapping
    rectangles using each display's current mode size; reject two placements
    that resolve to the same CRTC (mirrored displays); apply one CRTC write per
    display; if any write fails, roll back every already-written display to the
    captured pre-state before returning the error; apply the primary marker
    after the geometry writes; retry the failed write once on a fresh
    `config_timestamp` when the server answers `INVALID_CONFIG_TIME`.
  - macOS: one begin/configure/complete transaction; when the placement marked
    primary is not at origin `(0, 0)`, translate every placement by the
    primary's negative origin before applying (`(0, 0)` is the macOS main
    display); a failed `complete` cancels the configuration.
- `Platform` implements both traits per OS; construction stays
  `qol_windowing::Platform`. Windows and fallback return
  `DisplayError::Unsupported` for every new method.

Required tests (no hardware, seam-based, following the existing gamma transport
pattern in `plugins/monitor/src/monitor/backends/x11_randr_gamma.rs` and
`cg_gamma.rs`):

- Linux fake bus: snapshot position/mode/primary; refresh math from `dot_clock`,
  `htotal`, `vtotal`, including a 59.94 style and an interlace vector; modes
  list with the current one marked; exact-token `set_mode` and mismatch refusal;
  rollback when the second CRTC write fails; `INVALID_CONFIG_TIME` retry-once;
  overlap, `i16` range, and mirrored refusals; `validate_layout` cases.
- macOS fake seam: snapshot mapping, modes, layout translation to origin
  `(0, 0)`, begin/configure/complete ordering, cancel on failure, `set_mode`
  unsupported.
- Windows and fallback: unsupported variants.

## 4. plugin-monitor facade (lane B)

`plugins/monitor/src/monitor/mod.rs`:

- delete the local `DisplayMode`; re-export the shared types:
  `pub use qol_windowing::display::{DisplayMode, DisplayPlacement, DisplaySnapshot};`
- `DisplayControl` keeps its brightness and gamma methods and gains:
  `snapshot`, `list_modes`, `set_mode`, `set_layout` (the existing
  `list_modes`/`set_mode` stubs are retyped to the shared `DisplayMode`).
- `StubControl`: `snapshot` returns `Ok(Vec::new())`; the other three return
  `MonitorError::unsupported("modes" | "layout", <existing reason>)`.
- new `plugins/monitor/src/monitor/backends/shared_display.rs`: `SharedDisplay`
  wraps `qol_windowing::Platform` and maps `DisplayError` to `MonitorError`
  (`Unsupported` and `UnsupportedPlatform` to `unsupported`, `LayoutInvalid` to
  `refused("layout", reason)`, `Io` to `Display`).
- `plugins/monitor/src/platform/{linux,macos}.rs`: compose
  `PlatformControl { brightness: PolicyControl<..>, display: SharedDisplay }`
  and implement `DisplayControl` by delegation; export it from `control()`.
  Windows and fallback keep `StubControl`.

New `plugins/monitor/src/monitor/layout.rs`, pure and serde-friendly:

```rust
#[derive(serde::Serialize)] pub struct LayoutRow {
    pub id: String, pub connector: String, pub x: i32, pub y: i32,
    pub width: u32, pub height: u32, pub refresh_hz: u32,
    pub primary: bool, pub detail: String, pub settable: bool,
}
#[derive(serde::Serialize)] pub struct ModeRow {
    pub id: String, pub connector: String, pub width: u32, pub height: u32,
    pub refresh_hz: u32, pub label: String, pub detail: String,
    pub current: bool, pub selectable: bool,
}
#[derive(serde::Deserialize, Clone)] pub struct ArrangeRequest { pub id: String, pub x: i32, pub y: i32 }
#[derive(serde::Serialize, serde::Deserialize, Clone)] pub struct LayoutPosition { pub x: i32, pub y: i32, pub primary: bool }

pub fn placements_from_snapshots(snapshots: &[DisplaySnapshot]) -> Vec<DisplayPlacement>;
pub fn layout_rows(snapshots: &[DisplaySnapshot]) -> Vec<LayoutRow>;
pub fn mode_rows(snapshots: &[DisplaySnapshot], modes: &BTreeMap<String, Vec<DisplayMode>>) -> Vec<ModeRow>;
pub fn resolve_arrange(snapshots: &[DisplaySnapshot], requested: &[ArrangeRequest], primary: Option<&str>) -> Result<Vec<DisplayPlacement>, MonitorError>;
pub fn resolve_config_layout(snapshots: &[DisplaySnapshot], config: &BTreeMap<String, LayoutPosition>) -> Result<Vec<DisplayPlacement>, MonitorError>;
```

Rules: every requested or configured display id matches a snapshot by `id` or
`connector`; the result carries exactly one primary (the request's, else the
current primary); unknown ids, zero or several primaries, and an empty result
are `MonitorError::refused`. Geometry judgement (overlap, range) stays in the
backends. `detail` for a layout row is a compact position/size/mode string; the
`displays` query detail text gains `+x+y`, `WxH@Hz`, and ` primary` when known.

Required tests: row builders, `resolve_arrange` primary defaulting and unknown
id, `resolve_config_layout` primary refusals, `SharedDisplay` error mapping,
`PlatformControl` delegation, `StubControl` unsupported.

## 5. Daemon surface (lane C)

`parse_request` names, also declared in `qol-runtime.toml` by lane D:

- `"layout"`, `"modes"` → `live_query`
- `"set_mode"` input `{ id: String, width: u64, height: u64, refresh: u64? }`
- `"set_primary"` input `{ id: String }`
- `"arrange"` input `{ placements: [{ id: String, x: i64, y: i64 }], primary: String? }`
- `"apply_layout"` reads config `layout_position`

Commands: `Command::SetMode { display, width, height, refresh: Option<u32> }`,
`Command::SetPrimary { display }`,
`Command::Arrange { placements: Vec<ArrangeRequest>, primary: Option<String> }`,
`Command::ApplyLayout`. Each command resolves the display set from
`control.snapshot()`, refines errors (`unknown display`, `resolution is
ambiguous without a refresh rate, candidates: ...`, `no primary`), claims the
layout snapshot when none exists, applies, then touches the snapshot and
re-asserts gamma (section 6). A refusal claims nothing and mutates nothing.
Success notifications follow the existing `notify_on_change` rule.

Queries:

- `layout` → a bare JSON array of `LayoutRow` objects, matching the existing
  `displays` payload shape; a failed snapshot returns an empty array.
- `modes` → a bare JSON array of `ModeRow` objects; a display whose mode list
  errors contributes no rows.
- `displays` rows extend their detail as already specified in section 4.

Required tests: input parsing per action (missing, wrong type, out-of-range,
unknown id), ambiguity refusal, claim-before-first-write, no claim on refusal,
gamma re-assert invoked after both writes (spy), payload shapes.

## 6. Session, capture, restore, gamma (lane C)

New snapshot in `plugins/monitor/src/session.rs`:

```rust
pub struct LayoutSnapshot {
    pub schema_version: u32,
    pub layout_id: String,
    pub placements: Vec<PlacementRecord>,   // id, connector, x, y, primary
    pub modes: Vec<ModeRecord>,             // id, connector, token, width, height, refresh_hz
    pub mutations: u32,
}
```

`SessionSnapshot` with `SCHEMA_VERSION = 1`; stored through
`inner.owner_store("layout")` (its own directory, never the per-display session
dir, so the existing `load_all` never sees it). Store calls: `claim_layout`,
`touch_layout`, `load_layout`, `delete_layout`, `restore_layout(RestoreMode)`.

Restore order: modes first (exact token, fall back to width, height and refresh;
a gone display is skipped and keeps the snapshot), then the whole placement set
in one `set_layout`; a failed layout keeps the snapshot and reports `Failed`,
a fully restored snapshot is deleted.

Call sites: portable recovery in `Runtime::start` restores layout before the
per-display brightness and gamma restore; `Command::Kill` does the same on
portable hosts; resident hosts skip both, matching the existing
`is_resident()` branches.

Handoff: a reload or update handoff is not the end of the session, so the
layout snapshot carries the same handoff handling as the per-display snapshot
(`handoff` plus `adopt_generation`, `Session::mark_handoff_all`, and the daemon
`Command::Handoff` / `HandoffSuccessor` call sites). A handed-off snapshot is
kept by the successor instead of restored when its start runs; an unclean
recovery still restores it, and the final exit still restores it. New fields
are serde-defaulted so existing snapshots load.

Gamma re-assert: `SetCrtcConfig` and the macOS mode change reset the CRTC gamma
ramp. After any successful `set_mode` or `set_layout`, rewrite the current gamma
LUT for every changed display through the existing guarded write path, and
surface `ForeignLutPreserved` or a write failure with the existing gamma warning.
Never trust the cached gamma state after a mode write.

Required tests: snapshot envelope and checksum round-trip, modes-before-placements
restore order, skipped-gone-display retention, failed-layout retention, resident
skip, gamma re-assert invoked on mode set.

## 7. CLI (lane D)

New verbs in `plugins/monitor/src/cli.rs`, following the existing help, selector,
exit-code, and `--json` conventions:

- `modes [display]` (`--json`): one row per mode, current marked.
- `set-mode <WIDTHxHEIGHT[@HZ]> [display]`: exact match; an ambiguous
  resolution without a refresh rate refuses and lists candidates.
- `layout` (`--json`): display, connector, `+x+y`, `WxH@Hz`, primary.
- `arrange <display>=<x>,<y> [more...] [--primary <display>]`: atomic.
- `primary <display>`.

Doctor: `mode_control` (X11 ok when RandR supports primary and a connected
output reports modes; Wayland warn with the X11-only reason; macOS warn that mode
writing is gated while arrangement is available; unsupported fail) and
`layout_restore` (ok with no snapshot, warn naming the displays still pending).

Required tests: parsing and exit codes, `--json` only where declared, refusal
messages, help equivalence.

## 8. Contracts (lane D)

`qol-runtime.toml`: `[query.layout]`, `[query.modes]`,
`[action.set_mode] args = ["set_mode"]`,
`[action.set_primary] args = ["set_primary"]`,
`[action.arrange] args = ["arrange"]`,
`[action.apply_layout] args = ["apply_layout"]`.

`qol-config.toml`, new section `[section.arrangement]`:

- `[field.layout]` list, `query = "layout"`, `row_label = "{connector}"`,
  `row_subtitle = "{detail}"`, row action `set_primary`, label "Set primary",
  `when = "settable"`, `input = { id = "{id}" }`.
- `[field.modes]` list, `query = "modes"`, `row_label = "{label}"`,
  `row_subtitle = "{detail}"`, row action `set_mode`, label "Set",
  `when = "selectable"`,
  `input = { id = "{id}", width = "{width}", height = "{height}", refresh = "{refresh_hz}" }`.
- `[field.layout_position]` object_map, `config_key = "layout_position"`,
  `key_label = "Display ID"`, `entry_fields = { x = "number", y = "number", primary = "boolean" }`.
- `[field.apply_layout]` action, `action = "apply_layout"`, `variant = "primary"`,
  label "Apply positions".

`plugin.toml` needs no new tray actions; do not add any.

## 9. Platform matrix

| Capability | Linux/X11 | Linux/Wayland | macOS | Windows |
| --- | --- | --- | --- | --- |
| snapshot, layout read | yes | existing enumeration only, no geometry | yes | unsupported |
| modes read | yes | unsupported | yes | unsupported |
| set_mode | yes | refused | unsupported (gated private-API review) | unsupported |
| set_layout | yes | refused | yes, public CoreGraphics | unsupported |

On Linux every write first checks `platform::display_server() == DisplayServer::X11`
and otherwise returns `MonitorError::unsupported("modes" | "layout", "display
configuration needs an X11 session")`. Never infer the session from `DISPLAY`:
XWayland answers RandR requests while nothing real moves.

## 10. Lane ownership

| Lane | Owned paths | Deliverable |
| --- | --- | --- |
| A `mon-lib-display` | `libs/windowing/src/display/**`, `libs/windowing/src/lib.rs`, `libs/windowing/Cargo.toml` | shared types, per-OS impls, validation, seam tests |
| B `mon-plugin-core` | `plugins/monitor/src/monitor/**`, `plugins/monitor/src/platform/**` | facade, shared backend, layout helpers, wiring, tests |
| C `mon-runtime` | `plugins/monitor/src/session.rs`, `plugins/monitor/src/daemon.rs`, `plugins/monitor/src/config.rs` | commands, queries, capture, restore, gamma re-assert, tests |
| D `mon-surface` | `plugins/monitor/src/cli.rs`, `plugins/monitor/plugin.toml`, `plugins/monitor/qol-runtime.toml`, `plugins/monitor/qol-config.toml` | CLI verbs, doctor, contracts |

Edit only your owned paths. Never run build, test, lint, format, or git commands.
Add no code comments and no em-dash character anywhere. The interfaces above are
frozen: a required deviation is reported in the lane report, never invented.
A lane that believes it needs another file stops and reports instead.

## 11. Non-goals and follow-ups

- The host runtime wire type (`PlatformState.monitors`) still carries geometry
  without identity; making it transport `DisplaySnapshot` is a follow-up that
  needs an additive wire change and a consumer audit.
- Migrating `apps/qol-tray/src/desktop_state`, `plugins/window-actions` macOS
  `screen.rs`, `plugins/shot` display lists, `plugins/pointz` ScreenBoundsCache,
  and `tools/qol-cli trace_rs` onto the shared snapshot: follow-ups.
- Not in this feature: rotation, scale and mirroring editing, a drag arrange
  gpui surface, per-display preferred mode persistence, Windows
  `SetDisplayConfig`, and Wayland output-management protocols.

## 12. Verification (architect, after the lanes land)

1. `qol check` in the worktree.
2. Guest `qol env up linux/mint-cinnamon --dev-worktree <worktree>`:
   `plugin-monitor layout|modes`, a real `set-mode` verified with `xrandr`
   before and after, `primary` on the single display, an `arrange` refusal
   (unknown id and overlap), daemon stop and layout restore, and
   `plugin-monitor doctor`.
3. Multi-output arrangement, rollback, and refresh math are covered by the
   lane-A seam tests; the guest has one output.

## 13. Review corrections (frozen deltas)

These supersede the corresponding text above. Every fix ships with the
regression test named in the review finding.

### 13.1 Shared API (lane A)

- `DisplayError` gains `NotFound { capability: &'static str, selector: String }`.
- `set_mode` on a read-back mismatch writes the captured pre-state back before
  returning the typed error that names the observed token.
- `set_layout`: a primary-marker failure after the CRTC writes rolls every
  written CRTC back, and a rollback failure is reported in the returned error
  rather than discarded; after all writes the placements are read back and a
  mismatch is `LayoutInvalid`; on `INVALID_CONFIG_TIME` the target is re-read and
  re-resolved, the retry happens only when it still matches the request, and
  otherwise the call refuses.
- Linux `set_mode` accepts only tokens the target output advertises.
- macOS bounds the origin translation (placements whose translated origins leave
  `i32` are refused), and a failed `complete` cancels the configuration with the
  same ref.
- macOS `set_mode` is the trait method itself; `set_mode_with` is deleted.
- Export `cg_display_id_from_connector(connector: &str) -> Option<u32>` from
  `qol_windowing::display` and delete the plugin's copy and its duplicate test.

### 13.2 Plugin facade (lane B2)

- `ModeRow` gains `token: u64` and `writable: bool`; `selectable = writable &&
  !current`; the label stays `WxH@Hz` and the contract puts the connector in
  the row label. `ModeRow.id` becomes unique (`{display_id}#{token}`) and a new
  `display_id` field carries the display for the action input, so the settings
  list can dispatch each row independently.
- `writable` comes from the backend capability probe (`probe().modes`): Linux
  X11 true, macOS false, Windows and fallback false, so a mode row is never
  offered where the write is unsupported.
- New pure helpers in `monitor/layout.rs`, used by the CLI, the daemon, and the
  session: `snapshot_for(snapshots, selector) -> Result<&DisplaySnapshot,
  MonitorError>` (exact id, then exact connector, an ambiguous id refuses),
  `resolve_mode(modes, width, height, refresh) -> Result<DisplayMode,
  MonitorError>`, `mode_lists(snapshots, list) -> BTreeMap<String,
  Vec<DisplayMode>>`, and `geometry_detail` reused by the displays payload.
- `X11Display` gates `snapshot` and `modes` exactly like the writes: on a
  non-X11 session both return `DisplayError::Unsupported` with the X11 reason.
- `DisplayError::NotFound` maps to `MonitorError::DisplayNotFound`; mode
  failures map with capability `modes`; an unreadable mode list is an error the
  CLI reports, never a silent empty answer.
- `LayoutPosition` derives `PartialEq, Eq` and every field is serde-defaulted
  (`x`/`y` 0, `primary` false); `DeviceConfig` regains `PartialEq, Eq` and its
  tests compare directly again.
- `DisplayError::Io` renders the inner error alone; IO errors are never
  mislabelled as enumeration failures, and `MonitorError::Display` must not
  add or repeat an enumeration prefix.

### 13.3 Daemon and session (lane B1)

- The layout mutation is recorded before the hardware write: the baseline is
  captured and written with `mutations = 1` (or touched) before `set_layout` or
  `set_mode`, so a crash mid-write still restores. A capture or touch failure
  aborts before any display write and notifies with the operation name
  ("Mode not set", "Layout not applied", "Primary not set").
- `restore_layout`: the mode loop is per-display and non-fatal, so a failed
  display's mode is skipped and the placements still restore; placements restore
  the present subset; the gone check runs again immediately before the write;
  records match the stable id first and fall back to the connector only when the
  handle is `identity_unstable`; after the write the placements are read back
  and the snapshot is deleted only when the layout matches; a failed restore is
  notified, not stderr-only.
- `reassert_gamma`: with `lut.is_some() && mutations > 0`, write the current
  adjustment unconditionally, identity included, so a ramp reset is repaired;
  `restore_layout` re-asserts gamma for every display it moved before returning.
- Display state is restored after every successful display write, not only on
  restore: a mode, primary, arrange, or apply-layout write re-asserts gamma for
  every connected display (a mode write resets the whole screen's ramps, not
  only the target output) and immediately re-evaluates night mode with force,
  so the gamma tint or the host night light comes back at once instead of at
  the next tick. One `MONITOR_SESSION event=display_reassert op=... displays=...
  restored=... failed=... night_active=...` trace names the operation. A refused
  write restores nothing and writes nothing extra.
- `apply_layout` with an empty configured map claims nothing and writes nothing.
- `claim_layout` may replace a snapshot whose recorded displays are all absent
  from the current topology.
- `live_query` never holds the runtime mutex across display I/O: snapshot the
  control state, drop the guard, then build the payload, so `Command::Kill`
  cannot starve behind a poll.
- Ownership before recovery: the daemon owns its socket (or an exclusive
  session lock) before `Runtime::start` runs the portable Recovery pass, so a
  successor cannot restore and delete the baseline before the predecessor is
  evicted.

### 13.4 CLI and contracts (lane C)

- The modes row action input carries `id`, `token`, `width`, `height`, `refresh`;
  the daemon prefers `token` and keeps the ambiguity refusal when only
  width/height are given.
- `plugin.toml` keeps the four actions (the tray action endpoint refuses actions
  absent from the catalog); `args` name real CLI verbs (`set-mode`, `primary`,
  `arrange`, `apply-layout`); labels are aligned across `plugin.toml`,
  `qol-config.toml`, and `qol-runtime.toml`; `qol-runtime.toml` drops the inert
  `args` keys.
- New `apply-layout` CLI verb (reads the configured positions, applies through
  the local control, prints the applied count) so every runtime action has a
  headless command.
- CLI selector resolution uses the shared `snapshot_for` (exact id or connector,
  ambiguous id refused, no prefix matching) and the shared `resolve_mode`.
- The `layout_restore` doctor check warns only when `mutations > 0`.
- CLI write verbs stay inline and are documented as uncaptured diagnostics;
  daemon-dispatched writes are the captured path.

### 13.5 Tests

Each lane adds the regression tests named above in its own files: the seam
boundary tables (`i16` edges, retry rollback, stale-target refusal, primary
rollback, read-back mismatch) in lane A; the daemon and session tables (failure
after claim, claim failure, restore ordering, ordered gamma re-assert, stale
handoff, store isolation, notify on and off, kill during a query) in lane B1;
resolver, capability, config-default, and delegation-argument tables in lane B2;
contract round-trip, mode-row identity, ambiguous selector, and doctor-state
tables in lane C.

### 13.6 Deferred, accepted risks

The review named and the architect accepts these residual items: the read/write
trait split inside `DisplaySnapshot`/`DisplayOps` (interface redesign), batching
and connection reuse across display reads (performance), carrying the native
macOS display id on the identity type (interface addition), sharing the gamma
X11 transport with the shared owner (section 11 follow-up), and the tray's 2s
stop grace acknowledging a completed restore (host-side follow-up). CLI-dispatched
writes are uncaptured by design, matching the existing brightness CLI precedent.
