use std::os::unix::fs::PermissionsExt;

use super::{HostOs, PluginStorePlatformOps};

pub(crate) struct Platform;

impl PluginStorePlatformOps for Platform {
    fn host_os(&self) -> HostOs {
        HostOs::from_manifest_token(std::env::consts::OS)
    }

    fn lockfile_max_age(&self) -> std::time::Duration {
        std::time::Duration::from_secs(30)
    }

    fn lock_owner_alive(&self, pid: u32) -> Option<bool> {
        Some(crate::process_utils::is_pid_alive(pid as i32))
    }

    fn executable_permissions(&self, metadata: std::fs::Metadata) -> Option<std::fs::Permissions> {
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o755);
        Some(permissions)
    }
}

#[cfg(feature = "dev")]
impl super::dev::PluginStoreDevPlatformOps for Platform {
    fn bind_public_runtime_socket(&self) -> bool {
        crate::runtime::RuntimeServer::bind_public_socket()
    }
}
