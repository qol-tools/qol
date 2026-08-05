mod platform;

use std::path::Path;
use std::sync::Arc;

use super::state_store::SharedState;

pub(crate) use platform::{BindOutcome, Listener};

pub(crate) fn run_at(shared: Arc<SharedState>, path: &Path) {
    platform::run_at(shared, path);
}

pub(crate) fn bind_at(path: &Path) -> BindOutcome {
    platform::bind_at(path)
}

pub(crate) fn run_listener(shared: Arc<SharedState>, listener: Listener) {
    platform::run_listener(shared, listener);
}
