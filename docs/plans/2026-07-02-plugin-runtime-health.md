# Plugin Runtime Health Implementation Plan

> **For agentic workers:** implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Per-plugin daemon liveness on `qol dev`'s Plugins page, a crash-loop doctor check, and truthful supervisor semantics underneath both.

**Architecture:** The qol-tray daemon supervisor gains pid-aware probation (P2) and each tick projects a `HealthSnapshot` published two ways: a `tokio::sync::watch` channel served verbatim by a new dev endpoint, and an atomically-renamed `daemon-health.json` read by an offline doctor check.
The `qol dev` client fetches the endpoint on the existing links cadence and renders a second status dimension per plugin row.
The reload handoff (P1) starts returning real errors so the client never misreads a failed promotion.

**Tech Stack:** Rust, tokio (watch channel), axum (dev route), serde_json, ratatui (qol dev console).

**Spec:** `docs/specs/2026-07-02-plugin-runtime-health-design.md` - read it first; it defines all semantics referenced here.

## Global Constraints

- No code comments anywhere.
- `RUSTFLAGS=-D warnings` everywhere; no `#[allow(dead_code)]`.
- Exhaustive enum matches; no `_ =>` arms on workspace enums.
- Table-driven tests with context in assertions; generic test data (`plugin-foo`, `/a/b/c`); no tests for thin wrappers.
- One-liner conventional commits, no AI attribution, no pushing.
- qol-tray dev-gated code compiles only with `--features dev`; always verify both with and without it.
- `crate::paths::runtime_dir()` is a fixed `/tmp/qol-tray` NOT redirected by `push_test_path_root`; every new file-writing function must take the path as a parameter so tests can isolate it.
- `STABLE_TICKS = 3`, existing `MAX_CONSECUTIVE_FAILURES = 5`, existing `SUPERVISION_INTERVAL = 5s`, health file name `daemon-health.json`, endpoint `GET /api/dev/plugin-health`.

---

### Task 1: P1 - fail the handoff when promotion fails

**Files:**
- Modify: `tools/qol-cli/src/dev_console/reload.rs:147-185` (`restart_child_from_prebuilt`)
- Modify: `tools/qol-cli/src/dev_console.rs:733-737` (the `ReloadOutcome::Ready` branch)
- Test: `tools/qol-cli/src/dev_console/reload.rs` (existing `mod tests`)

**Interfaces:**
- Consumes: existing `TrayHandle`, `terminate_child`, `promote_shadow_generation`.
- Produces: `restart_child_from_prebuilt` now returns `Err` on failed promotion without mutating `*child`/`*lines`; Task 6's freeze/thaw relies on this.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `reload.rs` (unix-gated because it spawns a real process):

```rust
#[cfg(unix)]
#[test]
fn abandon_failed_successor_terminates_and_reaps() {
    let child = Command::new("sleep").arg("30").spawn().unwrap();
    let mut next = TrayHandle::Owned(child);

    abandon_failed_successor(&mut next);

    assert!(
        next.try_wait().unwrap().is_some(),
        "failed successor must be reaped, not left running"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p qol-cli abandon_failed_successor -- --nocapture`
Expected: FAIL with "cannot find function `abandon_failed_successor`".

Testing scope note: the spec's full-function assertion (returns `Err`, no `*child` mutation) is enforced structurally by the early return below; a whole-function test would need a fake tray binary plus a live promotion HTTP server, which is not worth the harness.
The extracted failure path is the testable unit.

- [ ] **Step 3: Implement**

In `reload.rs`, add next to `retire_child_for_handoff`:

```rust
fn abandon_failed_successor(next: &mut TrayHandle) {
    terminate_child(next);
    let _ = next.wait();
}
```

Replace the promotion-failure block in `restart_child_from_prebuilt`:

```rust
    if let Err(error) = promote_shadow_generation(ready.port, &mut next, &next_lines, dash) {
        dash.push_log(format!(
            "[qol dev] successor promotion failed: {error:#}"
        ));
        abandon_failed_successor(&mut next);
        return Err(error);
    }
    *child = next;
```

In `dev_console.rs`, replace the `?` call so a failed handoff does not kill the console loop (the retired predecessor will surface as `ChildExited` on the next `try_wait`):

```rust
        if let ReloadOutcome::Ready = poll_reload(dash) {
            match restart_child_from_prebuilt(child, lines, dash) {
                Ok(()) => {
                    return Ok(SessionEnd::SelfRestart {
                        tray_pid: child.id(),
                    });
                }
                Err(error) => {
                    dash.push_log(format!("[qol dev] handoff failed: {error:#}"));
                    dash.notice = Some((Instant::now(), "handoff failed".to_string()));
                }
            }
        }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p qol-cli`
Expected: PASS, including the new test.

- [ ] **Step 5: Commit**

```bash
git add tools/qol-cli/src/dev_console/reload.rs tools/qol-cli/src/dev_console.rs
git commit -m "fix(qol-cli): fail the dev handoff when successor promotion fails"
```

---

### Task 2: P2 - pid-aware probation in the daemon supervisor

**Files:**
- Modify: `apps/qol-tray/src/plugins/daemon_supervisor.rs` (SupervisorState rewrite)
- Modify: `apps/qol-tray/src/plugins/manager/mod.rs` (pid accessor for respawn)

**Interfaces:**
- Consumes: existing `classify_snapshots`, `LivenessTransition`, `MAX_CONSECUTIVE_FAILURES`.
- Produces: `SupervisorState { records: HashMap<PluginId, PluginRecord>, known: HashSet<PluginId>, tick: u64 }` with `begin_tick()`, `observe_alive(&PluginId, u32)`, `observe_spawned(&PluginId, Option<u32>)`, `observe_death(&PluginId)`, `record_failure(&PluginId)`, `can_retry(&PluginId)`. Task 3 projects from `records`.

