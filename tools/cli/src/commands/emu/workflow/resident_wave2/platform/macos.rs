use anyhow::{bail, Context, Result};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

pub(crate) fn file_mode(metadata: &std::fs::Metadata) -> Result<u32> {
    Ok(metadata.permissions().mode() & 0o777)
}

pub(crate) fn rename_noreplace(from: &Path, to: &Path) -> Result<()> {
    let from_c = std::ffi::CString::new(from.as_os_str().as_bytes())
        .with_context(|| format!("source path contains a NUL byte: {}", from.display()))?;
    let to_c = std::ffi::CString::new(to.as_os_str().as_bytes())
        .with_context(|| format!("destination contains a NUL byte: {}", to.display()))?;
    let result = unsafe { libc::renamex_np(from_c.as_ptr(), to_c.as_ptr(), libc::RENAME_EXCL) };
    if result == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::EEXIST) {
        bail!(
            "destination already exists; refusing to replace {}",
            to.display()
        );
    }
    Err(error).with_context(|| format!("failed to publish to {}", to.display()))
}
