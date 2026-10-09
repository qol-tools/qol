use anyhow::{bail, Result};
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr::null_mut;
use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows_sys::Win32::System::Registry::{
    RegDeleteKeyValueW, RegDeleteTreeW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER,
    REG_DWORD, REG_SZ, RRF_RT_REG_SZ,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::installer) enum Value {
    Text(String),
    Number(u32),
}

pub(in crate::installer) fn set(key: &str, name: &str, value: &Value) -> Result<()> {
    let key_wide = wide(key);
    let name_wide = wide(name);
    let status = match value {
        Value::Text(text) => {
            let data = wide(text);
            unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    key_wide.as_ptr(),
                    name_wide.as_ptr(),
                    REG_SZ,
                    data.as_ptr().cast(),
                    (data.len() * 2) as u32,
                )
            }
        }
        Value::Number(number) => unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                key_wide.as_ptr(),
                name_wide.as_ptr(),
                REG_DWORD,
                (number as *const u32).cast(),
                4,
            )
        },
    };
    check(status, || format!("write HKCU\\{key}\\{name}"))
}

pub(in crate::installer) fn text(key: &str, name: &str) -> Result<Option<String>> {
    let key_wide = wide(key);
    let name_wide = wide(name);
    let mut size = 0u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key_wide.as_ptr(),
            name_wide.as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            null_mut(),
            &mut size,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    check(status, || format!("read HKCU\\{key}\\{name}"))?;
    let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key_wide.as_ptr(),
            name_wide.as_ptr(),
            RRF_RT_REG_SZ,
            null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    check(status, || format!("read HKCU\\{key}\\{name}"))?;
    let len = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    Ok(Some(String::from_utf16_lossy(&buffer[..len])))
}

pub(in crate::installer) fn delete_value(key: &str, name: &str) -> Result<()> {
    let key_wide = wide(key);
    let name_wide = wide(name);
    let status =
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key_wide.as_ptr(), name_wide.as_ptr()) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    check(status, || format!("delete HKCU\\{key}\\{name}"))
}

pub(in crate::installer) fn delete_key(key: &str) -> Result<()> {
    let key_wide = wide(key);
    let status = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, key_wide.as_ptr()) };
    if status == ERROR_FILE_NOT_FOUND {
        return Ok(());
    }
    check(status, || format!("delete HKCU\\{key}"))
}

fn check(status: WIN32_ERROR, action: impl FnOnce() -> String) -> Result<()> {
    if status == ERROR_SUCCESS {
        return Ok(());
    }
    bail!("failed to {} (status {status})", action())
}

fn wide(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KEY: &str = r"Software\qol-tray-installer-registry-test";

    #[test]
    fn values_round_trip_and_delete_cleanly() {
        let key = format!("{TEST_KEY}-{}", std::process::id());
        let cases = [
            ("Name", "QoL Tray"),
            ("Path", r"C:\Users\x y\qol-tray.exe"),
            ("Empty", ""),
        ];
        for (name, value) in cases {
            set(&key, name, &Value::Text(value.to_string())).unwrap();
        }
        set(&key, "Flag", &Value::Number(1)).unwrap();
        for (name, value) in cases {
            assert_eq!(text(&key, name).unwrap().as_deref(), Some(value), "{name}");
        }
        delete_value(&key, "Name").unwrap();
        delete_value(&key, "Name").unwrap();
        assert_eq!(text(&key, "Name").unwrap(), None);
        delete_key(&key).unwrap();
        delete_key(&key).unwrap();
        assert_eq!(text(&key, "Path").unwrap(), None);
    }
}