- [ ] **Step 1: Write the failing tests**

Replace `observe_alive_resets_count` and add (same `mod tests`):

```rust
#[test]
fn crash_loop_after_successful_spawn_suppresses() {
    let mut state = SupervisorState::default();
    let p = pid("plugin-foo");

    for cycle in 0..MAX_CONSECUTIVE_FAILURES {
        assert!(state.can_retry(&p), "cycle {cycle}: retry allowed pre-threshold");
        state.begin_tick();
        state.observe_spawned(&p, Some(1000 + cycle));
        state.begin_tick();
        state.observe_death(&p);
    }
    assert!(!state.can_retry(&p), "spawn-then-die cycles must suppress");
}

#[test]
fn surviving_past_stable_ticks_resets_failures() {
    let mut state = SupervisorState::default();
    let p = pid("plugin-foo");

    state.begin_tick();
    state.record_failure(&p);
    state.begin_tick();
    state.record_failure(&p);
    state.begin_tick();
    state.observe_spawned(&p, Some(1234));
    for _ in 0..STABLE_TICKS {
        state.begin_tick();
        state.observe_alive(&p, 1234);
    }
    assert!(state.records.get(&p).unwrap().stable, "must be stable after STABLE_TICKS");
    assert_eq!(state.records.get(&p).unwrap().consecutive_failures, 0, "stability resets failures");
}

#[test]
fn early_alive_observation_does_not_reset_failures() {
    let mut state = SupervisorState::default();
    let p = pid("plugin-foo");

    state.begin_tick();
    state.record_failure(&p);
    state.begin_tick();
    state.observe_alive(&p, 42);
    assert_eq!(
        state.records.get(&p).unwrap().consecutive_failures,
        1,
        "pre-stable alive must not reset the counter"
    );
}

#[test]
fn pid_change_mid_probation_restarts_probation() {
    let mut state = SupervisorState::default();
    let p = pid("plugin-foo");

    state.begin_tick();
    state.observe_alive(&p, 41);
    state.begin_tick();
    state.observe_alive(&p, 42);
    let rec = state.records.get(&p).unwrap();
    assert_eq!(rec.probation_start, Some(2), "new pid re-enters probation");
    assert!(!rec.stable, "pid change clears stability");
}

#[test]
fn death_after_stable_starts_fresh_cycle_without_increment() {
    let mut state = SupervisorState::default();
    let p = pid("plugin-foo");

    state.begin_tick();
    state.observe_spawned(&p, Some(7));
    for _ in 0..STABLE_TICKS {
        state.begin_tick();
        state.observe_alive(&p, 7);
    }
    state.begin_tick();
    state.observe_death(&p);
    assert_eq!(
        state.records.get(&p).unwrap().consecutive_failures,
        0,
        "death after stability is a fresh cycle, not a failure"
    );
}

#[test]
fn death_and_failed_respawn_on_one_tick_increment_once() {
    let mut state = SupervisorState::default();
    let p = pid("plugin-foo");

    state.begin_tick();
    state.observe_alive(&p, 9);
    state.begin_tick();
    state.observe_death(&p);
    state.record_failure(&p);
    assert_eq!(
        state.records.get(&p).unwrap().consecutive_failures,
        1,
        "one tick may add at most one failure"
    );
}
```

Existing tests that set `state.last_seen` / `state.counts` directly must be ported to `state.records.entry(id).or_default().last_seen = Some(LastSeen::Alive)` style; existing `observe_alive(id)` call sites in tests gain a pid argument.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p qol-tray daemon_supervisor`
Expected: FAIL to compile (missing `begin_tick`, `records`, `STABLE_TICKS`).

- [ ] **Step 3: Implement the state machine**

Replace `SupervisorState` (delete `counts`/`last_seen` maps):

```rust
const STABLE_TICKS: u64 = 3;

#[derive(Debug, Default, Clone, Copy)]
struct PluginRecord {
    consecutive_failures: u32,
    last_failure_tick: Option<u64>,
    last_pid: Option<u32>,
    probation_start: Option<u64>,
    stable: bool,
    last_seen: Option<LastSeen>,
}

#[derive(Default)]
struct SupervisorState {
    records: HashMap<PluginId, PluginRecord>,
    known: HashSet<PluginId>,
    tick: u64,
}

impl SupervisorState {
    fn begin_tick(&mut self) {
        self.tick += 1;
    }

    fn can_retry(&self, plugin_id: &PluginId) -> bool {
        self.records
            .get(plugin_id)
            .map_or(0, |r| r.consecutive_failures)
            < MAX_CONSECUTIVE_FAILURES
    }

    fn record_failure(&mut self, plugin_id: &PluginId) {
        let tick = self.tick;
        let rec = self.records.entry(plugin_id.clone()).or_default();
        rec.last_seen = Some(LastSeen::Dead);
        rec.last_pid = None;
        rec.probation_start = None;
        rec.stable = false;
        if rec.last_failure_tick == Some(tick) {
            return;
        }
        rec.last_failure_tick = Some(tick);
        rec.consecutive_failures += 1;
        if rec.consecutive_failures == MAX_CONSECUTIVE_FAILURES {
            log::error!(
                "Daemon supervisor: plugin {} hit {} consecutive failures, suppressing",
                plugin_id,
                MAX_CONSECUTIVE_FAILURES
            );
        }
    }

    fn observe_alive(&mut self, plugin_id: &PluginId, pid: u32) {
        let tick = self.tick;
        let rec = self.records.entry(plugin_id.clone()).or_default();
        rec.last_seen = Some(LastSeen::Alive);
        if rec.last_pid != Some(pid) {
            rec.last_pid = Some(pid);
            rec.probation_start = Some(tick);
            rec.stable = false;
            return;
        }
        if rec.stable {
            return;
        }
        if tick.saturating_sub(rec.probation_start.unwrap_or(tick)) >= STABLE_TICKS {
            rec.stable = true;
            rec.consecutive_failures = 0;
            rec.last_failure_tick = None;
        }
    }

