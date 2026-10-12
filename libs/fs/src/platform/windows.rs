use std::fs::File;
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, RawHandle};
use std::path::Path;

use qol_platform::native::wide::wide_nul;

const FILE_ATTRIBUTE_READONLY: u32 = 0x1;
const INVALID_FILE_ATTRIBUTES: u32 = u32::MAX;
const FILE_SHARE_ALL: u32 = 0x1 | 0x2 | 0x4;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

#[repr(C)]
#[derive(Default)]
struct ByHandleFileInformation {
    _attributes_and_times: [u32; 7],
    volume_serial_number: u32,
    _size: [u32; 2],
    number_of_links: u32,
    file_index_high: u32,
    file_index_low: u32,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetFileAttributesW(file_name: *const u16) -> u32;
    fn SetFileAttributesW(file_name: *const u16, attributes: u32) -> i32;
    fn GetFileInformationByHandle(
        file: RawHandle,
        information: *mut ByHandleFileInformation,
    ) -> i32;
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
    let path = wide_nul(path);
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

pub fn file_identity(path: &Path, _: &std::fs::Metadata) -> Option<crate::FileIdentity> {
    let file = std::fs::OpenOptions::new()
        .access_mode(0)
        .share_mode(FILE_SHARE_ALL)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .ok()?;
    let mut information = ByHandleFileInformation::default();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut information) } == 0 {
        return None;
    }
    Some(crate::FileIdentity {
        device: u64::from(information.volume_serial_number),
        index: (u64::from(information.file_index_high) << 32)
            | u64::from(information.file_index_low),
        links: u64::from(information.number_of_links),
    })
}
