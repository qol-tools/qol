use crate::plugins::daemon_health::{
    DaemonExpectation, HealthPublisher, PluginHealth, PluginRuntimeStatus,
};
use crate::plugins::{PluginId, PluginManager};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;

const SUPERVISION_INTERVAL: Duration = Duration::from_secs(5);
const MAX_CONSECUTIVE_FAILURES: u32 = 5;
const STABLE_TICKS: u64 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
struct DaemonSnapshot {
    plugin_id: PluginId,
    expectation: DaemonExpectation,
    daemon_pid: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LivenessTransition {
    AliveToDead,
    DeadStaysDead,
    DeadToAlive,
    Alive,
}

pub fn spawn_supervisor(
    plugin_manager: Arc<Mutex<PluginManager>>,
    shutdown_rx: broadcast::Receiver<()>,
    publisher: HealthPublisher,
) {
    tokio::spawn(async move {
        run_supervision_loop(plugin_manager, shutdown_rx, publisher).await;
    });
}

async fn run_supervision_loop(
    plugin_manager: Arc<Mutex<PluginManager>>,
    mut shutdown_rx: broadcast::Receiver<()>,
    publisher: HealthPublisher,
) {
    let mut state = SupervisorState::default();
    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                log::info!("Daemon supervisor shutting down");
                return;
            }
            _ = tokio::time::sleep(SUPERVISION_INTERVAL) => {
                supervise_once(&plugin_manager, &mut state, &publisher);
            }
        }
    }
}

fn supervise_once(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    state: &mut SupervisorState,
    publisher: &HealthPublisher,
) {
    // DeadStaysDead is retriable, so without this gate the supervisor would
    // start held-back daemons on its first tick, while the predecessor
    // generation's twins are still alive.
    if crate::dev_generation::daemon_autostart_held() {
        return;
    }
    if let Err(error) = reconcile_profile_generation(plugin_manager) {
        log::error!("Daemon supervisor failed to reconcile profile generation: {error:#}");
        return;
    }
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
    publisher.publish(state.tick, state.project(&snapshots));

    if any_state_change {
        crate::hotkeys::trigger_reload();
    }
}

fn reconcile_profile_generation(plugin_manager: &Arc<Mutex<PluginManager>>) -> anyhow::Result<()> {
    let mut manager = plugin_manager
        .lock()
        .map_err(|_| anyhow::anyhow!("plugin manager lock poisoned"))?;
    manager.reconcile_profile_generation()?;
    Ok(())
}

#[derive(Default)]
struct TickOutcome {
    alive: Vec<(PluginId, u32)>,
    fresh_deaths: Vec<PluginId>,
    fresh_recoveries: Vec<PluginId>,
    retriable_dead: Vec<PluginId>,
}

fn classify_snapshots(snapshots: &[DaemonSnapshot], state: &SupervisorState) -> TickOutcome {
    let mut outcome = TickOutcome::default();
    for snap in snapshots {
        if snap.expectation != DaemonExpectation::Supervised {
            continue;
        }
        match state.transition_for(&snap.plugin_id, snap.daemon_pid) {
            LivenessTransition::Alive => {
                if let Some(pid) = snap.daemon_pid {
                    outcome.alive.push((snap.plugin_id.clone(), pid));
                }
            }
            LivenessTransition::DeadToAlive => {
                if let Some(pid) = snap.daemon_pid {
                    outcome.alive.push((snap.plugin_id.clone(), pid));
                    outcome.fresh_recoveries.push(snap.plugin_id.clone());
                }
            }
            LivenessTransition::AliveToDead => {
                outcome.fresh_deaths.push(snap.plugin_id.clone());
                if state.can_retry(&snap.plugin_id) {
                    outcome.retriable_dead.push(snap.plugin_id.clone());
                }
            }
            LivenessTransition::DeadStaysDead => {
                if state.can_retry(&snap.plugin_id) {
                    outcome.retriable_dead.push(snap.plugin_id.clone());
                }
            }
        }
    }
    outcome
}

fn reap_exited_daemons(plugin_manager: &Arc<Mutex<PluginManager>>) {
    let Ok(mut manager) = plugin_manager.lock() else {
        log::error!("Daemon supervisor: plugin manager lock poisoned");
        return;
    };
    manager.reap_exited_daemons();
}

