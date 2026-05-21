use std::sync::{Arc, OnceLock};

use qol_runtime::protocol::RuntimeEvent;

use super::server::shared::SharedState;

static PUBLISHER: OnceLock<Arc<SharedState>> = OnceLock::new();

pub(super) fn install(shared: Arc<SharedState>) {
    let _ = PUBLISHER.set(shared);
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
