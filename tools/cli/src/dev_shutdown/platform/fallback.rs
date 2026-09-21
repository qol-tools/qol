use super::DevShutdownPlatform;
use crate::dev_shutdown::TrackedDaemonPid;
use std::process::Command;

pub(crate) struct Platform;

impl DevShutdownPlatform for Platform {
    fn snapshot_runtime_daemon_pids(&self) -> Vec<TrackedDaemonPid> {
        Vec::new()
    }

    fn group_is_owned(&self, _daemon: &TrackedDaemonPid) -> bool {
        false
    }

    fn configure_tray_child(&self, _command: &mut Command) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "tray child configuration is not supported on this platform",
        ))
    }
}