    fn observe_spawned(&mut self, plugin_id: &PluginId, pid: Option<u32>) {
        let tick = self.tick;
        let rec = self.records.entry(plugin_id.clone()).or_default();
        rec.last_seen = Some(LastSeen::Alive);
        rec.last_pid = pid;
        rec.probation_start = pid.map(|_| tick);
        rec.stable = false;
    }

    fn observe_death(&mut self, plugin_id: &PluginId) {
        let was_stable = self.records.get(plugin_id).is_some_and(|r| r.stable);
        if was_stable {
            let rec = self.records.entry(plugin_id.clone()).or_default();
            rec.last_seen = Some(LastSeen::Dead);
            rec.last_pid = None;
            rec.probation_start = None;
            rec.stable = false;
            return;
        }
        self.record_failure(plugin_id);
    }

    fn transition_for(&self, plugin_id: &PluginId, pid: Option<u32>) -> LivenessTransition {
        let last_seen = self.records.get(plugin_id).and_then(|r| r.last_seen);
        if is_daemon_alive(pid) {
            return match last_seen {
                Some(LastSeen::Dead) => LivenessTransition::DeadToAlive,
                Some(LastSeen::Alive) | None => LivenessTransition::Alive,
            };
        }
        match last_seen {
            Some(LastSeen::Dead) => LivenessTransition::DeadStaysDead,
            Some(LastSeen::Alive) | None => LivenessTransition::AliveToDead,
        }
    }

    fn note_known_plugins(&mut self, snapshots: &[DaemonSnapshot]) {
        self.known = snapshots.iter().map(|s| s.plugin_id.clone()).collect();
    }

    fn prune_unknown_plugins(&mut self) {
        let known = std::mem::take(&mut self.known);
        self.records.retain(|id, _| known.contains(id));
    }
}
```

`TickOutcome.alive` becomes `Vec<(PluginId, u32)>`; in `classify_snapshots` push `(snap.plugin_id.clone(), pid)` guarded by `if let Some(pid) = snap.daemon_pid`.
`restart_daemon` returns the fresh pid:

```rust
fn restart_daemon(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    plugin_id: &PluginId,
) -> anyhow::Result<Option<u32>> {
    let mut manager = plugin_manager
        .lock()
        .map_err(|_| anyhow::anyhow!("plugin manager lock poisoned"))?;
    manager.ensure_plugin_daemon_running(plugin_id.as_str())?;
    Ok(manager.plugin_daemon_pid(plugin_id))
}
```

Add to `PluginManager` (`manager/mod.rs`):

```rust
    pub fn plugin_daemon_pid(&self, plugin_id: &PluginId) -> Option<u32> {
        self.plugins.get(plugin_id).and_then(|plugin| plugin.daemon_pid())
    }
```

`supervise_once` becomes:

```rust
    state.begin_tick();
    reap_exited_daemons(plugin_manager);
    let snapshots = snapshot_daemons(plugin_manager);
    let outcome = classify_snapshots(&snapshots, state);
    state.note_known_plugins(&snapshots);

    for (id, pid) in &outcome.alive {
        state.observe_alive(id, *pid);
    }
    for id in &outcome.fresh_deaths {
        state.observe_death(id);
    }

    let mut any_state_change =
        !outcome.fresh_deaths.is_empty() || !outcome.fresh_recoveries.is_empty();
    for plugin_id in outcome.retriable_dead {
        if !state.can_retry(&plugin_id) {
            continue;
        }
        match restart_daemon(plugin_manager, &plugin_id) {
            Ok(pid) => {
                state.observe_spawned(&plugin_id, pid);
                any_state_change = true;
                log::info!("Restarted dead daemon for plugin {}", plugin_id);
            }
            Err(e) => {
                state.record_failure(&plugin_id);
                any_state_change = true;
                log::warn!("Failed to restart daemon for plugin {}: {}", plugin_id, e);
            }
        }
    }
    state.prune_unknown_plugins();
```

The `can_retry` re-check inside the loop matters: `observe_death` may have just consumed the last allowed failure.

- [ ] **Step 4: Run tests**

Run: `cargo test -p qol-tray daemon_supervisor && cargo test -p qol-tray --features dev daemon_supervisor`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/qol-tray/src/plugins/daemon_supervisor.rs apps/qol-tray/src/plugins/manager/mod.rs
git commit -m "fix(qol-tray): suppress crash-looping daemons via pid-aware probation"
```

---

### Task 3: health projection and publishing

**Files:**
- Create: `apps/qol-tray/src/plugins/daemon_health.rs`
- Modify: `apps/qol-tray/src/plugins/mod.rs` (add `pub mod daemon_health;`)
- Modify: `apps/qol-tray/src/plugins/manager/autostart.rs` (expectation classifier)
- Modify: `apps/qol-tray/src/plugins/manager/mod.rs` (replace `supervised_daemon_snapshots`)
- Modify: `apps/qol-tray/src/plugins/daemon_supervisor.rs` (snapshot shape, projection, publish)
- Modify: `apps/qol-tray/src/dev_generation.rs` (id accessor)
- Modify: `apps/qol-tray/src/main.rs:513-531` (channel wiring)

