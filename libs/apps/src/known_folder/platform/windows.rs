use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use windows_sys::core::{GUID, PWSTR};
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::UI::Shell::{
    FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music, FOLDERID_Pictures,
    FOLDERID_Videos, SHGetKnownFolderPath,
};

pub(crate) fn known_folder(id: &GUID) -> Option<PathBuf> {
    let mut raw: PWSTR = std::ptr::null_mut();
    // SAFETY: `raw` is a valid out pointer; on success it holds a
    // NUL-terminated wide string that this function frees with CoTaskMemFree,
    // and on failure Windows still requires the same free.
    unsafe {
        let hr = SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut raw);
        let path = (hr >= 0 && !raw.is_null()).then(|| {
            let mut len = 0;
            while *raw.add(len) != 0 {
                len += 1;
            }
            PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(raw, len)))
        });
        CoTaskMemFree(raw as *const _);
        path
    }
}

pub fn user_content_folders() -> Vec<PathBuf> {
    [
        &FOLDERID_Desktop,
        &FOLDERID_Documents,
        &FOLDERID_Downloads,
        &FOLDERID_Pictures,
        &FOLDERID_Videos,
        &FOLDERID_Music,
    ]
    .into_iter()
    .filter_map(known_folder)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_folders_resolve_documents_and_downloads() {
        let folders = user_content_folders();
        let named = |name: &str| {
            folders
                .iter()
                .any(|path| path.file_name().is_some_and(|n| n == name))
        };
        assert!(named("Documents"));
        assert!(named("Downloads"));
    }
}
