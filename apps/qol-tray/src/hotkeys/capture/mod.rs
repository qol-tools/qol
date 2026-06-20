mod binding;
mod platform;

pub(crate) use binding::{parse_combo, Binding};
pub(crate) use platform::install;

pub(crate) type OnFire = Box<dyn Fn(&Binding) + Send + Sync>;
pub(crate) type RebuildBindings = Box<dyn Fn() -> Vec<Binding> + Send + Sync>;
