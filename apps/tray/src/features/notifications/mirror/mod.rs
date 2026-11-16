mod platform;

use super::takeover::Mechanism;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) enum ActivateOutcome {
    Pressed,
    Activated,
    Missing,
}

pub(super) fn apply(wanted: bool) -> Vec<Mechanism> {
    platform::apply(wanted)
}

pub(crate) fn activate(id: &str) -> anyhow::Result<ActivateOutcome> {
    platform::activate(id)
}

pub(crate) fn is_safe_banner_id(id: &str) -> bool {
    platform::is_safe_banner_id(id)
}
