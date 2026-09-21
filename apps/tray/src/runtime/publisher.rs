use std::sync::{Arc, OnceLock};

use qol_runtime::protocol::RuntimeEvent;

use crate::daemon::EventBus;

use super::server::state_store::SharedState;

static PUBLISHER: OnceLock<Arc<SharedState>> = OnceLock::new();
static EVENTS: OnceLock<Arc<EventBus>> = OnceLock::new();

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
