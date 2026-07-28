use super::{HostOs, PluginStorePlatformOps};

pub(crate) struct Platform;

impl PluginStorePlatformOps for Platform {
    fn host_os(&self) -> HostOs {
        HostOs::from_manifest_token("windows")
    }

    fn lockfile_max_age(&self) -> std::time::Duration {
        std::time::Duration::from_secs(300)
    }

    fn lock_owner_alive(&self, _pid: u32) -> Option<bool> {
        None
    }

    fn executable_permissions(&self, _metadata: std::fs::Metadata) -> Option<std::fs::Permissions> {
        None
    }
}

#[cfg(feature = "dev")]
impl super::dev::PluginStoreDevPlatformOps for Platform {
    fn bind_public_runtime_socket(&self) -> bool {
        true
    }
}
