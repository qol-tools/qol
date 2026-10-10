mod binding;
pub(crate) mod platform;

pub(crate) use binding::{parse_combo, Binding, CaptureEvent, Combo, Phase, HEARTBEAT_INTERVAL};
pub(crate) use platform::{
    cancel_recording, install, release_held_keys, start_recording, KEEP_REGISTERED_ONE_SHOTS,
};

pub(crate) type OnFire = Box<dyn Fn(&CaptureEvent) + Send + Sync>;
pub(crate) type RebuildBindings = Box<dyn Fn() -> anyhow::Result<Vec<Binding>> + Send + Sync>;
