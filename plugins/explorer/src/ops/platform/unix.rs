use std::ffi::{CStr, CString};
use std::fs::{self, DirBuilder, File, Metadata, OpenOptions, Permissions};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{symlink, DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

use crate::ops::simple::plain;
use crate::ops::NameProblem;

const OWNER_ONLY_FILE: u32 = 0o600;
const OWNER_ONLY_DIR: u32 = 0o700;
const PLAIN_BITS: u32 = 0o777;
const MAX_NAME_BYTES: usize = 255;
pub(super) const STICKY: u32 = 0o1000;
const PASSWD_BUFFER: usize = 4096;

pub(crate) fn new_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(OWNER_ONLY_FILE)
        .open(path)
}

pub(crate) fn new_dir(path: &Path) -> io::Result<()> {
    DirBuilder::new().mode(OWNER_ONLY_DIR).create(path)
}

pub(crate) fn open_dir(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
}

pub(crate) fn keep_mode(file: &File, source: &Metadata) -> io::Result<()> {
    let mode = source.permissions().mode() & PLAIN_BITS;
    file.set_permissions(Permissions::from_mode(mode))
}

pub(super) fn euid() -> u32 {
    // SAFETY: geteuid has no preconditions and cannot fail.
    unsafe { libc::geteuid() }
}

pub(crate) fn free_bytes(path: &Path) -> io::Result<u64> {
    let path = c_path(path)?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut stat) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64))
}

pub(crate) fn volume_of(path: &Path) -> io::Result<u64> {
    let metadata = fs::metadata(path).or_else(|_| fs::symlink_metadata(path))?;
    Ok(metadata.dev())
}

pub(crate) fn removable(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|error| plain(&error))?;
    let parent = path
        .parent()
        .ok_or_else(|| "it is the top of the drive".to_string())?;
    let parent_metadata = fs::metadata(parent).map_err(|error| plain(&error))?;
    let me = euid();
    if me == 0 {
        return Ok(());
    }
    let parent_c = c_path(parent).map_err(|error| plain(&error))?;
    if unsafe { libc::access(parent_c.as_ptr(), libc::W_OK | libc::X_OK) } != 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EROFS) {
            return Err("is on a read-only drive".into());
        }
        return Err(owner_reason(metadata.uid(), me));
    }
    let sticky = parent_metadata.mode() & STICKY != 0;
    if sticky && metadata.uid() != me && parent_metadata.uid() != me {
        return Err(owner_reason(metadata.uid(), me));
    }
    Ok(())
}

fn owner_reason(owner: u32, me: u32) -> String {
    if owner == me {
        return "is in a folder that does not let you remove things".into();
    }
    match user_name(owner) {
        Some(name) => format!("belongs to the user {name}"),
        None => format!("belongs to another user (id {owner})"),
    }
}

fn user_name(uid: u32) -> Option<String> {
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buffer = vec![0 as libc::c_char; PASSWD_BUFFER];
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    let status = unsafe {
        libc::getpwuid_r(
            uid,
            &mut entry,
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut found,
        )
    };
    if status != 0 || found.is_null() {
        return None;
    }
    let name = unsafe { CStr::from_ptr(entry.pw_name) };
    Some(name.to_string_lossy().into_owned())
}

fn c_path(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes()).map_err(|_| io::ErrorKind::InvalidInput.into())
}

pub(crate) fn name_problem(name: &str) -> Option<NameProblem> {
    if name.trim().is_empty() {
        return Some(NameProblem::Empty);
    }
    if name.contains('/') {
        return Some(NameProblem::Forbidden(
            "/ separates folders, so it cannot be in a name".into(),
        ));
    }
    if name.contains('\0') {
        return Some(NameProblem::Forbidden(
            "a name cannot hold a null character".into(),
        ));
    }
    if name == "." || name == ".." {
        return Some(NameProblem::Forbidden("a name cannot be . or ..".into()));
    }
    if name.len() > MAX_NAME_BYTES {
        return Some(NameProblem::TooLong);
    }
    None
}

pub(crate) fn copy_link(from: &Path, to: &Path) -> io::Result<()> {
    symlink(fs::read_link(from)?, to)
}
