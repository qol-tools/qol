use std::os::unix::ffi::OsStrExt;
use std::path::Path;

const UNRESERVED: &[u8] = b"-_.!~*'()";
const DATE_FORMAT: &str = "%Y-%m-%dT%H:%M:%S";

pub(super) fn encoded(path: &Path) -> String {
    let mut out = String::new();
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || byte == b'/' || UNRESERVED.contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

pub(super) fn contents(original: &Path) -> String {
    let date = chrono::Local::now().format(DATE_FORMAT);
    format!(
        "[Trash Info]\nPath={}\nDeletionDate={date}\n",
        encoded(original)
    )
}
