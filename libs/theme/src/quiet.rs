use std::cell::Cell;

use crate::{css_rgba_milli, SystemPalette, Theme, ThemeMode, WashPalette};

pub const LIGHT_QUIET_ACCENT: u32 = 0x8e8e99;
pub const LIGHT_QUIET_ACCENT_INK: u32 = 0x5f5f69;
pub const DARK_QUIET_ACCENT: u32 = 0x7c7c86;
pub const DARK_QUIET_ACCENT_INK: u32 = 0xb4b4be;

thread_local! {
    static WINDOW_QUIET: Cell<bool> = const { Cell::new(false) };
}

pub fn set_window_quiet(quiet: bool) {
    WINDOW_QUIET.set(quiet);
}

pub fn window_is_quiet() -> bool {
    WINDOW_QUIET.get()
}

impl Theme {
    pub fn quiet(self) -> Self {
        let (accent, accent_ink) = match self.mode {
            ThemeMode::Light => (LIGHT_QUIET_ACCENT, LIGHT_QUIET_ACCENT_INK),
            ThemeMode::Dark => (DARK_QUIET_ACCENT, DARK_QUIET_ACCENT_INK),
        };
        let system = SystemPalette {
            success: accent,
            ..self.system.with_accent_pair(accent, accent_ink)
        };
        Self::from_reference_and_system(self.mode, self.reference, system)
    }
}

impl WashPalette {
    pub const fn quiet(self) -> Self {
        Self {
            accent_halo: css_rgba_milli(self.accent_halo.rgb, 0),
            ..self
        }
    }
}