**Interfaces:**
- Consumes: Task 2's `SupervisorState.records`, `dev_generation::{is_shadow, daemon_autostart_held}`, `paths::runtime_dir()`.
- Produces: `daemon_health::{DaemonExpectation, PluginRuntimeStatus, PluginHealth, HealthSnapshot, HealthPublisher, channel(), default_file_path()}`; `PluginManager::daemon_health_snapshots() -> Vec<(PluginId, DaemonExpectation, Option<u32>)>`; `spawn_supervisor(manager, shutdown_rx, publisher: HealthPublisher)`. Tasks 4-6 consume `HealthSnapshot`'s wire shape.

- [ ] **Step 1: Write the failing tests**

In the new `daemon_health.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_serde_round_trips_every_variant() {
        let cases = [
            (PluginRuntimeStatus::NotExpected, r#"{"state":"not_expected"}"#),
            (PluginRuntimeStatus::AutostartBlocked, r#"{"state":"autostart_blocked"}"#),
            (
                PluginRuntimeStatus::Down { consecutive_failures: 5, suppressed: true },
                r#"{"state":"down","consecutive_failures":5,"suppressed":true}"#,
            ),
            (
                PluginRuntimeStatus::Probation { pid: 12, consecutive_failures: 1 },
                r#"{"state":"probation","pid":12,"consecutive_failures":1}"#,
            ),
            (
                PluginRuntimeStatus::Stable { pid: 12 },
                r#"{"state":"stable","pid":12}"#,
            ),
        ];
        for (status, expected_json) in cases {
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(json, expected_json, "serialize {status:?}");
            let back: PluginRuntimeStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(back, status, "round trip {expected_json}");
        }
    }

    #[test]
    fn publisher_writes_file_and_watch_atomically_consistent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("daemon-health.json");
        let (tx, rx) = channel();
        let publisher = HealthPublisher::new(tx, 42700, path.clone());

        publisher.publish(
            3,
            vec![PluginHealth {
                plugin_id: "plugin-foo".to_string(),
                status: PluginRuntimeStatus::Stable { pid: 12 },
            }],
        );

        let from_watch = rx.borrow().clone();
        assert_eq!(from_watch.tick, 3, "watch carries the published tick");
        assert_eq!(from_watch.bind_port, 42700);
        assert_eq!(from_watch.process_pid, std::process::id());
        let from_file: HealthSnapshot =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(from_file, from_watch, "both transports carry one snapshot");
    }
}
```

In `daemon_supervisor.rs`, projection tests (table-driven):

```rust
    #[test]
    fn projection_covers_every_status_variant() {
        let mut state = SupervisorState::default();
        state.begin_tick();
        state.observe_alive(&pid("plugin-probation"), 11);
        state.observe_spawned(&pid("plugin-stable"), Some(12));
        for _ in 0..STABLE_TICKS {
            state.begin_tick();
            state.observe_alive(&pid("plugin-stable"), 12);
        }
        state.begin_tick();
        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            state.begin_tick();
            state.record_failure(&pid("plugin-suppressed"));
        }

        use crate::plugins::daemon_health::{DaemonExpectation, PluginRuntimeStatus};
        let cases = [
            ("plugin-no-daemon", DaemonExpectation::NotExpected, None,
             PluginRuntimeStatus::NotExpected),
            ("plugin-blocked", DaemonExpectation::AutostartBlocked, None,
             PluginRuntimeStatus::AutostartBlocked),
            ("plugin-unseen", DaemonExpectation::Supervised, None,
             PluginRuntimeStatus::Down { consecutive_failures: 0, suppressed: false }),
            ("plugin-probation", DaemonExpectation::Supervised, Some(11),
             PluginRuntimeStatus::Probation { pid: 11, consecutive_failures: 0 }),
            ("plugin-stable", DaemonExpectation::Supervised, Some(12),
             PluginRuntimeStatus::Stable { pid: 12 }),
            ("plugin-suppressed", DaemonExpectation::Supervised, None,
             PluginRuntimeStatus::Down {
                 consecutive_failures: MAX_CONSECUTIVE_FAILURES,
                 suppressed: true,
             }),
        ];
        for (id, expectation, daemon_pid, expected) in cases {
            let snap = DaemonSnapshot {
                plugin_id: pid(id),
                expectation,
                daemon_pid,
            };
            assert_eq!(state.status_for(&snap), expected, "plugin: {id}");
        }
    }
```

