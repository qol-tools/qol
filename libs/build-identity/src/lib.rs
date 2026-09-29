mod build;
mod environment;

pub use build::emit_build_identity;
pub use environment::{
    BuildIdentityEnvironment, BuildIdentityEnvironmentError, PLUGIN_BUILD_SCOPE_PATHS,
    TRAY_BUILD_SCOPE_PATHS,
};
