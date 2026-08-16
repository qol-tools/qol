# Spec: pace wake deferral by draft activity, not a blind cap

Problem: `deliver_wake` in `tools/qol-cli/src/commands/sessions/watch.rs` defers a wake while `composer_busy` is true and only re-checks every 3s up to 100 attempts.
Two field failures on 2026-08-16:

1. Claude Code echoes every sent user message as a `> text` transcript line, and briefly hides the composer row early in a turn, so the last prompt-like line is a non-empty echo and `composer_busy` false-positives with nobody typing.
2. A human composing a message blocks the wake for the whole composition plus up to the full cap; the wake landed 3.5 minutes after the lane finished.

Fix: deliver as soon as the draft region stops changing.
A static region means nobody is typing; delivery appends and submits, so parked words end up in the transcript, never destroyed.

All work happens in this worktree on branch `wake-pacing`.
Scope is exactly one file: `tools/qol-cli/src/commands/sessions/watch.rs`.
Do not touch any other file.
Code comments are banned in this repo; use self-explanatory names.

## Constants

Replace the two current constants with:

```rust
const WAKE_COMPOSER_POLL_INTERVAL: Duration = Duration::from_secs(1);
const WAKE_COMPOSER_STATIC_POLLS: usize = 30;
const WAKE_COMPOSER_MAX_ATTEMPTS: usize = 300;
```

## Draft-region extraction

Add beside `composer_busy`:

```rust
fn composer_draft_region(screen: &str) -> Option<String>
```

It scans exactly like `composer_busy` (same three prefix forms) to find the LAST prompt-like line, and returns everything from that line to the end of the screen, with each line right-trimmed, joined by `\n`.
Returns `None` when no prompt-like line exists.
`composer_busy` keeps its current logic unchanged.

## Deferral loop rewrite

Inside `deliver_wake`, replace the current `while composer_busy(..) && deferrals < WAKE_COMPOSER_MAX_ATTEMPTS` loop with:

1. Read the screen once (as today). If the read fails, skip deferral and deliver.
2. Track `deferrals: usize`, `static_polls: usize`, and `previous_region: Option<String>` initialized from the first screen's `composer_draft_region`.
3. Loop while `composer_busy(&screen)` AND `deferrals < WAKE_COMPOSER_MAX_ATTEMPTS` AND `static_polls < WAKE_COMPOSER_STATIC_POLLS`:
   sleep one `WAKE_COMPOSER_POLL_INTERVAL`, increment `deferrals`, re-read the screen (on read error break to delivery), recompute the region; if the region equals `previous_region` increment `static_polls`, otherwise reset `static_polls` to 0 and store the new region.
4. After the loop, delivery proceeds exactly as today (`send_text` Submit).

Exit conditions in plain terms: composer cleared (deliver at once), region static for 30 consecutive 1s polls (nobody typing; deliver), or 300 total polls (pathological screen churn; deliver).

## Release-visible deferral trace

The existing `qol_runtime::probe!` stays, but release builds strip it, so also append one line to the driver's wake-debug log when `deferrals > 0`, right where the probe fires:

- Path: `trace_dir.join(format!("wake-debug-{}.log", sanitized))` where `sanitized` is the driver token with every `:` and `.` replaced by `_` - match the existing naming used by the files already in that directory (`wake-debug-v1_kitty_..._8_70942.log`).
- Line format: `{rfc3339-now} wake deferred driver={driver} polls={deferrals} static={static_polls} busy_cleared={bool}\n` appended with `OpenOptions::append(true).create(true)`; ignore write errors with `let _ =`.
- `busy_cleared` is whether the loop exited because `composer_busy` went false.

## Tests (same module, follow existing style; the injectable `sleep` collector already exists)

1. Update every existing test that references the old constants or 3s interval so it still passes with the new values.
2. `wake_delivers_immediately_when_the_composer_clears`: screens go busy, busy, clear; assert delivery after exactly 2 sleeps.
3. `wake_delivers_when_the_draft_region_stops_changing`: screens stay busy with an IDENTICAL region forever; assert delivery after exactly `WAKE_COMPOSER_STATIC_POLLS` sleeps.
4. `wake_defers_while_the_draft_region_keeps_changing`: screens stay busy and each poll appends one character to the draft line; assert delivery only at `WAKE_COMPOSER_MAX_ATTEMPTS` sleeps.
5. `wake_static_counter_resets_when_typing_resumes`: 10 identical busy screens, then one changed busy screen, then identical forever; assert delivery after `10 + 1 + WAKE_COMPOSER_STATIC_POLLS` sleeps.
6. `composer_draft_region_spans_from_the_prompt_line_to_the_end`: direct unit test - a screen with transcript `> old message`, a later `❯ draft` line, and a trailing statusline; assert the region starts at the `❯ draft` line and includes the statusline, and that a screen with no prompt-like line returns `None`.

## Gate before committing

Run from the worktree root and paste real output in your report:

```
cargo test -p qol-cli sessions
cargo fmt --check -p qol-cli
cargo clippy -p qol-cli --all-targets -- -D warnings
```

Commit on this branch with a conventional message like `fix(cli-sessions): pace wake deferral by draft activity`.
NEVER add Co-Authored-By, "Generated with", or any AI attribution to the commit message.