Port the manager test `supervised_daemon_snapshots_respect_daemon_autostart_policy` to assert `daemon_health_snapshots` expectations: installed+daemon -> `Supervised`, dev-linked without marker -> `AutostartBlocked`, dev-linked with marker -> `Supervised`, no daemon -> `NotExpected`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p qol-tray daemon_health; cargo test -p qol-tray daemon_supervisor`
Expected: FAIL to compile (module and types missing).

- [ ] **Step 3: Implement `daemon_health.rs`**

```rust
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::sync::watch;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonExpectation {
    NotExpected,
    AutostartBlocked,
    Supervised,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PluginRuntimeStatus {
    NotExpected,
    AutostartBlocked,
    Down { consecutive_failures: u32, suppressed: bool },
    Probation { pid: u32, consecutive_failures: u32 },
    Stable { pid: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginHealth {
    pub plugin_id: String,
    pub status: PluginRuntimeStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct HealthSnapshot {
    #[serde(default)]
    pub tick: u64,
    #[serde(default)]
    pub process_pid: u32,
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub bind_port: u16,
    #[serde(default)]
    pub daemon_autostart_held: bool,
    #[serde(default)]
    pub generation_id: Option<String>,
    #[serde(default)]
    pub plugins: Vec<PluginHealth>,
}

pub fn channel() -> (watch::Sender<HealthSnapshot>, watch::Receiver<HealthSnapshot>) {
    watch::channel(HealthSnapshot::default())
}

pub fn default_file_path() -> PathBuf {
    crate::paths::runtime_dir().join("daemon-health.json")
}

pub struct HealthPublisher {
    tx: watch::Sender<HealthSnapshot>,
    bind_port: u16,
    file_path: PathBuf,
}

impl HealthPublisher {
    pub fn new(tx: watch::Sender<HealthSnapshot>, bind_port: u16, file_path: PathBuf) -> Self {
        Self { tx, bind_port, file_path }
    }

    pub fn publish(&self, tick: u64, plugins: Vec<PluginHealth>) {
        let snapshot = HealthSnapshot {
            tick,
            process_pid: std::process::id(),
            role: if crate::dev_generation::is_shadow() { "shadow" } else { "stable" }.to_string(),
            bind_port: self.bind_port,
            daemon_autostart_held: crate::dev_generation::daemon_autostart_held(),
            generation_id: crate::dev_generation::current().generation_id(),
            plugins,
        };
        if let Err(error) = write_snapshot_file(&self.file_path, &snapshot) {
            log::warn!("Failed to write daemon health snapshot: {error:#}");
        }
        self.tx.send_replace(snapshot);
    }
}

fn write_snapshot_file(path: &Path, snapshot: &HealthSnapshot) -> anyhow::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("health file path has no parent"))?;
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(snapshot)?)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}
```

`send_replace` (not `send`) so publishing succeeds with zero receivers (prod builds drop the receiver).
Add to `dev_generation.rs` `impl GenerationContext` (the `id` field is private):

```rust
    pub fn generation_id(&self) -> Option<String> {
        self.id.clone()
    }
```

- [ ] **Step 4: Implement classifier and manager snapshot**

`autostart.rs`:

```rust
pub(super) fn daemon_expectation(plugin: &Plugin) -> crate::plugins::daemon_health::DaemonExpectation {
    use crate::plugins::daemon_health::DaemonExpectation;
    if !daemon_enabled(plugin) {
        return DaemonExpectation::NotExpected;
    }
    if daemon_auto_managed(plugin) {
        DaemonExpectation::Supervised
    } else {
        DaemonExpectation::AutostartBlocked
    }
}
```

`manager/mod.rs` - replace `supervised_daemon_snapshots` (update its one caller in Task 2's supervisor and the manager test; do not keep both):

```rust
    pub fn daemon_health_snapshots(
        &self,
    ) -> Vec<(PluginId, crate::plugins::daemon_health::DaemonExpectation, Option<u32>)> {
        self.plugins
            .values()
            .map(|plugin| {
                (
                    plugin.id.clone(),
                    autostart::daemon_expectation(plugin),
                    plugin.daemon_pid(),
                )
            })
            .collect()
    }
```

- [ ] **Step 5: Implement supervisor projection and publish**

`DaemonSnapshot` gains `expectation: DaemonExpectation`; `snapshot_daemons` maps the new manager method.
`classify_snapshots` skips non-supervised entries:

```rust
    for snap in snapshots {
        if snap.expectation != DaemonExpectation::Supervised {
            continue;
        }
        ...
    }
```

Projection on `SupervisorState`:

```rust
    fn project(&self, snapshots: &[DaemonSnapshot]) -> Vec<PluginHealth> {
        snapshots
            .iter()
            .map(|snap| PluginHealth {
                plugin_id: snap.plugin_id.as_str().to_string(),
                status: self.status_for(snap),
            })
            .collect()
    }

    fn status_for(&self, snap: &DaemonSnapshot) -> PluginRuntimeStatus {
        match snap.expectation {
            DaemonExpectation::NotExpected => PluginRuntimeStatus::NotExpected,
            DaemonExpectation::AutostartBlocked => PluginRuntimeStatus::AutostartBlocked,
            DaemonExpectation::Supervised => {
                let Some(rec) = self.records.get(&snap.plugin_id) else {
                    return PluginRuntimeStatus::Down { consecutive_failures: 0, suppressed: false };
                };
                if let (Some(LastSeen::Alive), Some(pid)) = (rec.last_seen, rec.last_pid) {
                    if rec.stable {
                        PluginRuntimeStatus::Stable { pid }
                    } else {
                        PluginRuntimeStatus::Probation {
                            pid,
                            consecutive_failures: rec.consecutive_failures,
                        }
                    }
                } else {
                    PluginRuntimeStatus::Down {
                        consecutive_failures: rec.consecutive_failures,
                        suppressed: rec.consecutive_failures >= MAX_CONSECUTIVE_FAILURES,
                    }
                }
            }
        }
    }
```

`spawn_supervisor` gains `publisher: HealthPublisher`, threads it to `run_supervision_loop` and `supervise_once`; the last line of `supervise_once` (after `prune_unknown_plugins`, before the hotkey trigger) becomes:

```rust
    publisher.publish(state.tick, state.project(&snapshots));
```

The `daemon_autostart_held()` early-return stays FIRST, before `begin_tick` - a shadow generation never publishes.
`main.rs` wiring - before `Plugins::start_server`:

```rust
    let (health_tx, health_rx) = qol_tray::plugins::daemon_health::channel();
```

and replace the `spawn_supervisor` call (after `ui_port` exists):

```rust
    qol_tray::plugins::daemon_supervisor::spawn_supervisor(
        plugin_manager.clone(),
        shutdown_tx.subscribe(),
        qol_tray::plugins::daemon_health::HealthPublisher::new(
            health_tx,
            ui_port,
            qol_tray::plugins::daemon_health::default_file_path(),
        ),
    );
```

`health_rx` is consumed by Task 5 (dev feature); until then hold it alive with `let _health_rx = health_rx;` so both feature configurations compile warning-free at this commit.

- [ ] **Step 6: Run tests**

Run: `cargo test -p qol-tray && cargo test -p qol-tray --features dev && cargo clippy -p qol-tray --all-targets -- -D warnings && cargo clippy -p qol-tray --all-targets --features dev -- -D warnings`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add apps/qol-tray/src/plugins/ apps/qol-tray/src/dev_generation.rs apps/qol-tray/src/main.rs
git commit -m "feat(qol-tray): project and publish daemon health snapshots"
```

---

### Task 4: offline doctor check `plugin_daemon_health`

**Files:**
- Create: `apps/qol-tray/src/doctor/checks/plugin_daemon_health.rs`
- Modify: `apps/qol-tray/src/doctor/checks.rs` (mod + registry, dev-gated)

**Interfaces:**
- Consumes: `daemon_health::{HealthSnapshot, PluginRuntimeStatus, default_file_path}`, `process_utils::is_pid_alive`, doctor `framework` types.
- Produces: check id `plugin_daemon_health`, group `dev-loop`, warn message `daemon crash loop suppressed for: <ids>`.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::daemon_health::{HealthSnapshot, PluginHealth, PluginRuntimeStatus};

    const DEAD_PID: u32 = 99_999_999;

    fn snapshot(process_pid: u32, statuses: Vec<(&str, PluginRuntimeStatus)>) -> String {
        serde_json::to_string(&HealthSnapshot {
            tick: 1,
            process_pid,
            plugins: statuses
                .into_iter()
                .map(|(id, status)| PluginHealth { plugin_id: id.to_string(), status })
                .collect(),
            ..HealthSnapshot::default()
        })
        .unwrap()
    }

    #[test]
    fn diagnose_table() {
        let alive = std::process::id();
        let suppressed = PluginRuntimeStatus::Down { consecutive_failures: 5, suppressed: true };
        let transient = PluginRuntimeStatus::Down { consecutive_failures: 1, suppressed: false };
        let cases = [
            ("missing file", None, false, ""),
            ("corrupt file", Some("not json".to_string()), false, ""),
            ("stale writer pid", Some(snapshot(DEAD_PID, vec![("plugin-foo", suppressed.clone())])), false, ""),
            ("suppressed plugin warns", Some(snapshot(alive, vec![("plugin-foo", suppressed)])), true, "plugin-foo"),
            ("transient down is ok", Some(snapshot(alive, vec![("plugin-foo", transient)])), false, ""),
        ];
        for (label, contents, expect_warn, expect_in_message) in cases {
            let tmp = tempfile::TempDir::new().unwrap();
            let path = tmp.path().join("daemon-health.json");
            if let Some(contents) = contents {
                std::fs::write(&path, contents).unwrap();
            }
            let report = diagnose(&path);
            assert_eq!(report.is_warn(), expect_warn, "{label}");
            if expect_warn {
                assert!(report.summary().contains(expect_in_message), "{label}: {}", report.summary());
            }
        }
    }
}
```

Adapt the two assertion helpers to `CheckReport`'s actual accessors (see how `plugin_staleness` tests assert on reports; reuse the same accessors instead of inventing `is_warn`/`summary` if they differ).

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p qol-tray --features dev plugin_daemon_health`
Expected: FAIL to compile (module missing).

