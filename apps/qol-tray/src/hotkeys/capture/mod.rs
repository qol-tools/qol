mod binding;
mod platform;

pub(crate) use binding::{parse_combo, Binding, CaptureEvent, Combo, Phase};
pub(crate) use platform::{cancel_recording, install, start_recording};

pub(crate) type OnFire = Box<dyn Fn(&CaptureEvent) + Send + Sync>;
pub(crate) type RebuildBindings = Box<dyn Fn() -> anyhow::Result<Vec<Binding>> + Send + Sync>;
