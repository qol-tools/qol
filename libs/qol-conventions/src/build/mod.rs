mod artifact;
mod plugin_manifest;

pub use artifact::emit_build_identity;
pub use plugin_manifest::{emit_daemon_port, emit_plugin_id};