- [ ] **Step 3: Implement**

```rust
use super::super::framework::{CheckCategory, CheckMeta, CheckReport, DoctorCheck, DoctorContext};
use crate::plugins::daemon_health::{HealthSnapshot, PluginRuntimeStatus};
use std::path::Path;

const ID: &str = "plugin_daemon_health";

pub(super) struct PluginDaemonHealthCheck;

impl DoctorCheck for PluginDaemonHealthCheck {
    fn meta(&self) -> CheckMeta {
        CheckMeta::new(ID, "Plugin daemon health", CheckCategory::Runtime)
            .group(&["dev-loop"])
            .dev_only()
    }

    fn run(&self, _ctx: &DoctorContext) -> CheckReport {
        diagnose(&crate::plugins::daemon_health::default_file_path())
    }
}

fn diagnose(path: &Path) -> CheckReport {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return CheckReport::ok("no daemon health snapshot (qol-tray not running here)");
    };
    let snapshot: HealthSnapshot = match serde_json::from_str(&raw) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return CheckReport::ok(format!("unreadable daemon health snapshot: {error}"))
        }
    };
    if !crate::process_utils::is_pid_alive(snapshot.process_pid as i32) {
        return CheckReport::ok("stale daemon health snapshot (qol-tray not running)");
    }
    let suppressed: Vec<&str> = snapshot
        .plugins
        .iter()
        .filter(|plugin| {
            matches!(
                plugin.status,
                PluginRuntimeStatus::Down { suppressed: true, .. }
            )
        })
        .map(|plugin| plugin.plugin_id.as_str())
        .collect();
    if suppressed.is_empty() {
        return CheckReport::ok("no crash-looped plugin daemons");
    }
    CheckReport::warn(
        format!("daemon crash loop suppressed for: {}", suppressed.join(", ")),
        ID,
        Vec::new(),
    )
}
```

Register in `checks.rs`: `#[cfg(feature = "dev")] mod plugin_daemon_health;` and inside the existing `#[cfg(feature = "dev")]` block of `registry()`:

```rust
        checks.push(Box::new(plugin_daemon_health::PluginDaemonHealthCheck));
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p qol-tray --features dev plugin_daemon_health`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add apps/qol-tray/src/doctor/
git commit -m "feat(qol-tray): doctor check surfacing suppressed daemon crash loops"
```

---

### Task 5: `GET /api/dev/plugin-health` endpoint

**Files:**
- Create: `apps/qol-tray/src/features/plugin_store/server/dev_health_handlers.rs`
- Modify: `apps/qol-tray/src/features/plugin_store/server.rs:195-201` (merge routes in `dev_api_router`)
- Modify: `apps/qol-tray/src/features/plugin_store/server/types.rs` (AppState field)
- Modify: the `AppState::new` -> `start_ui_server` -> `Plugins::start_server` -> `main.rs` parameter chain (follow the `core_log_controls` threading pattern exactly - it is the same dev-gated shape)

**Interfaces:**
- Consumes: Task 3's `health_rx` (`watch::Receiver<HealthSnapshot>`) currently parked in `main.rs`.
- Produces: `GET /api/dev/plugin-health` returning the last published `HealthSnapshot` as JSON.

- [ ] **Step 1: Implement (no new test - thin wrapper; wire shape covered by Task 3's serde tests)**

`dev_health_handlers.rs`:

```rust
use axum::{extract::State, routing::get, Json, Router};

