use std::time::Duration;

use anyhow::Result;

use super::hook::Hooks;
use super::profile;
use crate::platform::engine::{self, config, remap};

const IDLE_POLL: Duration = Duration::from_secs(1);

pub(super) fn run() -> Result<()> {
    engine::serve(IDLE_POLL, load, Hooks::start)
}

pub(super) fn load() -> remap::ResolvedConfig {
    remap::resolve(&profile::without_macos_profile(config::load_config()))
}
