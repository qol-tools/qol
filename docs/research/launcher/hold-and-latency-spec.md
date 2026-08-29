# Launcher input hold, input width, memory ask latency

Three user-visible defects, one spec, four lanes with disjoint file ownership.
Lanes edit only their owned paths, never run build, test, lint, format or git
commands, add no code comments, and never use the em-dash character.

## Findings (verified by the architect on 2026-08-29)

1. Memory asks time out (launcher shows "host did not answer within 12 s").
   The daemon runs the dev build (opt-level 0). Every ask goes through
   `retrieval::cache::build_or_load`, whose fingerprint hashes every doc ref, so
   any transcript append (the watcher ingests continuously while sessions run)
   makes the next ask rebuild BM25 indexes over 38 MB of units and rewrite the
   8 to 12 MB cache files. Measured: 17.3 s per ask in debug, 2.18 s cold and
   1.02 s warm in release. The daemon serves asks and ingests under the same
   `Mutex<WarmState>`.
2. The typed query in the launcher caps at 25 characters:
   `plugins/launcher/src/ui/view.rs:106` `SEARCH_VISIBLE_CHARS = 25`, a fixed
   character window that ignores the roughly 380 px of mono text room the
   500 px header actually has.
3. The launcher dismisses when any other application takes focus. On Linux the
   window is override-redirect and receives focus through `set_input_focus`,
   but nothing holds the keyboard, so a WM focus change (a new window mapping,
   a terminal tab opening) moves keyboard input away. gpui sees FocusOut, the
   blur and activation subscriptions in `libs/qol-gpui/src/ghost.rs`
   (`track_dismiss`) debounce 120 ms, `has_process_focus` reports false, and
   `hide_to_ghost("blur")` runs; `reset_for_show` then wipes the query and the
   trail. On macOS the launcher is a `WindowKind::Normal` NSWindow inside an
   Accessory-policy app, so app deactivation triggers the same dismiss path.

## Design

### Bug 1: deterministic ask latency

Two independent halves; both land.

- Build: `[profile.dev.package.qol-memory] opt-level = 2` in the root
  `Cargo.toml`, next to the existing per-package entries. The tokenizer and
  index code monomorphize into this crate, so the setting belongs on
  qol-memory itself.
- Daemon: indexes live in `WarmState` and grow incrementally. No ask in the
  daemon ever rebuilds an index from scratch unless the units file shrank or
  was replaced.
  - `retrieval::Index::extend(&mut self, items: &[DocRef<'_>])` appends docs,
    updates `df`, `n`, `total_length`, `avgdl`, and recomputes `idf` from `df`.
    `build_index` stays and must produce the same ranking as an empty index
    extended with the same items.
  - `CachedLayers` gains the indexes the ask uses (the answer pool, the all
    layer, the notes layer) built with exactly the filtering the ask applies
    today (`doc_refs`, `notes_refs`, `visible_notes`, `dedupe_user_units`,
    `is_boilerplate_unit`), plus the set of indexed keys.
  - `WarmState::refresh_layers`: when `units_len` grew and the notes run is
    unchanged, read `units.jsonl` from the previous length, parse only the
    appended lines, push them onto the units layer, and extend the answer and
    all indexes with the refs those units contribute, skipping keys already
    indexed. When only the notes run changed, rebuild only the notes index.
    When `units_len` shrank or the units mtime moved without growth, rebuild
    everything. `push_units` extends the same indexes.
  - The watcher and the startup ingest stop calling `invalidate_layers` after a
    successful append; the fingerprint path above absorbs the growth. The
    distill-run change still invalidates the notes index only.
  - `ask::run_and_log_with_layers` takes the warm indexes through a new
    `WarmIndexes<'a> { answer: &'a Index, all: &'a Index, notes: &'a Index }`
    parameter and no longer calls `cache::build_or_load` on the daemon path.
    The CLI path (no daemon) keeps `build_or_load` unchanged.
- Acceptance: `cargo test -p qol-memory` green; with the daemon reloaded on the
  dev build, two launcher asks with a transcript append between them each
  answer in under one second.

### Bug 2: width-derived query window

- `view::search_bar` gains a trailing `window: &mut gpui::Window` parameter.
  Lane C updates the single call site in `plugins/launcher/src/ui/render.rs`
  to `view::search_bar(<existing args>, window)`.
- The visible character count is derived, not fixed: measure the mono glyph
  advance once per render with the gpui text system
  (`window.text_system().shape_line(...)` on a single "0" run at `TEXT_BODY`
  in `font_mono()`; verify the exact gpui 0.2.2 signature in
  `~/.cargo/registry/src/*/gpui-0.2.2/src/text_system.rs` before use) and the
  trailing counter width the same way at `TEXT_NANO` (the spinner is
  `TEXT_BODY` wide when pending). Available width is
  `WINDOW_WIDTH - 2 * SPACE_PAD - chevron width - 2 * 10.0 gap - trailing`.
