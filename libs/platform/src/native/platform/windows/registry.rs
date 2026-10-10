use std::io;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_SUCCESS, WIN32_ERROR,
};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegDeleteKeyValueW, RegDeleteTreeW, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW,
    RegSetKeyValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY,
    KEY_WOW64_64KEY, REG_DWORD, REG_SZ, REG_VALUE_TYPE, RRF_RT_REG_BINARY, RRF_RT_REG_DWORD,
    RRF_RT_REG_SZ, RRF_SUBKEY_WOW6432KEY, RRF_SUBKEY_WOW6464KEY,
};

use super::wide::{from_wide, wide_nul};

const KEY_NAME_CAPACITY: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hive {
    CurrentUser,
    LocalMachine,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Default,
    Native,
    Wow32,
}

pub struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

impl Key {
    pub fn open(hive: Hive, path: &str, view: View) -> io::Result<Option<Key>> {
        open_under(hive_handle(hive), path, view)
    }

    pub fn open_child(&self, path: &str, view: View) -> io::Result<Option<Key>> {
        open_under(self.0, path, view)
    }

    pub fn subkey_names(&self) -> Vec<String> {
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
                _ => break,
            }
        }
        names
    }

    pub fn string(&self, name: &str) -> io::Result<Option<String>> {
        Ok(get_value(self.0, None, name, RRF_RT_REG_SZ)?.map(|bytes| text(&bytes)))
    }

    pub fn dword(&self, name: &str) -> io::Result<Option<u32>> {
        get_value(self.0, None, name, RRF_RT_REG_DWORD)?
            .map(|bytes| number(&bytes))
            .transpose()
    }
}

pub fn read_string(hive: Hive, path: &str, name: &str, view: View) -> io::Result<Option<String>> {
    let flags = RRF_RT_REG_SZ | view_value_flag(view);
    Ok(get_value(hive_handle(hive), Some(path), name, flags)?.map(|bytes| text(&bytes)))
}

pub fn read_dword(hive: Hive, path: &str, name: &str) -> io::Result<Option<u32>> {
    get_value(hive_handle(hive), Some(path), name, RRF_RT_REG_DWORD)?
        .map(|bytes| number(&bytes))
        .transpose()
}

pub fn read_binary(hive: Hive, path: &str, name: &str) -> io::Result<Option<Vec<u8>>> {
    get_value(hive_handle(hive), Some(path), name, RRF_RT_REG_BINARY)
}

pub fn write_string(hive: Hive, path: &str, name: &str, value: &str) -> io::Result<()> {
    let data = wide_nul(value);
    set_value(hive, path, name, REG_SZ, data_bytes(&data))
}

pub fn write_dword(hive: Hive, path: &str, name: &str, value: u32) -> io::Result<()> {
    set_value(hive, path, name, REG_DWORD, &value.to_le_bytes())
}

pub fn delete_value(hive: Hive, path: &str, name: &str) -> io::Result<()> {
    let path = wide_nul(path);
    let name = wide_nul(name);
    let status = unsafe { RegDeleteKeyValueW(hive_handle(hive), path.as_ptr(), name.as_ptr()) };
    absent_is_done(status)
}

pub fn delete_key(hive: Hive, path: &str) -> io::Result<()> {
    let path = wide_nul(path);
    let status = unsafe { RegDeleteTreeW(hive_handle(hive), path.as_ptr()) };
    absent_is_done(status)
}

fn hive_handle(hive: Hive) -> HKEY {
    match hive {
        Hive::CurrentUser => HKEY_CURRENT_USER,
        Hive::LocalMachine => HKEY_LOCAL_MACHINE,
    }
}

fn view_value_flag(view: View) -> u32 {
    match view {
        View::Default => 0,
        View::Native => RRF_SUBKEY_WOW6464KEY,
        View::Wow32 => RRF_SUBKEY_WOW6432KEY,
    }
}

fn open_under(parent: HKEY, path: &str, view: View) -> io::Result<Option<Key>> {
    let path = wide_nul(path);
    let access = KEY_READ
        | match view {
            View::Default => 0,
            View::Native => KEY_WOW64_64KEY,
            View::Wow32 => KEY_WOW64_32KEY,
        };
    let mut key: HKEY = null_mut();
    let status = unsafe { RegOpenKeyExW(parent, path.as_ptr(), 0, access, &mut key) };
    match status {
        ERROR_SUCCESS => Ok(Some(Key(key))),
        ERROR_FILE_NOT_FOUND => Ok(None),
        error => Err(os_error(error)),
    }
}