use crate::plugins::daemon_health::HealthSnapshot;

use super::types::AppState;

pub(super) fn routes() -> Router<AppState> {
    Router::new().route("/dev/plugin-health", get(get_plugin_health))
}

pub(super) async fn get_plugin_health(State(state): State<AppState>) -> Json<HealthSnapshot> {
    Json(state.daemon_health.borrow().clone())
}
```

`types.rs` AppState:

```rust
    #[cfg(feature = "dev")]
    pub(super) daemon_health: tokio::sync::watch::Receiver<crate::plugins::daemon_health::HealthSnapshot>,
```

Thread the receiver from `main.rs` through `Plugins::start_server` and `start_ui_server` into `AppState::new` as a `#[cfg(feature = "dev")]` parameter, exactly like `core_log_controls`; remove Task 3's `let _health_rx` placeholder binding, and in non-dev builds drop the receiver at the `main.rs` boundary the same way `core_log_controls` handles it.
Add `mod dev_health_handlers;` next to the sibling `mod` declarations in the server module and merge in `dev_api_router`:

```rust
        .merge(dev_health_handlers::routes())
```

- [ ] **Step 2: Verify both feature configurations**

Run: `cargo clippy -p qol-tray --all-targets -- -D warnings && cargo clippy -p qol-tray --all-targets --features dev -- -D warnings && cargo test -p qol-tray --features dev`
Expected: PASS; no unused-variable warning in either configuration.

- [ ] **Step 3: Manual probe**

Run: `cargo build -p qol-tray --features dev`, start via `qol dev`, then `curl -s localhost:42700/api/dev/plugin-health | head -c 400`
Expected: JSON with `tick > 0`, own `process_pid`, and one entry per loaded plugin.

- [ ] **Step 4: Commit**

```bash
git add apps/qol-tray/src/features/plugin_store/ apps/qol-tray/src/main.rs
git commit -m "feat(qol-tray): dev endpoint serving daemon health snapshots"
```

---

### Task 6: `qol dev` Plugins page health column

**Files:**
- Modify: `tools/qol-cli/src/dev_server.rs` (fetch + wire types)
- Modify: `tools/qol-cli/src/dev_console.rs` (poller, Dash field, row rendering)

**Interfaces:**
- Consumes: Task 5's endpoint; existing `Poller`, `LinksState`, `plugin_row_line`, `dash.is_reloading()`.
- Produces: `fetch_plugin_health() -> Result<PluginHealthSnapshot>`, `PluginDaemonStatus` mirroring the server's serde shape, a daemon-status span per plugin row.

- [ ] **Step 1: Write the failing tests**

In `dev_console.rs` tests, next to the existing `plugin_row_line` tests:

```rust
    #[test]
    fn plugin_row_line_renders_daemon_status_dimension() {
        let row = linked_row();
        let cases = [
            (None, ""),
            (Some(PluginDaemonStatus::NotExpected), ""),
            (Some(PluginDaemonStatus::AutostartBlocked), "daemon off"),
            (Some(PluginDaemonStatus::Stable { pid: 1 }), "running"),
            (Some(PluginDaemonStatus::Probation { pid: 1, consecutive_failures: 0 }), "starting"),
            (Some(PluginDaemonStatus::Down { consecutive_failures: 1, suppressed: false }), "dead"),
            (Some(PluginDaemonStatus::Down { consecutive_failures: 5, suppressed: true }), "crash-looped"),
        ];
        for (status, expected) in cases {
            let text = span_text(&plugin_row_line(&row, status.as_ref(), false).spans);
            if expected.is_empty() {
                assert!(!text.contains("·  "), "no dangling separator for {status:?}");
            } else {
                assert!(text.contains(expected), "{status:?} renders {expected}, got: {text}");
            }
        }
    }
```

Reuse the existing `linked_row`/`span_text` helpers from the neighboring tests (create `linked_row` from the same builder those tests use).
In `dev_server.rs` tests, a serde round trip:

```rust
    #[test]
    fn plugin_health_payload_parses_tagged_statuses() {
        let body = r#"{"tick":4,"process_pid":1,"role":"stable","bind_port":42700,
            "daemon_autostart_held":false,"generation_id":null,
            "plugins":[{"plugin_id":"plugin-foo","status":{"state":"down","consecutive_failures":5,"suppressed":true}}]}"#;
        let snapshot: PluginHealthSnapshot = serde_json::from_str(body).unwrap();
        assert_eq!(snapshot.tick, 4);
        assert_eq!(
            snapshot.plugins[0].status,
            PluginDaemonStatus::Down { consecutive_failures: 5, suppressed: true }
        );
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p qol-cli plugin_health; cargo test -p qol-cli plugin_row_line`
Expected: FAIL to compile (types missing, `plugin_row_line` arity).

- [ ] **Step 3: Implement fetch layer** (`dev_server.rs`)

