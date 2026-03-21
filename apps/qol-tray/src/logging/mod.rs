mod control;
mod error_capture;
pub mod file_logger;
#[cfg(feature = "dev")]
mod filter;
pub(crate) mod platform;
pub(crate) mod rate_limiter;
pub(crate) mod relay;

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
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::EnvFilter;

    let controls = load_core_controls_from_shared_config();
    let handle = std::sync::Arc::new(std::sync::RwLock::new(controls));

    file_logger::init();

    let dev_controls = handle.clone();
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_filter(EnvFilter::from_default_env().add_directive("info".parse().unwrap()))
        .with_filter(tracing_subscriber::filter::filter_fn(move |metadata| {
            !filter::is_suppressed(&dev_controls, metadata.target(), "")
        }));

    tracing_subscriber::registry()
        .with(stderr_layer)
        .with(error_capture::ErrorCaptureLayer.with_filter(error_capture::ErrorOnlyFilter))
        .init();

    tracing_log::LogTracer::init().ok();

    handle
}

#[cfg(not(feature = "dev"))]
pub fn init_logger() {
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::EnvFilter;

    file_logger::init();

    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_filter(EnvFilter::from_default_env().add_directive("info".parse().unwrap()));

    tracing_subscriber::registry()
        .with(stderr_layer)
        .with(error_capture::ErrorCaptureLayer.with_filter(error_capture::ErrorOnlyFilter))
        .init();

    tracing_log::LogTracer::init().ok();
}

#[cfg(feature = "dev")]
fn load_core_controls_from_shared_config() -> std::collections::HashMap<String, LogControl> {
    crate::paths::shared_config_dir()
        .map(|dir| control::load_all_core_controls(&dir))
        .unwrap_or_default()
}
