use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::ptr::null_mut;

use anyhow::{bail, Result};
use windows_sys::Win32::Foundation::ERROR_SUCCESS;
use windows_sys::Win32::System::Registry::{
    RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6464KEY,
};

const CRYPTOGRAPHY_KEY: &str = r"SOFTWARE\Microsoft\Cryptography";
const MACHINE_GUID_VALUE: &str = "MachineGuid";

pub(crate) fn device_id() -> Result<String> {
    let guid = machine_guid()?;
    let owner = crate::policy::host_owner_from_machine_id("residency", &guid)?;
    Ok(owner.as_str().to_string())
}

fn machine_guid() -> Result<String> {
    let key = wide(CRYPTOGRAPHY_KEY);
    let value = wide(MACHINE_GUID_VALUE);
    let flags = RRF_RT_REG_SZ | RRF_SUBKEY_WOW6464KEY;
    let mut size = 0u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            flags,
            null_mut(),
            null_mut(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS || size == 0 {
        bail!("failed to read HKLM\\{CRYPTOGRAPHY_KEY}\\{MACHINE_GUID_VALUE} (status {status})");
    }
    let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            key.as_ptr(),
            value.as_ptr(),
            flags,
            null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != ERROR_SUCCESS {
        bail!("failed to read HKLM\\{CRYPTOGRAPHY_KEY}\\{MACHINE_GUID_VALUE} (status {status})");
    }
    let len = buffer
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(buffer.len());
    Ok(OsString::from_wide(&buffer[..len])
        .to_string_lossy()
        .into_owned())
}

fn wide(value: &str) -> Vec<u16> {
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(Some(0))
        .collect()
}
