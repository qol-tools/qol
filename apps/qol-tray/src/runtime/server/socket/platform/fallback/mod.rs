use std::path::Path;
use std::sync::Arc;

use crate::runtime::server::state_store::SharedState;

pub(crate) struct Listener;

pub(super) fn run_at(_shared: Arc<SharedState>, path: &Path) {
    log::warn!(
        "Runtime socket is unavailable on this platform: {}",
        path.display()
    );
}

pub(super) fn bind_at(_path: &Path) -> super::BindOutcome {
    super::BindOutcome::Unsupported
}

pub(super) fn run_listener(_shared: Arc<SharedState>, _listener: Listener) {}
