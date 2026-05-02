use crate::plugins::{PluginId, PluginManager};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::broadcast;

const SUPERVISION_INTERVAL: Duration = Duration::from_secs(5);
const MAX_CONSECUTIVE_FAILURES: u32 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
struct DaemonSnapshot {
    plugin_id: PluginId,
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
) {
    tokio::spawn(async move {
        run_supervision_loop(plugin_manager, shutdown_rx).await;
    });
}

async fn run_supervision_loop(
    plugin_manager: Arc<Mutex<PluginManager>>,
    mut shutdown_rx: broadcast::Receiver<()>,
) {
    let mut state = SupervisorState::default();
    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => {
                log::info!("Daemon supervisor shutting down");
                return;
            }
            _ = tokio::time::sleep(SUPERVISION_INTERVAL) => {
                supervise_once(&plugin_manager, &mut state);
            }
        }
    }
}

fn supervise_once(plugin_manager: &Arc<Mutex<PluginManager>>, state: &mut SupervisorState) {
    let snapshots = snapshot_daemons(plugin_manager);
    let outcome = classify_snapshots(&snapshots, state);
    state.note_known_plugins(&snapshots);

    for id in &outcome.alive {
        state.observe_alive(id);
    }

    let mut any_state_change =
        !outcome.fresh_deaths.is_empty() || !outcome.fresh_recoveries.is_empty();
    for plugin_id in outcome.retriable_dead {
        match restart_daemon(plugin_manager, &plugin_id) {
            Ok(()) => {
                state.observe_alive(&plugin_id);
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

    if any_state_change {
        crate::hotkeys::trigger_reload();
    }
}

#[derive(Default)]
struct TickOutcome {
    alive: Vec<PluginId>,
    fresh_deaths: Vec<PluginId>,
    fresh_recoveries: Vec<PluginId>,
    retriable_dead: Vec<PluginId>,
}

fn classify_snapshots(snapshots: &[DaemonSnapshot], state: &SupervisorState) -> TickOutcome {
    let mut outcome = TickOutcome::default();
    for snap in snapshots {
        match state.transition_for(&snap.plugin_id, snap.daemon_pid) {
            LivenessTransition::Alive => outcome.alive.push(snap.plugin_id.clone()),
            LivenessTransition::DeadToAlive => {
                outcome.alive.push(snap.plugin_id.clone());
                outcome.fresh_recoveries.push(snap.plugin_id.clone());
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

fn snapshot_daemons(plugin_manager: &Arc<Mutex<PluginManager>>) -> Vec<DaemonSnapshot> {
    let Ok(manager) = plugin_manager.lock() else {
        log::error!("Daemon supervisor: plugin manager lock poisoned");
        return Vec::new();
    };
    manager
        .plugins()
        .filter(|p| p.manifest.daemon.as_ref().is_some_and(|d| d.enabled))
        .map(|p| DaemonSnapshot {
            plugin_id: p.id.clone(),
            daemon_pid: p.daemon_pid(),
        })
        .collect()
}

fn restart_daemon(
    plugin_manager: &Arc<Mutex<PluginManager>>,
    plugin_id: &PluginId,
) -> anyhow::Result<()> {
    let mut manager = plugin_manager
        .lock()
        .map_err(|_| anyhow::anyhow!("plugin manager lock poisoned"))?;
    manager.ensure_plugin_daemon_running(plugin_id.as_str())
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

#[derive(Default)]
struct SupervisorState {
    counts: HashMap<PluginId, u32>,
    last_seen: HashMap<PluginId, LastSeen>,
    known: HashSet<PluginId>,
}

impl SupervisorState {
    fn can_retry(&self, plugin_id: &PluginId) -> bool {
        self.counts.get(plugin_id).copied().unwrap_or(0) < MAX_CONSECUTIVE_FAILURES
    }

    fn record_failure(&mut self, plugin_id: &PluginId) {
        let entry = self.counts.entry(plugin_id.clone()).or_insert(0);
        *entry += 1;
        self.last_seen.insert(plugin_id.clone(), LastSeen::Dead);
        if *entry == MAX_CONSECUTIVE_FAILURES {
            log::error!(
                "Daemon supervisor: plugin {} hit {} consecutive failures, suppressing",
                plugin_id,
                MAX_CONSECUTIVE_FAILURES
            );
        }
    }

    fn observe_alive(&mut self, plugin_id: &PluginId) {
        self.counts.remove(plugin_id);
        self.last_seen.insert(plugin_id.clone(), LastSeen::Alive);
    }

    fn transition_for(&self, plugin_id: &PluginId, pid: Option<u32>) -> LivenessTransition {
        if is_daemon_alive(pid) {
            return match self.last_seen.get(plugin_id) {
                Some(LastSeen::Dead) => LivenessTransition::DeadToAlive,
                Some(LastSeen::Alive) | None => LivenessTransition::Alive,
            };
        }
        match self.last_seen.get(plugin_id) {
            Some(LastSeen::Dead) => LivenessTransition::DeadStaysDead,
            Some(LastSeen::Alive) | None => LivenessTransition::AliveToDead,
        }
    }

    fn note_known_plugins(&mut self, snapshots: &[DaemonSnapshot]) {
        self.known = snapshots.iter().map(|s| s.plugin_id.clone()).collect();
    }

    fn prune_unknown_plugins(&mut self) {
        let known = std::mem::take(&mut self.known);
        self.counts.retain(|id, _| known.contains(id));
        self.last_seen.retain(|id, _| known.contains(id));
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
            state.record_failure(&p);
        }
        assert!(
            !state.can_retry(&p),
            "should suppress after {} failures",
            MAX_CONSECUTIVE_FAILURES
        );
    }

    #[test]
    fn observe_alive_resets_count() {
        let mut state = SupervisorState::default();
        let p = pid("plugin-foo");

        state.record_failure(&p);
        state.record_failure(&p);
        state.observe_alive(&p);

        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            assert!(state.can_retry(&p), "observe_alive reset failure count");
            state.record_failure(&p);
        }
        assert!(!state.can_retry(&p), "threshold reached again after reset");
    }

    #[test]
    fn backoff_tracks_plugins_independently() {
        let mut state = SupervisorState::default();
        let foo = pid("plugin-foo");
        let bar = pid("plugin-bar");

        for _ in 0..MAX_CONSECUTIVE_FAILURES {
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

        state.last_seen.insert(foo.clone(), LastSeen::Alive);
        state.last_seen.insert(baz.clone(), LastSeen::Dead);

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
        state.last_seen.insert(recovered.clone(), LastSeen::Dead);

        assert_eq!(
            state.transition_for(&recovered, Some(alive_pid())),
            LivenessTransition::DeadToAlive
        );
    }

    #[test]
    fn classify_snapshots_emits_fresh_recovery_when_dead_plugin_returns() {
        let mut state = SupervisorState::default();
        let recovered_id = pid("plugin-recovered");
        state.last_seen.insert(recovered_id.clone(), LastSeen::Dead);

        let snapshots = vec![snapshot("plugin-recovered", Some(alive_pid()))];
        let outcome = classify_snapshots(&snapshots, &state);

        assert_eq!(outcome.alive, vec![recovered_id.clone()]);
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
            .last_seen
            .insert(stale_dead_id.clone(), LastSeen::Dead);
        state
            .last_seen
            .insert(fresh_dead_id.clone(), LastSeen::Alive);

        let snapshots = vec![
            snapshot("plugin-alive", Some(alive_pid())),
            snapshot("plugin-fresh-dead", None),
            snapshot("plugin-stale-dead", None),
        ];

        let outcome = classify_snapshots(&snapshots, &state);
        assert_eq!(outcome.alive, vec![alive_id]);
        assert_eq!(outcome.fresh_deaths, vec![fresh_dead_id.clone()]);
        assert_eq!(outcome.retriable_dead, vec![fresh_dead_id, stale_dead_id]);
    }

    #[test]
    fn classify_snapshots_skips_dead_when_backoff_exhausted() {
        let mut state = SupervisorState::default();
        let exhausted = pid("plugin-exhausted");

        state.last_seen.insert(exhausted.clone(), LastSeen::Dead);
        for _ in 0..MAX_CONSECUTIVE_FAILURES {
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

        state.last_seen.insert(dying.clone(), LastSeen::Alive);
        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            state.record_failure(&dying);
        }
        state.last_seen.insert(dying.clone(), LastSeen::Alive);

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
    fn prune_unknown_plugins_drops_state_for_removed_plugins() {
        let mut state = SupervisorState::default();
        let kept = pid("plugin-kept");
        let removed = pid("plugin-removed");

        state.record_failure(&kept);
        state.record_failure(&removed);
        state.last_seen.insert(removed.clone(), LastSeen::Dead);

        state.note_known_plugins(&[snapshot("plugin-kept", Some(alive_pid()))]);
        state.prune_unknown_plugins();

        assert!(state.counts.contains_key(&kept));
        assert!(!state.counts.contains_key(&removed), "stale count pruned");
        assert!(
            !state.last_seen.contains_key(&removed),
            "stale last_seen pruned"
        );
    }
}
