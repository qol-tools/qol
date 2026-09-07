use std::os::windows::ffi::OsStrExt;
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
        #[link(name = "kernel32")]
        extern "system" {
            fn GetDiskFreeSpaceExW(
                directory_name: *const u16,
                free_bytes_available_to_caller: *mut u64,
                total_number_of_bytes: *mut u64,
                total_number_of_free_bytes: *mut u64,
            ) -> i32;
        }

        let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        wide.push(0);
        let mut available = 0;
        let mut total = 0;
        let ok = unsafe {
            GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut available,
                &mut total,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(DiskSpace { available, total })
    }
}
