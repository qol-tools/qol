mod emitter;
mod environment;

pub use emitter::emit_build_identity;
pub use environment::{
    BuildIdentityEnvironment, BuildIdentityEnvironmentError, TRAY_BUILD_SCOPE_PATHS,
};
