mod binding;
pub(crate) mod platform;

pub(crate) use binding::{parse_combo, Binding, CaptureEvent, Combo, Phase};
#[cfg(target_os = "macos")]
pub(crate) use platform::release_tap;
pub(crate) use platform::{cancel_recording, install, release_held_keys, start_recording};

pub(crate) type OnFire = Box<dyn Fn(&CaptureEvent) + Send + Sync>;
pub(crate) type RebuildBindings = Box<dyn Fn() -> anyhow::Result<Vec<Binding>> + Send + Sync>;
