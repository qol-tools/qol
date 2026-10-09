use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER,
    HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY, RRF_RT_REG_DWORD,
    RRF_RT_REG_SZ,
};

use super::catalog::{entry_from, Hive, KeyLocation, RawValues, UninstallEntry, View};

const SOURCES: [(Hive, View); 3] = [
    (Hive::LocalMachine, View::Native),
    (Hive::LocalMachine, View::Wow32),
    (Hive::CurrentUser, View::Native),
];
const KEY_NAME_CAPACITY: usize = 256;

struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

impl Key {
    fn open(parent: HKEY, path: &str, view: View) -> Option<Key> {
        let path = wide(path);
        let view_flag = match view {
            View::Native => KEY_WOW64_64KEY,
            View::Wow32 => KEY_WOW64_32KEY,
        };
        let mut key: HKEY = null_mut();
        let status =
            unsafe { RegOpenKeyExW(parent, path.as_ptr(), 0, KEY_READ | view_flag, &mut key) };
        (status == ERROR_SUCCESS).then_some(Key(key))
    }

    fn subkey_names(&self) -> Vec<String> {
        let mut names = Vec::new();
        for index in 0.. {
            let mut buffer = [0u16; KEY_NAME_CAPACITY];
            let mut length = buffer.len() as u32;
            let status = unsafe {
                RegEnumKeyExW(
                    self.0,
                    index,
                    buffer.as_mut_ptr(),
                    &mut length,
                    null(),
                    null_mut(),
                    null_mut(),
                    null_mut(),
                )
            };
            match status {
                ERROR_SUCCESS => names.push(String::from_utf16_lossy(&buffer[..length as usize])),
                ERROR_MORE_DATA => continue,
                ERROR_NO_MORE_ITEMS => break,
                _ => break,
            }
        }
        names
    }

    fn string(&self, name: &str) -> Option<String> {
        let name = wide(name);
        let mut bytes = 0u32;
        let probe = unsafe {
            RegGetValueW(
                self.0,
                null(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                null_mut(),
                null_mut(),
                &mut bytes,
            )
        };
        if probe != ERROR_SUCCESS || bytes == 0 {
            return None;
        }
        let mut buffer = vec![0u16; (bytes as usize).div_ceil(2)];
        let mut size = (buffer.len() * 2) as u32;
        let read = unsafe {
            RegGetValueW(
                self.0,
                null(),
                name.as_ptr(),
                RRF_RT_REG_SZ,
                null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if read != ERROR_SUCCESS {
            return None;
        }
        let units = (size as usize / 2).min(buffer.len());
        let end = buffer[..units]
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(units);
        Some(String::from_utf16_lossy(&buffer[..end]))
    }

    fn flag(&self, name: &str) -> bool {
        let name = wide(name);
        let mut value = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        let read = unsafe {
            RegGetValueW(
                self.0,
                null(),
                name.as_ptr(),
                RRF_RT_REG_DWORD,
                null_mut(),
                (&mut value as *mut u32).cast(),
                &mut size,
            )
        };
        read == ERROR_SUCCESS && value != 0
    }

    fn values(&self) -> RawValues {
        RawValues {
            display_name: self.string("DisplayName"),
            publisher: self.string("Publisher"),
            uninstall_string: self.string("UninstallString"),
            quiet_uninstall_string: self.string("QuietUninstallString"),
            install_location: self.string("InstallLocation"),
            display_icon: self.string("DisplayIcon"),
            parent_key_name: self.string("ParentKeyName"),
            release_type: self.string("ReleaseType"),
            system_component: self.flag("SystemComponent"),
            windows_installer: self.flag("WindowsInstaller"),
            no_remove: self.flag("NoRemove"),
        }
    }
}

fn hive_handle(hive: Hive) -> HKEY {
    match hive {
        Hive::LocalMachine => HKEY_LOCAL_MACHINE,
        Hive::CurrentUser => HKEY_CURRENT_USER,
    }
}

pub(super) fn uninstall_entries() -> Vec<UninstallEntry> {
    let mut entries = Vec::new();
    for (hive, view) in SOURCES {
        let Some(parent) = Key::open(
            hive_handle(hive),
            &KeyLocation::parent_path(hive, view),
            view,
        ) else {
            continue;
        };
        for key in parent.subkey_names() {
            let Some(child) = Key::open(parent.0, &key, view) else {
                continue;
            };
            let location = KeyLocation { hive, view, key };
            entries.extend(entry_from(location, child.values()));
        }
    }
    entries
}

pub(super) fn is_registered(location: &KeyLocation) -> bool {
    let path = format!(
        r"{}\{}",
        KeyLocation::parent_path(location.hive, location.view),
        location.key
    );
    Key::open(hive_handle(location.hive), &path, location.view).is_some()
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}
