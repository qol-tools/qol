use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;

use super::input::backends::hid_helper;
use super::input::{selection, InputState};
use super::karabiner::backend::{Backend, Installed};
use super::karabiner::hotkeys;
use super::karabiner::store::Store;
use super::layout::{LayoutSnapshot, LayoutStore};
use super::tap::TapState;
use crate::platform::engine::config::KeyBackendChoice;
use crate::platform::engine::{self, config, remap};

const LAYOUT_POLL: Duration = Duration::from_secs(1);

struct Services {
    state: Arc<TapState>,
    layouts: Arc<LayoutStore>,
    karabiner: Option<Arc<Mutex<Backend<Installed>>>>,
}

impl Services {
    fn choose(&self, choice: KeyBackendChoice) {
        self.state
            .input()
            .allow_helper(selection::helper_allowed(choice));
        let Some(karabiner) = &self.karabiner else {
            return;
        };
        let config = self.state.config();
        let layout = self.layouts.get();
        if let Ok(mut backend) = karabiner.lock() {
            backend.apply(choice, &config, &layout, self.state.input());
        }
    }
}

impl engine::Remapper for Services {
    fn swap_config(&self, config: remap::ResolvedConfig) {
        self.state.swap_config(config);
        self.choose(config::load_config().key_backend);
    }

    fn idle(&self) {
        self.layouts.refresh();
        let Some(karabiner) = &self.karabiner else {
            return;
        };
        let config = self.state.config();
        let layout = self.layouts.get();
        let lease = karabiner.lock().ok().and_then(|mut backend| {
            backend.idle(&config, &layout, self.state.input());
            backend.lease(layout.physical)
        });
        self.state
            .input()
            .set_hotkeys_relayed(lease.as_ref().is_some_and(|(_, lease)| lease.lease));
        if let Some((relay, lease)) = lease {
            hotkeys::send_lease(&relay, &lease);
        }
    }

    fn stop(self) {
        if let Some(karabiner) = &self.karabiner {
            if let Ok(mut backend) = karabiner.lock() {
                backend.stop();
                self.state.input().set_hotkeys_relayed(false);
                if let Some((relay, lease)) = backend.lease(self.layouts.get().physical) {
                    hotkeys::send_lease(&relay, &lease);
                }
            }
        }
    }
}

fn follow_hotkeys(
    karabiner: Arc<Mutex<Backend<Installed>>>,
    state: Arc<TapState>,
    layouts: Arc<LayoutStore>,
) {
    hotkeys::follow(move |hotkeys| {
        let config = state.config();
        let layout = layouts.get();
        if let Ok(mut backend) = karabiner.lock() {
            backend.follow_hotkeys(hotkeys, &config, &layout);
        }
    });
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
            let choice = config::load_config().key_backend;
            let input = Arc::new(InputState::default());
            input.allow_helper(selection::helper_allowed(choice));
            let app_tracker = super::app_tracker::AppTracker::start();
            let state = Arc::new(TapState::new(resolved, app_tracker, Arc::clone(&input)));
            let karabiner = match Store::locate() {
                Ok(store) => Some(Arc::new(Mutex::new(Backend::new(Installed(store))))),
                Err(error) => {
                    log::warn!("Karabiner-Elements backend unavailable: {error:#}");
                    None
                }
            };
            let services = Services {
                state: Arc::clone(&state),
                layouts: Arc::clone(&layouts),
                karabiner,
            };
            services.choose(choice);
            if let Some(karabiner) = &services.karabiner {
                follow_hotkeys(
                    Arc::clone(karabiner),
                    Arc::clone(&state),
                    Arc::clone(&layouts),
                );
            }
            super::tap::start_tap(Arc::clone(&state));
            hid_helper::start(Arc::clone(&state), Arc::clone(&layouts));
            super::secure_input::watch(Arc::clone(state.input()));
            Ok(services)
        },
    )
}
