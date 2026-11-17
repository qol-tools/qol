use std::time::Duration;

use anyhow::Result;

use super::hook::Hooks;
use super::profile;
use crate::config::RemapConfig;
use crate::control::Host;
use crate::platform::engine::{self, remap};

const IDLE_POLL: Duration = Duration::from_secs(1);

pub(super) fn run(host: &Host) -> Result<()> {
    engine::serve(
        host,
        IDLE_POLL,
        profile::without_macos_profile,
        |resolved, _| Hooks::start(resolved, host.send_kill, host.plugin_id.clone()),
    )
}

pub(super) fn resolve(config: RemapConfig) -> remap::ResolvedConfig {
    remap::resolve(&profile::without_macos_profile(config))
}
