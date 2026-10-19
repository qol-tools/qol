use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use qol_runtime::protocol::{RuntimeEvent, RuntimeEventKind};

use crate::daemon::EventBus;

use super::server::state_store::SharedState;

type Replay = fn() -> RuntimeEvent;

static PUBLISHER: OnceLock<Arc<SharedState>> = OnceLock::new();
static EVENTS: OnceLock<Arc<EventBus>> = OnceLock::new();
static REPLAYS: OnceLock<Mutex<Vec<(RuntimeEventKind, Replay)>>> = OnceLock::new();

pub(super) fn install(shared: Arc<SharedState>) {
    let _ = PUBLISHER.set(shared);
}

pub(super) fn shared() -> Option<Arc<SharedState>> {
    PUBLISHER.get().cloned()
}

/// Installs the daemon event bus so outbound state changes (for example a
/// pushed plugin status) can reach the dashboard SSE stream. Same pattern as
/// [`install`]: the runtime server and the daemon are wired together once at
/// boot, before any plugin can push.
pub fn install_events(events: Arc<EventBus>) {
    let _ = EVENTS.set(events);
}

pub(crate) fn events() -> Option<Arc<EventBus>> {
    EVENTS.get().cloned()
}

pub fn publish(events: &[RuntimeEvent]) {
    let Some(shared) = PUBLISHER.get() else {
        log::warn!(
            "runtime publisher not installed; dropping {} event(s)",
            events.len()
        );
        return;
    };
    shared.publish(events);
}

pub fn is_subscribed(kind: RuntimeEventKind) -> bool {
    PUBLISHER
        .get()
        .is_some_and(|shared| shared.has_event_subscribers(kind))
}

pub fn replay_on_subscribe(kind: RuntimeEventKind, replay: Replay) {
    let mut replays = REPLAYS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    replays.retain(|(registered, _)| *registered != kind);
    replays.push((kind, replay));
}

pub(super) fn replays_for(events: &[RuntimeEventKind]) -> Vec<RuntimeEvent> {
    let Some(replays) = REPLAYS.get() else {
        return Vec::new();
    };
    let wanted: Vec<Replay> = replays
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .filter(|(kind, _)| events.contains(kind))
        .map(|(_, replay)| *replay)
        .collect();
    wanted.into_iter().map(|replay| replay()).collect()
}