fn get_value(key: HKEY, path: Option<&str>, name: &str, flags: u32) -> io::Result<Option<Vec<u8>>> {
    let subkey = path.map(wide_nul);
    let subkey_ptr = subkey.as_ref().map_or(null(), |subkey| subkey.as_ptr());
    let name = wide_nul(name);
    let read = |data: *mut u8, size: &mut u32| unsafe {
        RegGetValueW(
            key,
            subkey_ptr,
            name.as_ptr(),
            flags,
            null_mut::<REG_VALUE_TYPE>(),
            data.cast(),
            size,
        )
    };
    let mut size = 0u32;
    match read(null_mut(), &mut size) {
        ERROR_SUCCESS => {}
        ERROR_FILE_NOT_FOUND => return Ok(None),
        error => return Err(os_error(error)),
    }
    if size == 0 {
        return Ok(Some(Vec::new()));
    }
    let mut data = vec![0u8; size as usize];
    match read(data.as_mut_ptr(), &mut size) {
        ERROR_SUCCESS => {}
        ERROR_FILE_NOT_FOUND => return Ok(None),
        error => return Err(os_error(error)),
    }
    data.truncate(size as usize);
    Ok(Some(data))
}

fn set_value(hive: Hive, path: &str, name: &str, kind: u32, data: &[u8]) -> io::Result<()> {
    let path = wide_nul(path);
    let name = wide_nul(name);
    let size = u32::try_from(data.len()).map_err(io::Error::other)?;
    let status = unsafe {
        RegSetKeyValueW(
            hive_handle(hive),
            path.as_ptr(),
            name.as_ptr(),
            kind,
            data.as_ptr().cast(),
            size,
        )
    };
    match status {
        ERROR_SUCCESS => Ok(()),
        error => Err(os_error(error)),
    }
}

fn absent_is_done(status: WIN32_ERROR) -> io::Result<()> {
    match status {
        ERROR_SUCCESS | ERROR_FILE_NOT_FOUND => Ok(()),
        error => Err(os_error(error)),
    }
}

fn os_error(status: WIN32_ERROR) -> io::Error {
    io::Error::from_raw_os_error(status as i32)
}

fn data_bytes(units: &[u16]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(units.as_ptr().cast(), std::mem::size_of_val(units)) }
}

fn text(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes(*pair))
        .collect();
    from_wide(&units)
}

fn number(bytes: &[u8]) -> io::Result<u32> {
    let bytes: [u8; 4] = bytes.try_into().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("a registry DWORD holds {} bytes", bytes.len()),
        )
    })?;
    Ok(u32::from_le_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KEY: &str = r"Software\qol-platform-registry-test";

    #[test]
    fn values_round_trip_and_delete_cleanly() {
        let key = format!("{TEST_KEY}-{}", std::process::id());
        let cases = [
            ("Name", "QoL Tray"),
            ("Path", r"C:\Users\x y\qol-tray.exe"),
            ("Empty", ""),
        ];
        for (name, value) in cases {
            write_string(Hive::CurrentUser, &key, name, value).unwrap();
        }
        write_dword(Hive::CurrentUser, &key, "Flag", 7).unwrap();
        for (name, value) in cases {
            let read = read_string(Hive::CurrentUser, &key, name, View::Default).unwrap();
            assert_eq!(read.as_deref(), Some(value), "{name}");
        }
        assert_eq!(
            read_dword(Hive::CurrentUser, &key, "Flag").unwrap(),
            Some(7)
        );
        let opened = Key::open(Hive::CurrentUser, &key, View::Default)
            .unwrap()
            .unwrap();
        assert_eq!(opened.string("Name").unwrap().as_deref(), Some("QoL Tray"));
        assert_eq!(opened.dword("Flag").unwrap(), Some(7));
        assert_eq!(opened.dword("Missing").unwrap(), None);
        drop(opened);
        delete_value(Hive::CurrentUser, &key, "Name").unwrap();
        delete_value(Hive::CurrentUser, &key, "Name").unwrap();
        assert_eq!(
            read_string(Hive::CurrentUser, &key, "Name", View::Default).unwrap(),
            None
        );
        delete_key(Hive::CurrentUser, &key).unwrap();
        delete_key(Hive::CurrentUser, &key).unwrap();
        assert!(Key::open(Hive::CurrentUser, &key, View::Default)
            .unwrap()
            .is_none());
        assert_eq!(read_dword(Hive::CurrentUser, &key, "Flag").unwrap(), None);
    }

    #[test]
    fn subkeys_enumerate_through_open_child() {
        let key = format!("{TEST_KEY}-children-{}", std::process::id());
        for child in ["one", "two"] {
            write_dword(Hive::CurrentUser, &format!(r"{key}\{child}"), "Flag", 1).unwrap();
        }
        let parent = Key::open(Hive::CurrentUser, &key, View::Default)
            .unwrap()
            .unwrap();
        let mut names = parent.subkey_names();
        names.sort();
        assert_eq!(names, ["one", "two"]);
        let child = parent.open_child("two", View::Default).unwrap().unwrap();
        assert_eq!(child.dword("Flag").unwrap(), Some(1));
        drop((child, parent));
        delete_key(Hive::CurrentUser, &key).unwrap();
    }

    #[test]
    fn machine_guid_reads_through_the_native_view() {
        let guid = read_string(
            Hive::LocalMachine,
            r"SOFTWARE\Microsoft\Cryptography",
            "MachineGuid",
            View::Native,
        )
        .unwrap();
        assert!(guid.is_some_and(|guid| !guid.is_empty()));
    }
}