- Pure helpers with unit tests in `view.rs`:
  `visible_char_count(available_px: f32, advance_px: f32) -> usize` (floor,
  never below 8) and `search_window(char_count, cursor, visible) -> (usize,
  usize)` (the current `view_start` and `view_end` arithmetic, extracted).
  `SEARCH_VISIBLE_CHARS` is deleted.
- Acceptance: a 60 character query at 500 px shows every character that fits
  the row and scrolls only past that; `cargo test -p launcher` green.

### Bug 3: hold input while showing

Principle: while the launcher is showing, input is held by the launcher and
blur or deactivation never dismisses it. Dismissal happens only on Esc, on a
launch, on the hotkey toggle, or on a pointer click outside the launcher.
Everything is event-driven; the one bounded retry is documented below.

Shared contract in `libs/qol-gpui/src/popup_window/platform/mod.rs`, exported
for every cfg:

```rust
pub fn register_native_display(window: &gpui::Window);
pub fn hold_input(title: &str) -> bool;
pub fn release_input(title: &str);
pub fn input_held() -> bool;
```

`libs/qol-gpui/src/ghost.rs`: a new `track_dismiss_held` that takes one more
closure, `input_held: impl Fn(&V) -> bool + 'static`. The blur and activation
handlers return early with a `skip_held` trace while it is true, and the
debounce verdict returns `Recover` while it is true (`debounce_verdict` gains a
`held: bool` parameter with a unit test). `track_dismiss` and
`track_dismiss_confirmed` keep their signatures and pass `|_| false`, so
qol-shot's preview behavior is untouched.

Launcher wiring (`plugins/launcher/src/ui/render.rs`): call
`register_native_display(window)` once on the first render, switch the dismiss
tracker to `track_dismiss_held` with `|_| qol_gpui::popup_window::input_held()`,
and pass `window` to `view::search_bar`. `hide_to_ghost` in
`plugins/launcher/src/ui/mod.rs` calls `release_input(&self.window_title)`
before `dismiss_to_ghost`.

#### Linux (X11, Cinnamon Muffin)

- The keyboard grab must be issued on gpui's own X connection, because an
  active grab delivers key events to the grabbing client only. gpui's
  `Window` implements `HasDisplayHandle`; on X11 it yields an
  `XcbDisplayHandle` whose `connection` is the live `xcb_connection_t`.
  `register_native_display` stores that pointer (and the screen) in a static;
  `hold_input` wraps it with
  `x11rb::xcb_ffi::XCBConnection::from_raw_xcb_connection(ptr, false)`
  (feature `allow-unsafe-code`, already enabled through the workspace-hack),
  resolves the launcher wid by title, and issues
  `grab_keyboard(false, wid, CURRENT_TIME, GrabMode::ASYNC, GrabMode::ASYNC)`,
  flushes, and checks the reply status. `SUCCESS` sets the held flag and
  returns true; `ALREADY_GRABBED` returns false; other statuses return false.
  `release_input` issues `ungrab_keyboard(CURRENT_TIME)`, flushes, clears the
  flag. Never keep the wrapped connection past the call.
- Bounded retry: the tray's hotkey is a passive `XGrabKey`, so at show time the
  server may still hold that grab until the key is released. In
  `plugins/launcher/src/ui/platform/linux.rs::show_topmost_window`, after the
  existing focus reassert, call `hold_input`; if it returns false, run one
  ladder through `qol_gpui::platform::spawn_reassert_driver` with delays
  `[30, 30, 30, 30, 60, 120, 240]` whose poll returns `Stop` once
  `input_held()` and whose reassert calls `hold_input` again. The ladder ends
  after 540 ms whatever happens; if the grab never lands, today's blur dismiss
  stays in force because `input_held()` is false. Use a dedicated
  `static HOLD_GEN: AtomicU64` so a later show cancels an older ladder.
- Key delivery while the WM moves focus elsewhere: X sends FocusOut with mode
  NotifyWhileGrabbed, gpui marks the window inactive, and key events keep
  arriving on the grab window. Verify in gpui 0.2.2 that keystroke dispatch
  (`src/window.rs`, `dispatch_event` and the keystroke path) does not gate on
  the window's active flag, and report the file and line. If it does gate,
  additionally select `FocusChange` on the launcher wid from the click-away
  monitor's connection and answer each FocusOut while held with
  `set_input_focus(PARENT, wid, CURRENT_TIME)`.
