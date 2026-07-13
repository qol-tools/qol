pub mod platform;

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
}

pub fn toggle() -> Result<ColorScheme> {
    let platform = Platform;
    let target = platform.current_scheme()?.opposite();
    platform.apply_scheme(target)?;
    Ok(target)
}
