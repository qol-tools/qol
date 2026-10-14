mod platform;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MirrorAccess {
    Unsupported,
    Readable,
    Unreadable(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) enum ActivateOutcome {
    Pressed,
    Activated,
    Missing,
}

pub(super) fn sync(enabled: bool) {
    platform::sync(enabled);
}

pub(super) fn access() -> MirrorAccess {
    platform::access()
}

pub(crate) fn activate(id: &str) -> anyhow::Result<ActivateOutcome> {
    platform::activate(id)
}

pub(crate) fn is_safe_banner_id(id: &str) -> bool {
    platform::is_safe_banner_id(id)
}
