mod platform;

pub(in crate::plugins) use platform::DaemonListener;
pub(super) use platform::{apply_to_command, bind_for_plugin, refresh_for_respawn};