fn snapshot_daemons(plugin_manager: &Arc<Mutex<PluginManager>>) -> Vec<DaemonSnapshot> {
    let Ok(manager) = plugin_manager.lock() else {
        log::error!("Daemon supervisor: plugin manager lock poisoned");
        return Vec::new();
    };
    manager
        .daemon_health_snapshots()
        .into_iter()
        .map(|(plugin_id, expectation, daemon_pid)| DaemonSnapshot {
            plugin_id,
            expectation,
            daemon_pid,
        })
        .collect()
}

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

fn is_daemon_alive(daemon_pid: Option<u32>) -> bool {
    let Some(pid) = daemon_pid else {
        return false;
    };
    crate::process_utils::is_pid_alive(pid as i32)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LastSeen {
    Alive,
    Dead,
}

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
            .map_or(0, |record| record.consecutive_failures)
            < MAX_CONSECUTIVE_FAILURES
    }

    fn record_failure(&mut self, plugin_id: &PluginId) {
        let tick = self.tick;
        let record = self.records.entry(plugin_id.clone()).or_default();
        record.last_seen = Some(LastSeen::Dead);
        record.last_pid = None;
        record.probation_start = None;
        record.stable = false;
        if record.last_failure_tick == Some(tick) {
            return;
        }
        record.last_failure_tick = Some(tick);
        record.consecutive_failures += 1;
        if record.consecutive_failures == MAX_CONSECUTIVE_FAILURES {
            log::error!(
                "Daemon supervisor: plugin {} hit {} consecutive failures, suppressing",
                plugin_id,
                MAX_CONSECUTIVE_FAILURES
            );
        }
    }

    fn observe_alive(&mut self, plugin_id: &PluginId, pid: u32) {
        let tick = self.tick;
        let record = self.records.entry(plugin_id.clone()).or_default();
        record.last_seen = Some(LastSeen::Alive);
        if record.last_pid != Some(pid) {
            record.last_pid = Some(pid);
            record.probation_start = Some(tick);
            record.stable = false;
            return;
        }
        if record.stable {
            return;
        }
        if tick.saturating_sub(record.probation_start.unwrap_or(tick)) >= STABLE_TICKS {
            record.stable = true;
            record.consecutive_failures = 0;
            record.last_failure_tick = None;
        }
    }

    fn observe_spawned(&mut self, plugin_id: &PluginId, pid: Option<u32>) {
        let tick = self.tick;
        let record = self.records.entry(plugin_id.clone()).or_default();
        record.last_seen = Some(LastSeen::Alive);
        record.last_pid = pid;
        record.probation_start = pid.map(|_| tick);
        record.stable = false;
    }

    fn observe_death(&mut self, plugin_id: &PluginId) {
        let was_stable = self
            .records
            .get(plugin_id)
            .is_some_and(|record| record.stable);
        if was_stable {
            let record = self.records.entry(plugin_id.clone()).or_default();
            record.last_seen = Some(LastSeen::Dead);
            record.last_pid = None;
            record.probation_start = None;
            record.stable = false;
            return;
        }
        self.record_failure(plugin_id);
    }

    fn transition_for(&self, plugin_id: &PluginId, pid: Option<u32>) -> LivenessTransition {
        let last_seen = self
            .records
            .get(plugin_id)
            .and_then(|record| record.last_seen);
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
            DaemonExpectation::AutostartBlocked => {
                match snap.daemon_pid.filter(|pid| is_daemon_alive(Some(*pid))) {
                    Some(pid) => PluginRuntimeStatus::OnDemand { pid },
                    None => PluginRuntimeStatus::AutostartBlocked,
                }
            }
            DaemonExpectation::Supervised => {
                let Some(record) = self.records.get(&snap.plugin_id) else {
                    return PluginRuntimeStatus::Down {
                        consecutive_failures: 0,
                        suppressed: false,
                    };
                };
                if let (Some(LastSeen::Alive), Some(pid)) = (record.last_seen, record.last_pid) {
                    if record.stable {
                        PluginRuntimeStatus::Stable { pid }
                    } else {
                        PluginRuntimeStatus::Probation {
                            pid,
                            consecutive_failures: record.consecutive_failures,
                        }
                    }
                } else {
                    PluginRuntimeStatus::Down {
                        consecutive_failures: record.consecutive_failures,
                        suppressed: record.consecutive_failures >= MAX_CONSECUTIVE_FAILURES,
                    }
                }
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(id: &str) -> PluginId {
        PluginId::new(id.to_string())
    }

    fn snapshot(id: &str, daemon_pid: Option<u32>) -> DaemonSnapshot {
        DaemonSnapshot {
            plugin_id: pid(id),
            expectation: DaemonExpectation::Supervised,
            daemon_pid,
        }
    }

    fn alive_pid() -> u32 {
        std::process::id()
    }

    #[test]
    fn backoff_allows_retries_until_threshold_then_suppresses() {
        let mut state = SupervisorState::default();
        let p = pid("plugin-foo");

        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            assert!(state.can_retry(&p), "should allow retry below threshold");
            state.begin_tick();
            state.record_failure(&p);
        }
        assert!(
            !state.can_retry(&p),
            "should suppress after {} failures",
            MAX_CONSECUTIVE_FAILURES
        );
    }

    #[test]
    fn crash_loop_after_successful_spawn_suppresses() {
        let mut state = SupervisorState::default();
        let p = pid("plugin-foo");

        for cycle in 0..MAX_CONSECUTIVE_FAILURES {
            assert!(
                state.can_retry(&p),
                "cycle {cycle}: retry allowed pre-threshold"
            );
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
        let rec = state.records.get(&p).unwrap();
        assert!(rec.stable, "must be stable after STABLE_TICKS");
        assert_eq!(rec.consecutive_failures, 0, "stability resets failures");
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

    #[test]
    fn backoff_tracks_plugins_independently() {
        let mut state = SupervisorState::default();
        let foo = pid("plugin-foo");
        let bar = pid("plugin-bar");

        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            state.begin_tick();
            state.record_failure(&foo);
        }
        assert!(!state.can_retry(&foo), "foo suppressed");
        assert!(state.can_retry(&bar), "bar still retriable");
    }

    #[test]
    fn is_daemon_alive_none_returns_false() {
        assert!(!is_daemon_alive(None));
    }

    #[test]
    fn transition_classifies_alive_dead_first_seen_and_repeated_dead() {
        let mut state = SupervisorState::default();
        let foo = pid("plugin-foo");
        let bar = pid("plugin-bar");
        let baz = pid("plugin-baz");

        state.records.entry(foo.clone()).or_default().last_seen = Some(LastSeen::Alive);
        state.records.entry(baz.clone()).or_default().last_seen = Some(LastSeen::Dead);

        let live = alive_pid();
        let cases = [
            (
                "alive: pid present and previously alive",
                &foo,
                Some(live),
                LivenessTransition::Alive,
            ),
            (
                "alive: pid present and never seen",
                &bar,
                Some(live),
                LivenessTransition::Alive,
            ),
            (
                "alive_to_dead: pid none, previously alive",
                &foo,
                None,
                LivenessTransition::AliveToDead,
            ),
            (
                "alive_to_dead: pid none, never seen (treat unknown as previously alive)",
                &bar,
                None,
                LivenessTransition::AliveToDead,
            ),
            (
                "dead_stays_dead: pid none, previously dead",
                &baz,
                None,
                LivenessTransition::DeadStaysDead,
            ),
        ];
        for (label, plugin, daemon_pid, expected) in cases {
            assert_eq!(
                state.transition_for(plugin, daemon_pid),
                expected,
                "{label}"
            );
        }
    }

    #[test]
    fn transition_classifies_dead_to_alive_when_pid_recovers() {
        let mut state = SupervisorState::default();
        let recovered = pid("plugin-recovered");
        state
            .records
            .entry(recovered.clone())
            .or_default()
            .last_seen = Some(LastSeen::Dead);

        assert_eq!(
            state.transition_for(&recovered, Some(alive_pid())),
            LivenessTransition::DeadToAlive
        );
    }

    #[test]
    fn classify_snapshots_emits_fresh_recovery_when_dead_plugin_returns() {
        let mut state = SupervisorState::default();
        let recovered_id = pid("plugin-recovered");
        state
            .records
            .entry(recovered_id.clone())
            .or_default()
            .last_seen = Some(LastSeen::Dead);

        let snapshots = vec![snapshot("plugin-recovered", Some(alive_pid()))];
        let outcome = classify_snapshots(&snapshots, &state);

        assert_eq!(outcome.alive, vec![(recovered_id.clone(), alive_pid())]);
        assert_eq!(outcome.fresh_recoveries, vec![recovered_id]);
        assert!(outcome.fresh_deaths.is_empty());
        assert!(outcome.retriable_dead.is_empty());
    }

    #[test]
    fn classify_snapshots_separates_alive_fresh_dead_and_repeated_dead() {
        let mut state = SupervisorState::default();
        let alive_id = pid("plugin-alive");
        let fresh_dead_id = pid("plugin-fresh-dead");
        let stale_dead_id = pid("plugin-stale-dead");

        state
            .records
            .entry(stale_dead_id.clone())
            .or_default()
            .last_seen = Some(LastSeen::Dead);
        state
            .records
            .entry(fresh_dead_id.clone())
            .or_default()
            .last_seen = Some(LastSeen::Alive);

        let snapshots = vec![
            snapshot("plugin-alive", Some(alive_pid())),
            snapshot("plugin-fresh-dead", None),
            snapshot("plugin-stale-dead", None),
        ];

        let outcome = classify_snapshots(&snapshots, &state);
        assert_eq!(outcome.alive, vec![(alive_id, alive_pid())]);
        assert_eq!(outcome.fresh_deaths, vec![fresh_dead_id.clone()]);
        assert_eq!(outcome.retriable_dead, vec![fresh_dead_id, stale_dead_id]);
    }

    #[test]
    fn classify_snapshots_skips_dead_when_backoff_exhausted() {
        let mut state = SupervisorState::default();
        let exhausted = pid("plugin-exhausted");

        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            state.begin_tick();
            state.record_failure(&exhausted);
        }

        let snapshots = vec![snapshot("plugin-exhausted", None)];
        let outcome = classify_snapshots(&snapshots, &state);

        assert!(
            outcome.fresh_deaths.is_empty(),
            "no fresh death edge for plugin already known dead"
        );
        assert!(
            outcome.retriable_dead.is_empty(),
            "exhausted backoff withholds retry"
        );
    }

    #[test]
    fn classify_snapshots_emits_fresh_death_even_when_backoff_blocks_retry() {
        let mut state = SupervisorState::default();
        let dying = pid("plugin-dying");

        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            state.begin_tick();
            state.record_failure(&dying);
        }
        state.records.entry(dying.clone()).or_default().last_seen = Some(LastSeen::Alive);

        let snapshots = vec![snapshot("plugin-dying", None)];
        let outcome = classify_snapshots(&snapshots, &state);

        assert_eq!(
            outcome.fresh_deaths,
            vec![dying],
            "alive_to_dead transition fires regardless of backoff state"
        );
        assert!(
            outcome.retriable_dead.is_empty(),
            "exhausted backoff still blocks restart attempt"
        );
    }

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
        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            state.begin_tick();
            state.record_failure(&pid("plugin-suppressed"));
        }

        let cases = [
            (
                "plugin-no-daemon",
                DaemonExpectation::NotExpected,
                None,
                PluginRuntimeStatus::NotExpected,
            ),
            (
                "plugin-blocked",
                DaemonExpectation::AutostartBlocked,
                None,
                PluginRuntimeStatus::AutostartBlocked,
            ),
            (
                "plugin-on-demand",
                DaemonExpectation::AutostartBlocked,
                Some(alive_pid()),
                PluginRuntimeStatus::OnDemand { pid: alive_pid() },
            ),
            (
                "plugin-unseen",
                DaemonExpectation::Supervised,
                None,
                PluginRuntimeStatus::Down {
                    consecutive_failures: 0,
                    suppressed: false,
                },
            ),
            (
                "plugin-probation",
                DaemonExpectation::Supervised,
                Some(11),
                PluginRuntimeStatus::Probation {
                    pid: 11,
                    consecutive_failures: 0,
                },
            ),
            (
                "plugin-stable",
                DaemonExpectation::Supervised,
                Some(12),
                PluginRuntimeStatus::Stable { pid: 12 },
            ),
            (
                "plugin-suppressed",
                DaemonExpectation::Supervised,
                None,
                PluginRuntimeStatus::Down {
                    consecutive_failures: MAX_CONSECUTIVE_FAILURES,
                    suppressed: true,
                },
            ),
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

    #[test]
    fn prune_unknown_plugins_drops_state_for_removed_plugins() {
        let mut state = SupervisorState::default();
        let kept = pid("plugin-kept");
        let removed = pid("plugin-removed");

        state.record_failure(&kept);
        state.record_failure(&removed);

        state.note_known_plugins(&[snapshot("plugin-kept", Some(alive_pid()))]);
        state.prune_unknown_plugins();

        assert!(state.records.contains_key(&kept));
        assert!(!state.records.contains_key(&removed), "stale record pruned");
    }

    #[test]
    fn unchanged_supervisor_ticks_do_not_read_profile_state() {
        let _env_lock = crate::test_support::env_lock().blocking_lock();
        let root = tempfile::TempDir::new().unwrap();
        let _path = crate::paths::push_test_path_root(root.path());
        let plugin_manager = Arc::new(Mutex::new(PluginManager::new()));
        let (health_tx, _health_rx) = crate::plugins::daemon_health::channel();
        let publisher = HealthPublisher::new(
            health_tx,
            qol_conventions::DEFAULT_PORT,
            root.path().join("health.json"),
        );

        crate::plugins::config::reset_profile_config_read_count();
        let mut state = SupervisorState::default();
        supervise_once(&plugin_manager, &mut state, &publisher);
        supervise_once(&plugin_manager, &mut state, &publisher);

        assert_eq!(crate::plugins::config::profile_config_read_count(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn supervisor_reconciles_applied_profile_before_observing_daemon() {
        use std::os::unix::fs::PermissionsExt;

        let _env = crate::test_support::env_lock().blocking_lock();
        let root = tempfile::TempDir::new().unwrap();
        let _guard = crate::paths::push_test_path_root(root.path());
        let plugin_id = "plugin-supervisor-generation";
        let plugins_dir = crate::paths::plugins_dir().unwrap();
        let plugin_dir = plugins_dir.join(plugin_id);
        std::fs::create_dir_all(&plugin_dir).unwrap();
        let daemon = plugin_dir.join("daemon");
        std::fs::write(&daemon, "#!/bin/sh\nexec sleep 30\n").unwrap();
        std::fs::set_permissions(&daemon, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.toml"),
            format!(
                r#"
[plugin]
id = "{plugin_id}"
name = "{plugin_id}"
description = ""
version = "1.0.0"

[menu]
label = "{plugin_id}"
items = []

[daemon]
enabled = true
command = "daemon"
"#
            ),
        )
        .unwrap();
        let config_dir = crate::paths::shared_config_dir().unwrap();
        crate::plugins::registry::record_release_install(&config_dir, plugin_id, plugin_dir)
            .unwrap();

        let mut manager = PluginManager::new();
        manager.load_plugins().unwrap();
        manager.ensure_plugin_daemon_running(plugin_id).unwrap();
        let old_pid = manager
            .plugin_daemon_pid(&PluginId::new(plugin_id))
            .unwrap();

        {
            let _profile_guard = crate::plugins::config::profile_config_write_guard();
            let profile_configs = crate::paths::profile_plugin_configs_dir().unwrap();
            std::fs::create_dir_all(&profile_configs).unwrap();
            std::fs::write(profile_configs.join(format!("{plugin_id}.json")), "{}\n").unwrap();
        }

        let plugin_manager = Arc::new(Mutex::new(manager));
        let (health_tx, _health_rx) = crate::plugins::daemon_health::channel();
        let publisher = HealthPublisher::new(
            health_tx,
            qol_conventions::DEFAULT_PORT,
            crate::paths::runtime_dir().join("supervisor-generation-health.json"),
        );
        supervise_once(&plugin_manager, &mut SupervisorState::default(), &publisher);

        let manager = plugin_manager.lock().unwrap();
        let new_pid = manager
            .plugin_daemon_pid(&PluginId::new(plugin_id))
            .unwrap();
        assert_ne!(new_pid, old_pid);
        assert!(!crate::process_utils::is_pid_alive(old_pid as i32));
    }
}
