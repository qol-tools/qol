use std::ffi::CStr;
use std::fs::Metadata;
use std::os::unix::fs::MetadataExt;

use crate::props::Fact;

const NAME_BUFFER: usize = 4096;

pub(crate) fn facts(metadata: &Metadata) -> Vec<Fact> {
    vec![
        Fact::new("Permissions", permissions(metadata.mode())),
        Fact::new("Owner", owner(metadata.uid(), metadata.gid())),
    ]
}

pub(crate) fn volume(metadata: &Metadata) -> Option<u64> {
    Some(metadata.dev())
}

pub(super) fn permissions(mode: u32) -> String {
    let bit = |mask: u32, on: char| if mode & mask != 0 { on } else { '-' };
    let execute = |run: u32, special: u32, lower: char, upper: char| match (
        mode & run != 0,
        mode & special != 0,
    ) {
        (true, true) => lower,
        (false, true) => upper,
        (true, false) => 'x',
        (false, false) => '-',
    };
    format!(
        "{}{}{}{}{}{}{}{}{} ({:04o})",
        bit(0o400, 'r'),
        bit(0o200, 'w'),
        execute(0o100, 0o4000, 's', 'S'),
        bit(0o040, 'r'),
        bit(0o020, 'w'),
        execute(0o010, 0o2000, 's', 'S'),
        bit(0o004, 'r'),
        bit(0o002, 'w'),
        execute(0o001, 0o1000, 't', 'T'),
        mode & 0o7777,
    )
}

pub(super) fn owner(uid: u32, gid: u32) -> String {
    let user = user_name(uid).unwrap_or_else(|| uid.to_string());
    let group = group_name(gid).unwrap_or_else(|| gid.to_string());
    format!("{user}:{group}")
}

fn user_name(uid: u32) -> Option<String> {
    let mut buffer = vec![0 as libc::c_char; NAME_BUFFER];
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
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

fn group_name(gid: u32) -> Option<String> {
    let mut buffer = vec![0 as libc::c_char; NAME_BUFFER];
    let mut entry: libc::group = unsafe { std::mem::zeroed() };
    let mut found: *mut libc::group = std::ptr::null_mut();
    let status = unsafe {
        libc::getgrgid_r(
            gid,
            &mut entry,
            buffer.as_mut_ptr(),
            buffer.len(),
            &mut found,
        )
    };
    if status != 0 || found.is_null() {
        return None;
    }
    let name = unsafe { CStr::from_ptr(entry.gr_name) };
    Some(name.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::{owner, permissions};

    #[test]
    fn modes_read_as_rwx_with_special_bits_and_octal() {
        let cases = [
            (0o100644, "rw-r--r-- (0644)"),
            (0o040755, "rwxr-xr-x (0755)"),
            (0o100000, "--------- (0000)"),
            (0o104755, "rwsr-xr-x (4755)"),
            (0o104644, "rwSr--r-- (4644)"),
            (0o102755, "rwxr-sr-x (2755)"),
            (0o041777, "rwxrwxrwt (1777)"),
            (0o041776, "rwxrwxrwT (1776)"),
        ];
        for (mode, expected) in cases {
            assert_eq!(permissions(mode), expected, "{mode:o}");
        }
    }

    #[test]
    fn an_owner_with_no_account_reads_as_its_numbers() {
        assert_eq!(owner(4_000_000_001, 4_000_000_002), "4000000001:4000000002");
    }

    #[test]
    fn root_is_named() {
        assert!(owner(0, 0).starts_with("root:"));
    }
}
