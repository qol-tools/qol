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

    fn disk_space(&self, path: &Path) -> std::io::Result<DiskSpace> {
        let c_path = super::unix::c_path(path)?;
        let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statfs(c_path.as_ptr(), &mut stat) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let block = stat.f_bsize as u64;
        Ok(DiskSpace {
            available: stat.f_bavail * block,
            total: stat.f_blocks * block,
        })
    }
}
