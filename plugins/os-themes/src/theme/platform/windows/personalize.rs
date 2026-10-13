use anyhow::{bail, Context, Result};
use qol_platform::native::registry::{self, Hive};
use qol_platform::native::wide::wide_nul;
use windows_sys::Win32::Foundation::LPARAM;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE,
};

use crate::session::{RestoreMode, RestoreReport};
use crate::theme::ColorScheme;

use super::super::snapshot;

const PERSONALIZE_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
const APPS_VALUE: &str = "AppsUseLightTheme";
const SYSTEM_VALUE: &str = "SystemUsesLightTheme";
const SCHEME_VALUES: [&str; 2] = [APPS_VALUE, SYSTEM_VALUE];
const CHANGE_AREA: &str = "ImmersiveColorSet";
const SNAPSHOT_SCHEMA: &str = "personalize";
const BROADCAST_TIMEOUT_MS: u32 = 500;

pub(super) fn current_scheme() -> Result<ColorScheme> {
    Ok(scheme_from_light_flag(read_dword(APPS_VALUE)?))
}

pub(super) fn apply_scheme(target: ColorScheme) -> Result<()> {
    let flag = light_flag(target);
    for name in SCHEME_VALUES {
        let prior = read_dword(name)?;
        snapshot::record_baseline(SNAPSHOT_SCHEMA, name, &stored_value(prior))?;
        write_dword(name, flag)?;
    }
    broadcast_color_set_change();
    log::debug!("applied Windows {} mode", target.as_str());
    Ok(())
}

pub(super) fn restore(mode: RestoreMode, report: &mut RestoreReport) {
    let before = report.restored;
    snapshot::restore(mode, report, |saved| {
        if saved.schema != SNAPSHOT_SCHEMA || !SCHEME_VALUES.contains(&saved.key.as_str()) {
            bail!("unknown Windows theme value {}:{}", saved.schema, saved.key);
        }
        match restored_value(&saved.value)? {
            Some(value) => write_dword(&saved.key, value),
            None => delete_value(&saved.key),
        }
    });
    if report.restored > before {
        broadcast_color_set_change();
    }
}

fn stored_value(value: Option<u32>) -> String {
    value.map(|value| value.to_string()).unwrap_or_default()
}

fn restored_value(stored: &str) -> Result<Option<u32>> {
    if stored.is_empty() {
        return Ok(None);
    }
    Ok(Some(stored.parse()?))
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
    registry::read_dword(Hive::CurrentUser, PERSONALIZE_KEY, name)
        .with_context(|| registry_context("read", name))
}

fn write_dword(name: &str, value: u32) -> Result<()> {
    registry::write_dword(Hive::CurrentUser, PERSONALIZE_KEY, name, value)
        .with_context(|| registry_context("write", name))
}

fn delete_value(name: &str) -> Result<()> {
    registry::delete_value(Hive::CurrentUser, PERSONALIZE_KEY, name)
        .with_context(|| registry_context("delete", name))
}

fn registry_context(verb: &str, name: &str) -> String {
    format!(r"could not {verb} HKCU\{PERSONALIZE_KEY}\{name}")
}

fn broadcast_color_set_change() {
    let area = wide_nul(CHANGE_AREA);
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
    fn stored_values_round_trip_including_an_absent_value() {
        for value in [None, Some(0), Some(1), Some(7)] {
            assert_eq!(
                restored_value(&stored_value(value)).unwrap(),
                value,
                "{value:?}"
            );
        }
        assert!(restored_value("dark").is_err());
    }

    #[test]
    fn scheme_round_trips_through_the_light_flag() {
        for scheme in [ColorScheme::Light, ColorScheme::Dark] {
            assert_eq!(scheme_from_light_flag(Some(light_flag(scheme))), scheme);
        }
    }
}
