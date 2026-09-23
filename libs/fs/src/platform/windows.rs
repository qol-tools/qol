use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;

#[link(name = "kernel32")]
extern "system" {
    fn GetFileAttributesW(file_name: *const u16) -> u32;
    fn SetFileAttributesW(file_name: *const u16, attributes: u32) -> i32;
}

pub(crate) fn set_private(_: &File) -> io::Result<()> {
    Ok(())
}

pub(crate) fn set_private_dir(_: &Path) -> io::Result<()> {
    Ok(())
}

pub(crate) fn sync_parent(_: &Path) -> io::Result<()> {
    Ok(())
}

pub(crate) fn prepare_file_removal(path: &Path) -> io::Result<()> {
    let path = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let attributes = unsafe { GetFileAttributesW(path.as_ptr()) };
    if attributes == INVALID_FILE_ATTRIBUTES {
        return Err(io::Error::last_os_error());
    }
    if attributes & FILE_ATTRIBUTE_READONLY == 0 {
        return Ok(());
    }
    if unsafe { SetFileAttributesW(path.as_ptr(), attributes & !FILE_ATTRIBUTE_READONLY) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(crate) fn set_mode(_: &std::fs::File, _: u32) -> io::Result<()> {
    Ok(())
}