- Click-away on Linux (new backend in
  `plugins/launcher/src/ui/click_away/platform/linux.rs`, same
  `start(window_title, tx) -> Option<Monitor>` contract as the macOS file): a
  dedicated `x11rb::rust_connection::RustConnection`, `xinput` extension
  version 2.0 negotiated, `xi_select_events` on the root window for
  `XIEventMask::RAW_BUTTON_PRESS` with device `XIAllMasterDevices`. The thread
  blocks in `poll(2)` on the connection fd and a stop pipe; `Monitor` owns the
  pipe's write end and its `Drop` wakes the thread, which then exits. On each
  raw button press it calls
  `qol_gpui::popup_window::pointer_over_window_by_title(&title)` once and
  sends on `tx` when that is `Some(false)`. No timers, no polling. Add
  `x11rb = { workspace = true, features = ["xinput"] }` and `libc` under the
  launcher's `[target.'cfg(target_os = "linux")'.dependencies]`.
- Known hazard, accepted for this round: while the grab is held an X screen
  locker cannot grab the keyboard. The grab lasts only while the launcher is
  showing and is released on every dismiss and on process exit.

#### macOS

- `hold_input` in `libs/qol-gpui/src/popup_window/platform/macos.rs`: locate
  the NSWindow by title with the helpers already in that file, set
  `level = NSFloatingWindowLevel`, `setHidesOnDeactivate(false)`, add
  `NSWindowCollectionBehaviorCanJoinAllSpaces | FullScreenAuxiliary`, and
  register `NSNotificationCenter` observers for
  `NSWindowDidResignKeyNotification` (object: that window) and
  `NSApplicationDidResignActiveNotification`. The observer block, while the
  held flag is set and fewer than three reasserts have happened for this hold,
  calls `NSApplication::activateIgnoringOtherApps(true)` (or the non-deprecated
  `activate` when the objc2-app-kit binding exposes it) and
  `makeKeyAndOrderFront`. Returns true when the window was found.
  `release_input` removes both observers, clears the flag and the counter.
  `register_native_display` is a no-op on macOS.
- `plugins/launcher/src/ui/platform/macos.rs` (new): `show_topmost_window`
  calls `qol_gpui::ghost::show_ghost_window_topmost` then `hold_input`.
  Lane C adds the `macos` cfg arms in `plugins/launcher/src/ui/platform/mod.rs`.
- The existing macOS click-away monitor already dismisses on clicks outside.
- Gate for this platform: `cargo clippy --target aarch64-apple-darwin
  -p qol-gpui -p launcher --all-targets` run by the architect.

## Lane ownership (no file appears twice)

- Lane A `lh-memory-latency`: `Cargo.toml` (root, profile section only),
  `plugins/qol-memory/src/retrieval/mod.rs`,
  `plugins/qol-memory/src/app/warm.rs`,
  `plugins/qol-memory/src/app/request.rs`,
  `plugins/qol-memory/src/app/mod.rs`,
  `plugins/qol-memory/src/watch/mod.rs`,
  `plugins/qol-memory/src/ask/mod.rs`.
- Lane B `lh-input-width`: `plugins/launcher/src/ui/view.rs`,
  `plugins/launcher/src/ui/layout.rs`.
- Lane C `lh-hold-linux`: `libs/qol-gpui/src/ghost.rs`,
  `libs/qol-gpui/src/popup_window/mod.rs`,
  `libs/qol-gpui/src/popup_window/platform/mod.rs`,
  `libs/qol-gpui/src/popup_window/platform/linux.rs`,
  `libs/qol-gpui/src/popup_window/platform/fallback.rs`,
  `libs/qol-gpui/Cargo.toml`,
  `plugins/launcher/Cargo.toml`,
  `plugins/launcher/src/ui/mod.rs`,
  `plugins/launcher/src/ui/render.rs`,
  `plugins/launcher/src/ui/platform/mod.rs`,
  `plugins/launcher/src/ui/platform/linux.rs`,
  `plugins/launcher/src/ui/platform/fallback.rs`,
  `plugins/launcher/src/ui/click_away/platform/linux.rs`.
- Lane D `lh-hold-macos`:
  `libs/qol-gpui/src/popup_window/platform/macos.rs`,
  `plugins/launcher/src/ui/platform/macos.rs` (new).

## Gate (architect, once per round)

`cargo test -p qol-memory -p launcher -p qol-gpui`, `qol check`
(`env -u QOL_TRAY_HTTP_TOKEN cargo run -q -p qol -- check`), the macOS
cross-target clippy above, then a dev reload of qol-memory and launcher and the
latency measurement from Bug 1.

## Acceptance list

1. Two consecutive launcher asks with an append in between answer in under one
   second each on the dev build.
2. The query row shows as many characters as fit the header width.
3. With the launcher showing on Linux, mapping a new window (for example a
   terminal) leaves the launcher visible and typing still lands in it; a click
   outside dismisses it; Esc dismisses it; the hotkey still toggles.
4. macOS compiles clean under cross-target clippy with the same hold contract.
5. qol-shot's preview dismiss behavior is unchanged (`track_dismiss` signature
   and semantics untouched).
