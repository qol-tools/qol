use std::ptr::null_mut;

use anyhow::{bail, Result};
use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, LPARAM, WIN32_ERROR};
use windows_sys::Win32::System::Registry::{
    RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_DWORD, RRF_RT_REG_DWORD,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
};

use crate::theme::ColorScheme;

const PERSONALIZE_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
const APPS_VALUE: &str = "AppsUseLightTheme";
const SYSTEM_VALUE: &str = "SystemUsesLightTheme";
const SCHEME_VALUES: [&str; 2] = [APPS_VALUE, SYSTEM_VALUE];
const CHANGE_AREA: &str = "ImmersiveColorSet";
const BROADCAST_TIMEOUT_MS: u32 = 500;

pub(super) fn current_scheme() -> Result<ColorScheme> {
    Ok(scheme_from_light_flag(read_dword(APPS_VALUE)?))
}

pub(super) fn apply_scheme(target: ColorScheme) -> Result<()> {
    let flag = light_flag(target);
    for name in SCHEME_VALUES {
        write_dword(name, flag)?;
    }
    broadcast_color_set_change();
    log::debug!("applied Windows {} mode", target.as_str());
    Ok(())
}

fn scheme_from_light_flag(flag: Option<u32>) -> ColorScheme {
    match flag {
        Some(0) => ColorScheme::Dark,
        Some(_) | None => ColorScheme::Light,
    }
}

fn light_flag(scheme: ColorScheme) -> u32 {
    match scheme {
        ColorScheme::Light => 1,
        ColorScheme::Dark => 0,
    }
}

fn read_dword(name: &str) -> Result<Option<u32>> {
    let key = wide(PERSONALIZE_KEY);
    let value_name = wide(name);
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value_name.as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&mut value as *mut u32).cast(),
            &mut size,
        )
    };
    match status {
        ERROR_SUCCESS => Ok(Some(value)),
        ERROR_FILE_NOT_FOUND => Ok(None),
        error => registry_error("read", name, error),
    }
}

fn write_dword(name: &str, value: u32) -> Result<()> {
    let key = wide(PERSONALIZE_KEY);
    let value_name = wide(name);
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value_name.as_ptr(),
            REG_DWORD,
            (&value as *const u32).cast(),
            std::mem::size_of::<u32>() as u32,
        )
    };
    if status == ERROR_SUCCESS {
        return Ok(());
    }
    registry_error("write", name, status)
}

fn registry_error<T>(verb: &str, name: &str, status: WIN32_ERROR) -> Result<T> {
    let error = std::io::Error::from_raw_os_error(status as i32);
    bail!(r"could not {verb} HKCU\{PERSONALIZE_KEY}\{name}: {error}")
}

fn broadcast_color_set_change() {
    let area = wide(CHANGE_AREA);
    let mut result = 0usize;
    let sent = unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            area.as_ptr() as LPARAM,
            SMTO_ABORTIFHUNG,
            BROADCAST_TIMEOUT_MS,
            &mut result,
        )
    };
    if sent == 0 {
        log::warn!(
            "{CHANGE_AREA} broadcast timed out: {}",
            std::io::Error::last_os_error()
        );
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn light_flag_maps_to_scheme() {
        let cases = [
            (Some(0), ColorScheme::Dark),
            (Some(1), ColorScheme::Light),
            (Some(7), ColorScheme::Light),
            (None, ColorScheme::Light),
        ];
        for (flag, expected) in cases {
            assert_eq!(scheme_from_light_flag(flag), expected, "flag={flag:?}");
        }
    }

    #[test]
    fn scheme_round_trips_through_the_light_flag() {
        for scheme in [ColorScheme::Light, ColorScheme::Dark] {
            assert_eq!(scheme_from_light_flag(Some(light_flag(scheme))), scheme);
        }
    }
}
