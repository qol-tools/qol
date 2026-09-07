use std::path::Path;

use crate::{DiskSpace, LinuxDisplayBackend, PlatformCapabilities};

use super::{PlatformApi, FULL_CAPABILITIES};

pub(crate) struct Platform;

impl PlatformApi for Platform {
    fn linux_display_backend(&self) -> LinuxDisplayBackend {
        LinuxDisplayBackend::Unknown
    }

    fn current_capabilities(&self) -> PlatformCapabilities {
        FULL_CAPABILITIES
    }

    fn disk_space(&self, _path: &Path) -> std::io::Result<DiskSpace> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "disk_space is not supported on this platform",
        ))
    }
}
