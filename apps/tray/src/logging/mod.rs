pub mod console_guard;
mod control;
mod error_capture;
pub mod file_logger;
#[cfg(feature = "dev")]
mod filter;
pub(crate) mod rate_limiter;
mod redaction;
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
pub use filter::{is_valid_core_section, CoreControlsHandle, CORE_LOG_SECTION_IDS};

pub fn log_build_identity() {
    let Some(identity) = qol_conventions::artifact::current() else {
        log::error!("[artifact-identity] running build identity is unavailable");
        return;
    };
    match serde_json::to_string(identity) {
        Ok(identity) => log::info!("[artifact-identity] {identity}"),
        Err(error) => log::error!("[artifact-identity] serialization failed: {error}"),
    }
}

fn rust_log_targets() -> tracing_subscriber::filter::Targets {
    use tracing_subscriber::filter::{LevelFilter, Targets};
    std::env::var("RUST_LOG")
        .ok()
        .and_then(|s| s.parse::<Targets>().ok())
        .unwrap_or_else(|| Targets::new().with_default(LevelFilter::INFO))
}

#[cfg(feature = "dev")]
pub fn init_logger() -> CoreControlsHandle {
    use tracing_subscriber::prelude::*;

    let controls = load_core_controls_from_shared_config();
    let handle = std::sync::Arc::new(std::sync::RwLock::new(controls));

    #[cfg(any(test, debug_assertions))]
    if let Some(dir) = crate::paths::test_log_dir() {
        qol_log::set_dir_override(dir);
    }

    file_logger::init();

    let dev_controls = handle.clone();
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_filter(rust_log_targets())
        .with_filter(filter::CoreLogFilter::new(dev_controls));

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

    #[cfg(any(test, debug_assertions))]
    if let Some(dir) = crate::paths::test_log_dir() {
        qol_log::set_dir_override(dir);
    }

    file_logger::init();

    let stderr_layer = tracing_subscriber::fmt::layer().with_filter(rust_log_targets());

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
