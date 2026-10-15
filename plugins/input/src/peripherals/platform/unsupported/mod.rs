use anyhow::{bail, Result};

use super::super::{Baseline, Keyboard, Level, Mouse, Setting, Switch, Touchpad, Values};

const UNSUPPORTED: &str = "input settings work on Linux, macOS and Windows only";

pub(crate) fn backend() -> Result<&'static str> {
    bail!(UNSUPPORTED)
}

pub(crate) fn mouse() -> Result<Mouse> {
    bail!(UNSUPPORTED)
}

pub(crate) fn touchpad() -> Result<Touchpad> {
    bail!(UNSUPPORTED)
}

pub(crate) fn keyboard() -> Result<Keyboard> {
    bail!(UNSUPPORTED)
}

pub(crate) fn set_switch(_switch: Switch, _on: bool) -> Result<()> {
    bail!(UNSUPPORTED)
}

pub(crate) fn set_level(_level: Level, _value: u32) -> Result<()> {
    bail!(UNSUPPORTED)
}

pub(crate) fn capture(_setting: Setting) -> Result<Baseline> {
    bail!(UNSUPPORTED)
}

pub(crate) fn restore(_setting: Setting, _baseline: Baseline) -> Result<()> {
    bail!(UNSUPPORTED)
}

pub(crate) fn values(level: Level) -> Values {
    Values::Steps(level.bounds())
}

pub(crate) fn owner(setting: Setting) -> Setting {
    setting
}
