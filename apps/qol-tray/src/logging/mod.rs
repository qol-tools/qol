mod control;
#[cfg(feature = "dev")]
mod filter;
pub(crate) mod platform;
pub(crate) mod relay;
pub(crate) mod writer;

pub use control::LogControl;

#[cfg(feature = "dev")]
pub use control::{
    load_all_plugin_controls, load_plugin_control, load_plugin_control_from_shared_config,
    save_all_plugin_controls, upsert_plugin_control,
};

#[cfg(feature = "dev")]
pub use control::upsert_core_control;

#[cfg(feature = "dev")]
pub use filter::CoreControlsHandle;

#[cfg(feature = "dev")]
pub fn init_dev_logger() -> CoreControlsHandle {
    let controls = load_core_controls_from_shared_config();
    let handle = std::sync::Arc::new(std::sync::RwLock::new(controls));
    filter::init(handle.clone());
    handle
}

#[cfg(feature = "dev")]
fn load_core_controls_from_shared_config() -> std::collections::HashMap<String, LogControl> {
    crate::paths::shared_config_dir()
        .map(|dir| control::load_all_core_controls(&dir))
        .unwrap_or_default()
}
