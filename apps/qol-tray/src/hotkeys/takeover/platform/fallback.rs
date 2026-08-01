use super::super::{Compositor, HostFailure};

const REASON: &str = "desktop keybinding takeover is only implemented for dconf desktops";

fn unsupported(command: &str) -> HostFailure {
    HostFailure {
        command: command.to_string(),
        detail: REASON.to_string(),
        tool_missing: true,
    }
}

pub(crate) fn available() -> bool {
    false
}

pub(crate) fn dump(_root: &str) -> Result<String, HostFailure> {
    Err(unsupported("dconf"))
}

pub(crate) fn read(_full_key: &str) -> Result<String, HostFailure> {
    Err(unsupported("dconf"))
}

pub(crate) fn write(_full_key: &str, _value: &str) -> Result<(), HostFailure> {
    Err(unsupported("dconf"))
}

pub(crate) fn compositor() -> Option<Compositor> {
    None
}
