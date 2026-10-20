use std::ffi::OsString;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};

use crate::places::Mount;

const DRIVE_PARENTS: [&str; 3] = ["/media", "/mnt", "/run/media"];
const DEVICE_PREFIX: &str = "/dev/";
const FIELDS_END: &str = "-";
const MOUNT_POINT_FIELD: usize = 4;

pub(super) fn parse(contents: &str) -> Vec<Mount> {
    let mut mounts: Vec<Mount> = Vec::new();
    for mount in contents.lines().filter_map(parse_line) {
        mounts.retain(|known| known.path != mount.path);
        mounts.push(mount);
    }
    mounts
}

fn parse_line(line: &str) -> Option<Mount> {
    let fields: Vec<&str> = line.split(' ').collect();
    let mount_point = fields.get(MOUNT_POINT_FIELD)?;
    let end = fields.iter().position(|field| *field == FIELDS_END)?;
    let source = fields.get(end + 2)?;
    if !source.starts_with(DEVICE_PREFIX) {
        return None;
    }
    let path = PathBuf::from(OsString::from_vec(unescape(mount_point)));
    if !holds_drives(&path) {
        return None;
    }
    Some(Mount {
        label: label(&path),
        path,
    })
}

fn holds_drives(path: &Path) -> bool {
    path == Path::new("/") || DRIVE_PARENTS.iter().any(|parent| path.starts_with(parent))
}

fn label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn unescape(field: &str) -> Vec<u8> {
    let bytes = field.as_bytes();
    let mut unescaped = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escaped = (bytes[index] == b'\\')
            .then(|| octal(bytes.get(index + 1..index + 4)))
            .flatten();
        match escaped {
            Some(byte) => {
                unescaped.push(byte);
                index += 4;
            }
            None => {
                unescaped.push(bytes[index]);
                index += 1;
            }
        }
    }
    unescaped
}

fn octal(digits: Option<&[u8]>) -> Option<u8> {
    let digits = digits?;
    if !digits.iter().all(|digit| (b'0'..=b'7').contains(digit)) {
        return None;
    }
    u8::from_str_radix(std::str::from_utf8(digits).ok()?, 8).ok()
}
