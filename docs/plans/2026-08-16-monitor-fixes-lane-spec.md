# Spec: monitor round-2 fixes (command drop, hotkey uid, CLI policy)

Three defects from the architect review of the phase-1 monitor plugin.
All work happens in this worktree on branch `monitor-fixes`.
Scope: `plugins/monitor/src/daemon.rs`, `plugins/monitor/src/hotkeys.rs`, `plugins/monitor/src/cli.rs`, `plugins/monitor/src/main.rs` (one test assert), `plugins/monitor/src/platform/mod.rs` (one shared fn).
Do not touch any other file.
Code comments are banned in this repo; use self-explanatory names.

## 1. daemon.rs: heartbeat draining drops queued commands

`drain_trailing_heartbeats` uses `while let Ok(Command::Brightness { phase: Phase::Heartbeat, .. }) = rx.try_recv()`, so a non-heartbeat command (Stop phase or Kill) popped during draining is consumed and silently dropped.
A dropped Kill skips the exit restore until the SIGTERM backstop.

Fix:

```rust
fn drain_trailing_heartbeats(rx: &Receiver<Command>) -> Option<Command> {
    loop {
        match rx.try_recv() {
            Ok(Command::Brightness {
                phase: Phase::Heartbeat,
                ..
            }) => {}
            Ok(other) => return Some(other),
            Err(_) => return None,
        }
    }
}
```

In `run_loop`, when the received command is a heartbeat, capture `let carried = drain_trailing_heartbeats(rx)`, handle the heartbeat as today, and afterwards handle `carried` if present; either handle returning false drains all queued and breaks, exactly like today.

Tests (daemon.rs `mod tests`, existing style):

- `queued_kill_behind_heartbeats_still_runs_the_exit_restore`: pre-mutate a display through the session, queue `[Brightness Start, Heartbeat, Heartbeat, Kill]` on the channel, drop the sender, run `run_loop`; assert the loop exits and the stored snapshot has `clean == true` (the exit restore ran).
- `queued_stop_behind_heartbeats_halts_stepping`: queue `[Start, Heartbeat, Stop]`, drop the sender, run `run_loop`; then assert no step beyond the Start-and-first-heartbeat ones occurred even though more heartbeats were queued after the Stop: full queue `[Start, Heartbeat, Stop, Heartbeat, Heartbeat]`. Build the queue before spawning the loop thread so timing is deterministic; steps gated by debounce may make the first heartbeat a no-op, so assert on "no calls after the Stop was consumed" by comparing against the calls recorded immediately after the run, allowing either 1 or 2 pre-Stop calls.

## 2. hotkeys.rs: doctor filter never matches the tray's uid

`monitor_bindings` filters `binding.plugin_uid == PLUGIN_ID` (`"plugin-monitor"`), but qol-tray writes the manifest uid into hotkeys.json, and plugin.toml declares `uid = "d3d4cda9-f9cf-44dc-aacd-07419b5b5ea0"`, so the doctor always reports the brightness hotkeys unbound.

Fix:

- Add `pub const PLUGIN_UID: &str = "d3d4cda9-f9cf-44dc-aacd-07419b5b5ea0";` beside `PLUGIN_ID`.
- `monitor_bindings` matches `binding.plugin_uid == PLUGIN_ID || binding.plugin_uid == PLUGIN_UID`.
- In the existing `live_manifest_declares_the_headless_contract` test in `main.rs`, add an assert that the manifest's declared uid equals `plugin_monitor::hotkeys::PLUGIN_UID` (the manifest struct exposes the plugin section's uid; find the accessor rather than guessing - if the uid is only readable as a raw toml value, parse `plugin.toml` with `toml` only if that dependency already exists, otherwise assert via the loaded `PluginManifest` fields).
- hotkeys.rs tests: extend `monitor_bindings_filter_by_plugin_id` so a binding carrying the UUID also matches, and a foreign uid does not.

## 3. cli.rs: configured per-display policy is ignored

`app()` passes the raw platform control to every command, so a display configured `off` is still mutated from the CLI and `gamma`/`ddc` selections are ignored; the daemon applies them but the CLI never does.

Fix:

- Add to `plugins/monitor/src/platform/mod.rs`:

```rust
pub(crate) fn apply_configured_policies(control: &Control, device: &crate::config::DeviceConfig) {
    let stable_ids: std::collections::HashSet<String> = control
        .enumerate()
        .unwrap_or_default()
        .into_iter()
        .filter(|handle| !handle.identity_unstable())
        .map(|handle| handle.id().to_string())
        .collect();
    for (display_id, label) in &device.policy {
        if stable_ids.contains(display_id) {
            if let Some(policy) = crate::monitor::BrightnessPolicy::parse(label) {
                control.select(display_id, policy);
            }
        }
    }
}
```

- `daemon.rs` `build_runtime` replaces its inline stable-id/select block with a call to this fn.
- `cli.rs` `app()` becomes: build the control, load the device config from `crate::config::config_root()` (ignore load errors with the same eprintln pattern `daemon::run` uses, falling back to default), call `apply_configured_policies`, then build the app with the control coerced as today. The test-only `app_with` path stays unchanged.
- Test (cli.rs, existing fake style is built on `DisplayControl`, which has no `select`, so test the shared fn directly in platform/mod.rs tests instead): `configured_off_policy_refuses_cli_brightness` - build a `PolicyControl` over the existing fakes from policy.rs tests if importable; if not importable, add the test in `monitor/policy.rs` tests: apply `apply_configured_policies` with a config mapping the fake display id to `off` and assert `set_brightness` returns the Refused error, plus an `identity_unstable` handle whose configured policy is NOT applied.

## Gate before committing

Run from the worktree root and paste real output in your report:

```
cargo test -p plugin-monitor
cargo fmt --check -p plugin-monitor
cargo clippy -p plugin-monitor --all-targets -- -D warnings
```

Commit on this branch with a conventional message like `fix(monitor): honor queued commands, tray uid, and CLI policy`.
NEVER add Co-Authored-By, "Generated with", or any AI attribution to the commit message.
