pub mod platform;
#[cfg(target_os = "linux")]
pub(crate) mod session;

use anyhow::Result;

use platform::{Platform, ThemePlatform};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColorScheme {
    Light,
    Dark,
}

impl ColorScheme {
    fn opposite(self) -> Self {
        match self {
            ColorScheme::Light => ColorScheme::Dark,
            ColorScheme::Dark => ColorScheme::Light,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ColorScheme::Light => "light",
            ColorScheme::Dark => "dark",
        }
    }

    pub fn is_dark(self) -> bool {
        self == ColorScheme::Dark
    }
}

pub fn current() -> Result<ColorScheme> {
    Platform.current_scheme()
}

pub fn toggle() -> Result<ColorScheme> {
    let platform = Platform;
    let target = platform.current_scheme()?.opposite();
    platform.apply_scheme(target)?;
    Ok(target)
}

#[cfg(target_os = "linux")]
pub fn restore(mode: crate::session::RestoreMode, report: &mut crate::session::RestoreReport) {
    platform::restore_linux(mode, report);
}
