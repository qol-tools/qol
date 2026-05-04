//! Pluggable hotkey capture backends.
//!
//! Today qol-tray uses the `global_hotkey` crate (XGrabKey on Linux), which
//! loses silently when another X11 client holds a passive grab on the same
//! combo (csd-keyboard for `<Super>space`, IBus for input-source switching,
//! etc.). The Linux `evdev` backend reads `/dev/input/event*` and re-emits
//! via `/dev/uinput`, capturing keys before X11 ever sees them.
//!
//! Cross-platform surface kept here: `Binding`, `Combo`, `parse_combo`, and
//! `install`. Backend-internal pure-logic types (`BindingMatcher`,
//! `ModifierState`, the modifier keycode tables, the `Mod -> [u16; 2]`
//! mapping) live with the Linux backend at
//! `platform/linux/matcher.rs` since no other backend consumes them.

mod binding;
mod platform;

pub(crate) use binding::{parse_combo, Binding};
pub(crate) use platform::install;
