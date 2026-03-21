mod control;
#[cfg(feature = "dev")]
mod filter;
pub(crate) mod platform;
pub mod prod;
pub(crate) mod rate_limiter;
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
pub fn init_logger() -> CoreControlsHandle {
    let controls = load_core_controls_from_shared_config();
    let handle = std::sync::Arc::new(std::sync::RwLock::new(controls));
    let (inner, max_level) = filter::build(handle.clone());
    prod::init_with_inner(inner, max_level);
    handle
}

#[cfg(not(feature = "dev"))]
pub fn init_logger() {
    let inner =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).build();
    let max_level = inner.filter();
    prod::init_with_inner(Box::new(inner), max_level);
}

#[cfg(feature = "dev")]
fn load_core_controls_from_shared_config() -> std::collections::HashMap<String, LogControl> {
    crate::paths::shared_config_dir()
        .map(|dir| control::load_all_core_controls(&dir))
        .unwrap_or_default()
}