```rust
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub(crate) struct PluginHealthSnapshot {
    #[serde(default)]
    pub(crate) tick: u64,
    #[serde(default)]
    pub(crate) daemon_autostart_held: bool,
    #[serde(default)]
    pub(crate) plugins: Vec<PluginHealthRow>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct PluginHealthRow {
    pub(crate) plugin_id: String,
    pub(crate) status: PluginDaemonStatus,
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum PluginDaemonStatus {
    NotExpected,
    AutostartBlocked,
    Down { consecutive_failures: u32, suppressed: bool },
    Probation { pid: u32, consecutive_failures: u32 },
    Stable { pid: u32 },
}

fn plugin_health_url() -> String {
    api_url("/api/dev/plugin-health")
}

pub(crate) fn fetch_plugin_health() -> Result<PluginHealthSnapshot> {
    let url = plugin_health_url();
    let (status, body) = http_exchange("GET", &url, None)?;
    if status != 200 {
        bail!("GET {url} returned {status}");
    }
    serde_json::from_str(&body).context("invalid plugin health payload")
}
```

Unused fields trip `-D warnings` via dead_code: consume `tick` (treat `tick == 0` as no data: return health `None`) and `daemon_autostart_held` (map to `None` health as well - held means nothing is running yet) inside the fetch adapter rather than leaving them unread, e.g.:

```rust
pub(crate) fn fetch_plugin_health_rows() -> Result<Option<Vec<PluginHealthRow>>> {
    let snapshot = fetch_plugin_health()?;
    if snapshot.tick == 0 || snapshot.daemon_autostart_held {
        return Ok(None);
    }
    Ok(Some(snapshot.plugins))
}
```

- [ ] **Step 4: Implement console consumption** (`dev_console.rs`)

Extend the links poller closure (no new timer; health failure degrades to `None` instead of failing links):

```rust
            links: Poller::spawn(LINKS_REFRESH_INTERVAL, || {
                fetch_workspace_plugins()
                    .map(|plugins| (plugins, fetch_plugin_health_rows().ok().flatten()))
                    .map_err(|error| format!("{error:#}"))
            }),
```

Adjust the `Poller` type parameter and the drain site; freeze health during a reload (P1 guarantees the flag is truthful):

```rust
        if let Some(outcome) = probes.links.latest() {
            match outcome {
                Ok((links, health)) => {
                    if !dash.is_reloading() {
                        dash.health = health;
                    }
                    dash.links = LinksState::Live(links);
                }
                Err(_) => {
                    dash.health = None;
                    dash.links = LinksState::Unreachable;
                }
            }
        }
```

Add `health: Option<Vec<PluginHealthRow>>` to `Dash` (initialize `None`).
`plugin_row_line` gains the status parameter; `draw_plugins` resolves it per row:

```rust
        .map(|(index, row)| {
            let status = dash.health.as_deref().and_then(|rows| {
                rows.iter()
                    .find(|health| health.plugin_id == row.id)
                    .map(|health| &health.status)
            });
            plugin_row_line(row, status, index == cursor)
        })
```

Status span, appended as the last span in `plugin_row_line`:

```rust
fn daemon_status_span(status: Option<&PluginDaemonStatus>) -> Option<Span<'static>> {
    match status? {
        PluginDaemonStatus::NotExpected => None,
        PluginDaemonStatus::AutostartBlocked => Some(" · daemon off".fg(Color::DarkGray)),
        PluginDaemonStatus::Stable { pid: _ } => Some(" · running".fg(Color::Green)),
        PluginDaemonStatus::Probation { pid: _, consecutive_failures: _ } => {
            Some(" · starting".fg(Color::Yellow))
        }
        PluginDaemonStatus::Down { consecutive_failures: _, suppressed: true } => {
            Some(" · crash-looped".fg(Color::Red).bold())
        }
        PluginDaemonStatus::Down { consecutive_failures: _, suppressed: false } => {
            Some(" · dead".fg(Color::Red))
        }
    }
}
```

Update the two existing `plugin_row_line` tests to pass `None` for the new parameter.
Extend Task 1's `Err` branch in `dev_console.rs` with `dash.health = None;` so a failed handoff renders unknown rather than the frozen predecessor snapshot (the spec's keep-stale/unknown rule).
The reload/handoff visual state needs no new chrome beyond that: the breadcrumb already carries the global `RELOADING` flag, freezing `dash.health` satisfies the never-reinterpret rule, and Task 1 already posts the failure notice.

- [ ] **Step 5: Run tests**

Run: `cargo test -p qol-cli && cargo clippy -p qol-cli --all-targets -- -D warnings`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add tools/qol-cli/src/dev_server.rs tools/qol-cli/src/dev_console.rs
git commit -m "feat(qol-cli): daemon health column on the dev plugins page"
```

---

### Task 7: end-to-end verification (no commit)

- [ ] **Step 1: Full gate**

Run, from the repo root:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p qol-tray --all-targets --features dev -- -D warnings
cargo test --workspace
cargo test -p qol-tray --features dev
qol build
```

Expected: all green. The `--features dev` runs are mandatory - default-feature runs skip every dev-gated path this plan touches.

- [ ] **Step 2: Live crash-loop drill**

1. Start `qol dev`; confirm the Plugins page shows `running` for daemon plugins after ~15s (probation -> stable).
2. Pick a dev-linked daemon plugin, `kill -9` its pid; confirm the row passes through `dead` -> `starting` -> `running`.
3. Make it crash-loop (temporary `panic!` at daemon start, rebuild the plugin); confirm the row reaches `crash-looped` within ~1 min, `qol dev`'s Doctor panel warns `daemon crash loop suppressed for: <id>`, and `/tmp/qol-tray/daemon-health.json` shows `"suppressed":true`.
4. Revert the panic, press the row's rebuild/heal path, confirm recovery to `running`.
5. Trigger an ARMED Ctrl+R reload; confirm health freezes during the handoff and resumes against the successor (`process_pid` changes in the endpoint payload).

- [ ] **Step 3: Report**

Report the four-part completion summary (ran/happened/inferred/commits) with real command output.
