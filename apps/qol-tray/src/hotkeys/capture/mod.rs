mod binding;
mod platform;

pub(crate) use binding::{parse_combo, Binding, CaptureEvent, Phase};
pub(crate) use platform::install;

pub(crate) type OnFire = Box<dyn Fn(&CaptureEvent) + Send + Sync>;
pub(crate) type RebuildBindings = Box<dyn Fn() -> anyhow::Result<Vec<Binding>> + Send + Sync>;
