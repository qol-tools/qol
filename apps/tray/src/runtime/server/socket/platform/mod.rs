use std::path::Path;
use std::sync::Arc;

use crate::runtime::server::state_store::SharedState;

#[cfg(not(unix))]
mod fallback;
#[cfg(unix)]
mod unix;

#[cfg(not(unix))]
use fallback as active;
#[cfg(unix)]
use unix as active;

pub(crate) use active::Listener;

pub(crate) enum BindOutcome {
    #[cfg_attr(not(unix), allow(dead_code))]
    Bound(Listener),
    #[cfg_attr(not(unix), allow(dead_code))]
    Failed,
    #[cfg_attr(unix, allow(dead_code))]
    Unsupported,
}

pub(super) fn run_at(shared: Arc<SharedState>, path: &Path) {
    active::run_at(shared, path);
}

pub(super) fn bind_at(path: &Path) -> BindOutcome {
    active::bind_at(path)
}

pub(super) fn run_listener(shared: Arc<SharedState>, listener: Listener) {
    active::run_listener(shared, listener);
}
