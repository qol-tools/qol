mod config;
mod execution;
mod handlers;
mod interpolation;
mod platform;

pub use config::{ActionConfig, TaskRunnerConfig};

use axum::Router;

pub fn router() -> Router {
    handlers::router(config::load_state())
}
