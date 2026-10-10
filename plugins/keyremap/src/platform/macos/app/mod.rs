use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;

use super::input::backends::virtual_hid;
use super::input::InputState;
use super::layout::{LayoutSnapshot, LayoutStore};
use super::tap::TapState;
use crate::platform::engine::{self, config, remap};

const LAYOUT_POLL: Duration = Duration::from_secs(1);

struct Services {
    state: Arc<TapState>,
    layouts: Arc<LayoutStore>,
}

impl engine::Remapper for Services {
    fn swap_config(&self, config: remap::ResolvedConfig) {
        self.state.swap_config(config);
    }

    fn idle(&self) {
        self.layouts.refresh();
    }

    fn stop(self) {}
}

pub(crate) fn run() -> Result<()> {
    let layouts = Arc::new(LayoutStore::new(
        LayoutSnapshot::read_current().unwrap_or_else(|error| {
            log::warn!("no keyboard layout to type characters with: {error}");
            LayoutSnapshot::empty()
        }),
    ));

    engine::serve(
        LAYOUT_POLL,
        || remap::resolve(&config::load_config()),
        |resolved| {
            let app_tracker = super::app_tracker::AppTracker::start();
            let state = Arc::new(TapState::new(
                resolved,
                app_tracker,
                Arc::new(InputState::default()),
            ));
            super::tap::start_tap(Arc::clone(&state));
            virtual_hid::start(Arc::clone(&state), Arc::clone(&layouts));
            super::secure_input::watch(Arc::clone(state.input()));
            Ok(Services { state, layouts })
        },
    )
}
