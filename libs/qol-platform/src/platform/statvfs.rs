use std::path::Path;

use crate::DiskSpace;

pub(super) fn disk_space(path: &Path) -> std::io::Result<DiskSpace> {
    let c_path = super::unix::c_path(path)?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let block = stat.f_frsize as u64;
    Ok(DiskSpace {
        available: stat.f_bavail as u64 * block,
        total: stat.f_blocks as u64 * block,
    })
}
