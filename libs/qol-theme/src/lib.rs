pub mod css;

use qol_color::{
    clamp_unit, mix_rgb, parse_hex_color, rgb24, rgba_from_rgb, scale_rgb, with_alpha,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AccentPreset {
    pub key: &'static str,
    pub label: &'static str,
    pub rgb: u32,
    pub hover: u32,
    pub ink: u32,
}

pub const PROD_ACCENT_KEY: &str = "amber";

pub const THEME_COLOR_SENTINEL: &str = "theme";

pub const TEXT_NANO: f32 = 10.0;
pub const TEXT_MICRO: f32 = 11.0;
pub const TEXT_CAPTION: f32 = 12.0;
pub const TEXT_BODY: f32 = 14.0;
pub const TEXT_TITLE: f32 = 17.0;
pub const TEXT_DISPLAY: f32 = 20.0;

pub const TEXT_SCALE: [f32; 5] = [TEXT_NANO, TEXT_CAPTION, TEXT_BODY, TEXT_TITLE, TEXT_DISPLAY];

pub fn font_ui() -> &'static str {
    if cfg!(target_os = "macos") {
        "SF Pro Text"
    } else if cfg!(target_os = "windows") {
        "Segoe UI"
    } else {
        "DejaVu Sans"
    }
}

pub fn font_mono() -> &'static str {
    if cfg!(target_os = "macos") {
        "SF Mono"
    } else if cfg!(target_os = "windows") {
        "Cascadia Mono"
    } else {
        "DejaVu Sans Mono"
    }
}

pub const DARK_ACCENT_PRESETS: [AccentPreset; 6] = [
    AccentPreset {
        key: "amber",
        label: "Amber",
        rgb: DARK_REFERENCE.orange_400,
        hover: 0xffc77a,
        ink: DARK_REFERENCE.orange_400,
    },
    AccentPreset {
        key: "green",
        label: "Green",
        rgb: 0x46e08a,
        hover: 0x7ff0ab,
        ink: 0x46e08a,
    },
    AccentPreset {
        key: "cyan",
        label: "Cyan",
        rgb: 0x56d6e0,
        hover: 0x8fe8f0,
        ink: 0x56d6e0,
    },
    AccentPreset {
        key: "magenta",
        label: "Magenta",
        rgb: 0xe879c6,
        hover: 0xf49ad6,
        ink: 0xe879c6,
    },
    AccentPreset {
        key: "blue",
        label: "Blue",
        rgb: DARK_TRAY_RAMP.blue_500,
        hover: 0x68b0ff,
        ink: DARK_TRAY_RAMP.blue_500,
    },
    AccentPreset {
        key: "violet",
        label: "Violet",
        rgb: 0x8a93f7,
        hover: 0xa5afff,
        ink: 0x8a93f7,
    },
];

pub fn dark_accent_presets() -> &'static [AccentPreset] {
    &DARK_ACCENT_PRESETS
}

pub fn dark_accent_preset(key: &str) -> Option<AccentPreset> {
    DARK_ACCENT_PRESETS
        .iter()
        .copied()
        .find(|preset| preset.key == key)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeMode {
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    pub mode: ThemeMode,
    pub reference: ReferencePalette,
    pub system: SystemPalette,
    pub components: ComponentPalettes,
}

impl Theme {
    pub fn from_reference(mode: ThemeMode, reference: ReferencePalette) -> Self {
        let system = SystemPalette::from_reference(reference);
        Self::from_reference_and_system(mode, reference, system)
    }

    pub fn from_reference_and_system(
        mode: ThemeMode,
        reference: ReferencePalette,
        system: SystemPalette,
    ) -> Self {
        let components = ComponentPalettes::new(mode, reference, system);
        Self {
            mode,
            reference,
            system,
            components,
        }
    }
}

pub fn dark_theme() -> Theme {
    Theme::from_reference(ThemeMode::Dark, DARK_REFERENCE)
}

pub fn dark_theme_with_accent_key(key: &str) -> Theme {
    let accent = dark_accent_preset(key)
        .unwrap_or_else(|| dark_accent_preset(PROD_ACCENT_KEY).expect("default accent exists"))
        .rgb;
    let system = SystemPalette::from_reference(DARK_REFERENCE).with_accent(accent);
    Theme::from_reference_and_system(ThemeMode::Dark, DARK_REFERENCE, system)
}

pub fn runtime_dark_theme() -> Theme {
    match std::env::var(qol_conventions::ENV_THEME_ACCENT) {
        Ok(key) => dark_theme_with_accent_key(&key),
        Err(_) => dark_theme(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReferencePalette {
    pub black: u32,
    pub white: u32,
    pub night_950: u32,
    pub night_900: u32,
    pub night_850: u32,
    pub night_800: u32,
    pub slate_750: u32,
    pub slate_700: u32,
    pub slate_650: u32,
    pub slate_600: u32,
    pub slate_550: u32,
    pub slate_500: u32,
    pub slate_300: u32,
    pub slate_200: u32,
    pub slate_100: u32,
    pub slate_050: u32,
    pub warm_slate_300: u32,
    pub orange_400: u32,
    pub accent_ink: u32,
    pub accent_fill: u32,
    pub blue_400: u32,
    pub green_400: u32,
    pub red_500: u32,
    pub amber_500: u32,
}

pub const DARK_REFERENCE: ReferencePalette = ReferencePalette {
    black: 0x000000,
    white: 0xffffff,
    night_950: 0x0c0e13,
    night_900: 0x14181f,
    night_850: 0x171c26,
    night_800: 0x1f2531,
    slate_750: 0x2f3644,
    slate_700: 0x3a4252,
    slate_650: 0x4a5268,
    slate_600: 0x4d5870,
    slate_550: 0x5e6a84,
    slate_500: 0x67748f,
    slate_300: 0xb8c0d0,
    slate_200: 0xd4dbea,
    slate_100: 0xedf2fb,
    slate_050: 0xf8fbff,
    warm_slate_300: 0xc7d0c9,
    orange_400: 0xffb454,
    accent_ink: 0xffb454,
    accent_fill: 0x2b2116,
    blue_400: 0x68b0ff,
    green_400: 0x4ade80,
    red_500: 0xff6b6b,
    amber_500: 0xffc107,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SystemPalette {
    pub surface_canvas: u32,
    pub surface_elevated: u32,
    pub surface_raised: u32,
    pub surface_hovered: u32,
    pub text_primary: u32,
    pub text_secondary: u32,
    pub text_muted: u32,
    pub text_faint: u32,
    pub border_subtle: u32,
    pub accent: u32,
    pub accent_ink: u32,
    pub success: u32,
    pub danger: u32,
    pub info: u32,
    pub warning: u32,
    pub accent_fill: u32,
}

impl SystemPalette {
    pub const fn from_reference(reference: ReferencePalette) -> Self {
        Self {
            surface_canvas: reference.night_950,
            surface_elevated: reference.night_900,
            surface_raised: reference.night_850,
            surface_hovered: reference.night_800,
            text_primary: reference.slate_100,
            text_secondary: reference.slate_300,
            text_muted: reference.slate_500,
            text_faint: reference.slate_600,
            border_subtle: reference.slate_750,
            accent: reference.orange_400,
            accent_ink: reference.accent_ink,
            success: reference.green_400,
            danger: reference.red_500,
            info: reference.blue_400,
            warning: reference.amber_500,
            accent_fill: reference.accent_fill,
        }
    }

    pub const fn with_accent(self, accent: u32) -> Self {
        Self {
            accent,
            accent_ink: accent,
            ..self
        }
    }

    pub const fn with_accent_pair(self, accent: u32, accent_ink: u32) -> Self {
        Self {
            accent,
            accent_ink,
            ..self
        }
    }
}

pub const DARK_SYSTEM: SystemPalette = SystemPalette::from_reference(DARK_REFERENCE);

pub const LIGHT_ACCENT_PRESETS: [AccentPreset; 6] = [
    AccentPreset {
        key: "amber",
        label: "Harbour",
        rgb: 0x2f74a0,
        hover: 0x4785ad,
        ink: 0x1f5a82,
    },
    AccentPreset {
        key: "green",
        label: "Moss",
        rgb: 0x5f8a42,
        hover: 0x719a54,
        ink: 0x47692f,
    },
    AccentPreset {
        key: "cyan",
        label: "Verdigris",
        rgb: 0x2f8a86,
        hover: 0x479a96,
        ink: 0x226662,
    },
    AccentPreset {
        key: "magenta",
        label: "Clay",
        rgb: 0xb75f4d,
        hover: 0xc4735f,
        ink: 0x94452f,
    },
    AccentPreset {
        key: "blue",
        label: "Iris",
        rgb: 0x6f5da8,
        hover: 0x8070b5,
        ink: 0x54438a,
    },
    AccentPreset {
        key: "violet",
        label: "Brass",
        rgb: 0xa98f1c,
        hover: 0xb89f33,
        ink: 0x7f6a10,
    },
];

pub fn light_accent_presets() -> &'static [AccentPreset] {
    &LIGHT_ACCENT_PRESETS
}

pub fn light_accent_preset(key: &str) -> Option<AccentPreset> {
    LIGHT_ACCENT_PRESETS
        .iter()
        .copied()
        .find(|preset| preset.key == key)
}

pub const LIGHT_REFERENCE: ReferencePalette = ReferencePalette {
    black: 0x000000,
    white: 0xffffff,
    night_950: 0xe4dccb,
    night_900: 0xfaf7f0,
    night_850: 0xfffdf8,
    night_800: 0xefe8d9,
    slate_750: 0xa89a7c,
    slate_700: 0x6e6556,
    slate_650: 0x8c8270,
    slate_600: 0x8c8270,
    slate_550: 0x6e6556,
    slate_500: 0x6e6556,
    slate_300: 0x4a443a,
    slate_200: 0x4a443a,
    slate_100: 0x2b2721,
    slate_050: 0x2b2721,
    warm_slate_300: 0x4b4334,
    orange_400: 0x2f74a0,
    accent_ink: 0x1f5a82,
    accent_fill: 0xd7e8f3,
    blue_400: 0x2f7ba6,
    green_400: 0x3d9150,
    red_500: 0xc34a32,
    amber_500: 0xe08a00,
};

pub const LIGHT_SYSTEM: SystemPalette = SystemPalette::from_reference(LIGHT_REFERENCE);

pub fn light_theme() -> Theme {
    Theme::from_reference(ThemeMode::Light, LIGHT_REFERENCE)
}

pub fn light_theme_with_accent_key(key: &str) -> Theme {
    let Some(preset) = light_accent_preset(key) else {
        return light_theme();
    };
    let system =
        SystemPalette::from_reference(LIGHT_REFERENCE).with_accent_pair(preset.rgb, preset.ink);
    Theme::from_reference_and_system(ThemeMode::Light, LIGHT_REFERENCE, system)
}

pub fn theme_for_native_key(native: Option<&str>, accent: Option<&str>) -> Theme {
    match native {
        Some("slate") => match accent {
            Some(key) => dark_theme_with_accent_key(key),
            None => dark_theme(),
        },
        _ => match accent {
            Some(key) => light_theme_with_accent_key(key),
            None => light_theme(),
        },
    }
}

static RUNTIME_THEME_OVERRIDE: std::sync::RwLock<Option<(Option<String>, Option<String>)>> =
    std::sync::RwLock::new(None);

pub fn set_runtime_theme_override(native: Option<&str>, accent: Option<&str>) {
    let mut guard = RUNTIME_THEME_OVERRIDE
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = Some((native.map(str::to_string), accent.map(str::to_string)));
}

pub fn runtime_theme() -> Theme {
    let override_guard = RUNTIME_THEME_OVERRIDE
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some((native, accent)) = override_guard.as_ref() {
        return theme_for_native_key(native.as_deref(), accent.as_deref());
    }
    theme_for_native_key(
        std::env::var(qol_conventions::ENV_THEME_NAME)
            .ok()
            .as_deref(),
        std::env::var(qol_conventions::ENV_THEME_ACCENT)
            .ok()
            .as_deref(),
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverlayPalette {
    pub surface_rgb: u32,
    pub deep_rgb: u32,
    pub ink_rgb: u32,
    pub scrim_rgb: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TuiBackgroundPalette {
    pub desktop: u32,
    pub screen: u32,
    pub panel: u32,
    pub card: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThemeIdentity {
    pub key: &'static str,
    pub font_ui: &'static str,
    pub font_data: &'static str,
    pub case_label: &'static str,
    pub tracking_label: &'static str,
    pub radius_2xs: u8,
    pub radius_xs: u8,
    pub radius_sm: u8,
    pub radius_chip: u8,
    pub radius_md: u8,
    pub radius_lg: u8,
    pub radius_lg_plus: u8,
    pub radius_xl: u8,
    pub glow_text: &'static str,
    pub line: &'static str,
    pub line_soft: &'static str,
    pub surface_inset: &'static str,
    pub surface_row: &'static str,
    pub desktop_bg: &'static str,
    pub prompt_display: &'static str,
    pub frame_border: &'static str,
    pub frame_texture: &'static str,
    pub frame_bg: &'static str,
    pub frame_radius: &'static str,
    pub frame_shadow: &'static str,
    pub crt_band_display: &'static str,
    pub card_border: &'static str,
    pub card_bg: &'static str,
    pub card_shadow: &'static str,
    pub cover_bg: &'static str,
    pub cover_texture: &'static str,
    pub cover_scrim: &'static str,
    pub sel_outline: &'static str,
    pub sel_outline_offset: &'static str,
    pub ghost_btn_bg: &'static str,
    pub ghost_btn_radius: &'static str,
    pub hint_bg: &'static str,
    pub hint_border: &'static str,
    pub hint_shadow: &'static str,
    pub panel_bg: &'static str,
    pub panel_border: &'static str,
    pub panel_radius: &'static str,
    pub panel_shadow: &'static str,
    pub heading_size: &'static str,
    pub heading_weight: &'static str,
    pub heading_border: &'static str,
    pub heading_bg: &'static str,
    pub minimap_slab_radius: u8,
}

pub const RETRO_IDENTITY: ThemeIdentity = ThemeIdentity {
    key: "retro",
    font_ui: "var(--font-mono)",
    font_data: "var(--font-mono)",
    case_label: "uppercase",
    tracking_label: "var(--ls-md)",
    radius_2xs: 2,
    radius_xs: 3,
    radius_sm: 4,
    radius_chip: 5,
    radius_md: 6,
    radius_lg: 8,
    radius_lg_plus: 10,
    radius_xl: 12,
    glow_text: "var(--tui-glow-text)",
    line: "var(--tui-line)",
    line_soft: "var(--tui-line-soft)",
    surface_inset: "var(--tui-bg-screen)",
    surface_row: "var(--tui-bg-card)",
    desktop_bg: "var(--tui-desktop-bg)",
    prompt_display: "inline",
    frame_border: "var(--border-w-3) double var(--tui-line)",
    frame_texture: "var(--tui-scanline)",
    frame_bg: "var(--tui-bg-screen)",
    frame_radius: "var(--radius-md)",
    frame_shadow: "none",
    crt_band_display: "block",
    card_border: "var(--border-w-1) solid var(--tui-line-soft)",
    card_bg: "linear-gradient(180deg, rgba(var(--accent-rgb), 0.05), transparent 55%), var(--tui-bg-card)",
    card_shadow: "inset 0 1px 0 rgba(var(--accent-rgb), 0.08), 0 6px 18px var(--layer-ink-45)",
    cover_bg: "var(--tui-screen-bg)",
    cover_texture: "var(--tui-scanline)",
    cover_scrim: "var(--ink-overlay-strong)",
    sel_outline: "none",
    sel_outline_offset: "0px",
    ghost_btn_bg: "var(--layer-ink-45)",
    ghost_btn_radius: "var(--radius-pill)",
    hint_bg: "var(--tui-bg-panel)",
    hint_border: "var(--border-w-1) solid var(--tui-line-soft)",
    hint_shadow: "none",
    panel_bg: "var(--tui-bg-panel)",
    panel_border: "var(--border-w-1) solid var(--tui-line)",
    panel_radius: "var(--radius-md)",
    panel_shadow: "none",
    heading_size: "var(--fs-xl-plus)",
    heading_weight: "var(--fw-bold)",
    heading_border: "var(--border-w-3) double var(--tui-line)",
    heading_bg: "var(--tui-sign-bg)",
    minimap_slab_radius: 3,
};

pub const MODERN_IDENTITY: ThemeIdentity = ThemeIdentity {
    key: "modern",
    font_ui: "var(--font-sans)",
    font_data: "var(--font-mono)",
    case_label: "none",
    tracking_label: "normal",
    radius_2xs: 6,
    radius_xs: 7,
    radius_sm: 8,
    radius_chip: 9,
    radius_md: 10,
    radius_lg: 13,
    radius_lg_plus: 15,
    radius_xl: 18,
    glow_text: "none",
    line: "var(--qol-system-border-subtle)",
    line_soft: "rgba(var(--paper-rgb), 0.07)",
    surface_inset: "var(--qol-system-surface-raised)",
    surface_row: "var(--qol-system-surface-raised)",
    desktop_bg: "var(--surface-canvas)",
    prompt_display: "none",
    frame_border: "none",
    frame_texture:
        "linear-gradient(180deg, rgba(var(--paper-rgb), 0.035), rgba(var(--paper-rgb), 0.015))",
    frame_bg: "var(--surface-elevated)",
    frame_radius: "var(--radius-xl)",
    frame_shadow: "0 24px 60px rgba(0, 0, 0, 0.55), 0 0 0 1px rgba(var(--paper-rgb), 0.07)",
    crt_band_display: "none",
    card_border: "none",
    card_bg: "var(--qol-system-surface-raised)",
    card_shadow: "0 14px 36px rgba(0, 0, 0, 0.5), 0 2px 6px rgba(0, 0, 0, 0.35)",
    cover_bg: "transparent",
    cover_texture: "none",
    cover_scrim: "var(--qol-system-surface-raised)",
    sel_outline: "2px solid rgba(var(--accent-rgb), 0.85)",
    sel_outline_offset: "3px",
    ghost_btn_bg: "rgba(var(--accent-rgb), 0.14)",
    ghost_btn_radius: "10px",
    hint_bg: "var(--qol-system-surface-raised)",
    hint_border: "1px solid rgba(var(--accent-rgb), 0.25)",
    hint_shadow: "0 6px 18px rgba(0, 0, 0, 0.4)",
    panel_bg: "var(--qol-system-surface-raised)",
    panel_border: "none",
    panel_radius: "16px",
    panel_shadow: "0 14px 36px rgba(0, 0, 0, 0.5)",
    heading_size: "1.55rem",
    heading_weight: "700",
    heading_border: "none",
    heading_bg: "transparent",
    minimap_slab_radius: 8,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrayThemePreset {
    pub key: &'static str,
    pub label: &'static str,
    pub accent_key: &'static str,
    pub identity: &'static ThemeIdentity,
    pub system: SystemPalette,
    pub overlay: OverlayPalette,
    pub tui: TuiBackgroundPalette,
}

pub const DEFAULT_TRAY_THEME_KEY: &str = "slate";

pub const NATIVE_THEME_KEYS: [&str; 2] = ["bone", "slate"];

pub const DEFAULT_NATIVE_THEME_KEY: &str = "bone";

pub const TRAY_THEME_PRESETS: [TrayThemePreset; 2] = [
    TrayThemePreset {
        key: "slate",
        label: "Slate",
        accent_key: "amber",
        identity: &RETRO_IDENTITY,
        system: SystemPalette {
            surface_canvas: 0x0b0d12,
            surface_elevated: 0x151a23,
            surface_raised: 0x1b212c,
            surface_hovered: 0x242c3a,
            text_primary: DARK_REFERENCE.slate_100,
            text_secondary: DARK_REFERENCE.slate_300,
            text_muted: DARK_REFERENCE.slate_500,
            text_faint: DARK_REFERENCE.slate_600,
            border_subtle: DARK_REFERENCE.slate_750,
            accent: DARK_REFERENCE.orange_400,
            accent_ink: DARK_REFERENCE.orange_400,
            success: DARK_REFERENCE.green_400,
            danger: DARK_REFERENCE.red_500,
            info: DARK_REFERENCE.blue_400,
            warning: DARK_REFERENCE.amber_500,
            accent_fill: DARK_REFERENCE.accent_fill,
        },
        overlay: OverlayPalette {
            surface_rgb: 0x12161e,
            deep_rgb: 0x111419,
            ink_rgb: 0x070a0e,
            scrim_rgb: 0x040508,
        },
        tui: TuiBackgroundPalette {
            desktop: 0x07080b,
            screen: 0x070809,
            panel: 0x0c0e12,
            card: 0x0a0b0d,
        },
    },
    TrayThemePreset {
        key: "midnight",
        label: "Midnight",
        accent_key: "violet",
        identity: &MODERN_IDENTITY,
        system: SystemPalette {
            surface_canvas: 0x090b19,
            surface_elevated: 0x141626,
            surface_raised: 0x1b1e30,
            surface_hovered: 0x26283c,
            text_primary: 0xe5e7f4,
            text_secondary: 0xb2b6cc,
            text_muted: 0x777b97,
            text_faint: 0x555974,
            border_subtle: 0x34374d,
            accent: 0x8a93f7,
            accent_ink: 0x8a93f7,
            success: DARK_REFERENCE.green_400,
            danger: DARK_REFERENCE.red_500,
            info: DARK_REFERENCE.blue_400,
            warning: DARK_REFERENCE.amber_500,
            accent_fill: 0x1c1830,
        },
        overlay: OverlayPalette {
            surface_rgb: 0x0f1121,
            deep_rgb: 0x0b0d1b,
            ink_rgb: 0x03030b,
            scrim_rgb: 0x010105,
        },
        tui: TuiBackgroundPalette {
            desktop: 0x04040e,
            screen: 0x03030b,
            panel: 0x080916,
            card: 0x050510,
        },
    },
];

pub fn tray_theme_presets() -> &'static [TrayThemePreset] {
    &TRAY_THEME_PRESETS
}

pub fn tray_theme_preset(key: &str) -> Option<TrayThemePreset> {
    TRAY_THEME_PRESETS
        .iter()
        .copied()
        .find(|preset| preset.key == key)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrayRampPalette {
    pub slate_975: u32,
    pub slate_950: u32,
    pub slate_900: u32,
    pub slate_850: u32,
    pub slate_800: u32,
    pub slate_700: u32,
    pub slate_400: u32,
    pub slate_200: u32,
    pub blue_700: u32,
    pub blue_600: u32,
    pub blue_500: u32,
    pub blue_300: u32,
    pub green_700: u32,
    pub green_600: u32,
    pub green_500: u32,
    pub green_300: u32,
    pub red_700: u32,
    pub red_600: u32,
    pub red_400: u32,
    pub red_300: u32,
    pub amber_700: u32,
    pub amber_600: u32,
    pub amber_400: u32,
    pub amber_300: u32,
}

pub const DARK_TRAY_RAMP: TrayRampPalette = TrayRampPalette {
    slate_975: 0x111317,
    slate_950: 0x161920,
    slate_900: 0x1a1e26,
    slate_850: 0x20252f,
    slate_800: 0x272d38,
    slate_700: 0x394257,
    slate_400: 0x8a97ae,
    slate_200: 0xd6deeb,
    blue_700: 0x2d75d5,
    blue_600: 0x3a88ea,
    blue_500: 0x4a9eff,
    blue_300: 0x8fc4ff,
    green_700: 0x179e4f,
    green_600: 0x1fb55b,
    green_500: 0x32cd73,
    green_300: 0x78e8a0,
    red_700: 0xdb4747,
    red_600: 0xea5656,
    red_400: 0xff8787,
    red_300: 0xffabab,
    amber_700: 0xc98a00,
    amber_600: 0xdca110,
    amber_400: 0xffd34e,
    amber_300: 0xffe08a,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CssRgba {
    pub rgb: u32,
    pub alpha_milli: u16,
}

pub const fn css_rgba_milli(rgb: u32, alpha_milli: u16) -> CssRgba {
    CssRgba { rgb, alpha_milli }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TrayInternalPalette {
    pub border_default_2: u32,
    pub border_strong: u32,
    pub atmosphere_wood_bg: u32,
    pub atmosphere_wood_glow_a: CssRgba,
    pub atmosphere_wood_glow_b: CssRgba,
    pub atmosphere_parchment_bg: u32,
    pub atmosphere_parchment_glow_a: CssRgba,
    pub atmosphere_parchment_glow_b: CssRgba,
    pub atmosphere_terminal_bg: u32,
    pub atmosphere_terminal_glow: CssRgba,
    pub atmosphere_spacecraft_bg: u32,
    pub atmosphere_spacecraft_glow_a: CssRgba,
    pub atmosphere_spacecraft_glow_b: CssRgba,
    pub minimap_inactive_fill: CssRgba,
    pub minimap_inactive_stroke: CssRgba,
    pub minimap_inactive_text: CssRgba,
    pub minimap_active_fill: CssRgba,
    pub minimap_active_stroke: CssRgba,
    pub minimap_active_text: CssRgba,
    pub dissolve_target: u32,
    pub transparent_rgba: CssRgba,
    pub config_field_thumb_bg: u32,
    pub config_qr_dark: u32,
    pub config_qr_light: u32,
    pub config_live_color_fallback: u32,
    pub config_color_thumb_stroke: u32,
    pub config_color_thumb_shadow: CssRgba,
}

pub const DARK_TRAY_INTERNAL: TrayInternalPalette = TrayInternalPalette {
    border_default_2: 0x3e485b,
    border_strong: 0x55627a,
    atmosphere_wood_bg: 0x120a05,
    atmosphere_wood_glow_a: css_rgba_milli(0x653d1e, 180),
    atmosphere_wood_glow_b: css_rgba_milli(0x3c220f, 180),
    atmosphere_parchment_bg: 0x141210,
    atmosphere_parchment_glow_a: css_rgba_milli(0x8c693c, 120),
    atmosphere_parchment_glow_b: css_rgba_milli(0x5f4828, 140),
    atmosphere_terminal_bg: 0x030a06,
    atmosphere_terminal_glow: css_rgba_milli(DARK_TRAY_RAMP.green_500, 80),
    atmosphere_spacecraft_bg: 0x080a14,
    atmosphere_spacecraft_glow_a: css_rgba_milli(0xff69b4, 100),
    atmosphere_spacecraft_glow_b: css_rgba_milli(0x783cb4, 120),
    minimap_inactive_fill: css_rgba_milli(DARK_REFERENCE.white, 50),
    minimap_inactive_stroke: css_rgba_milli(DARK_REFERENCE.white, 150),
    minimap_inactive_text: css_rgba_milli(DARK_REFERENCE.white, 450),
    minimap_active_fill: css_rgba_milli(DARK_REFERENCE.white, 220),
    minimap_active_stroke: css_rgba_milli(DARK_REFERENCE.white, 850),
    minimap_active_text: css_rgba_milli(DARK_REFERENCE.white, 980),
    dissolve_target: DARK_TRAY_RAMP.blue_500,
    transparent_rgba: css_rgba_milli(DARK_REFERENCE.black, 0),
    config_field_thumb_bg: DARK_REFERENCE.white,
    config_qr_dark: DARK_REFERENCE.black,
    config_qr_light: DARK_REFERENCE.white,
    config_live_color_fallback: DARK_REFERENCE.white,
    config_color_thumb_stroke: DARK_REFERENCE.white,
    config_color_thumb_shadow: css_rgba_milli(DARK_REFERENCE.black, 500),
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ComponentPalettes {
    pub cli_sessions: CliSessionsPalette,
    pub launcher: LauncherPalette,
    pub remove_app: RemoveAppPalette,
    pub shot_selector: ShotSelectorPalette,
    pub shot_preview: ShotPreviewPalette,
    pub toast: ToastPalette,
    pub settings_panel: SettingsPanelPalette,
    pub alt_tab_preview_plane: AltTabPreviewPlanePalette,
    pub picker_surface: PickerSurfacePalette,
}

impl ComponentPalettes {
    pub fn new(mode: ThemeMode, reference: ReferencePalette, system: SystemPalette) -> Self {
        Self {
            cli_sessions: CliSessionsPalette::from_system(system),
            launcher: LauncherPalette::from_system(system),
            remove_app: RemoveAppPalette::from_system(system),
            shot_selector: ShotSelectorPalette::from_theme(reference, system),
            shot_preview: ShotPreviewPalette::from_system(system),
            toast: ToastPalette::from_system(system),
            settings_panel: SettingsPanelPalette::from_theme(mode, system),
            alt_tab_preview_plane: AltTabPreviewPlanePalette::from_theme(reference, system),
            picker_surface: PickerSurfacePalette::themed(system, None, 1.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CliSessionsPalette {
    pub panel_bg: u32,
    pub chrome_bg: u32,
    pub border: u32,
    pub divider: u32,
    pub text_primary: u32,
    pub text_heading: u32,
    pub text_secondary: u32,
    pub text_muted: u32,
    pub text_faint: u32,
    pub keycap_bg_rgba: u32,
    pub selection_border: u32,
    pub selection_bg: u32,
    pub needs_you: u32,
    pub your_turn: u32,
    pub working: u32,
    pub service: u32,
    pub bridged: u32,
    pub unknown: u32,
    pub needs_you_tint_rgba: u32,
    pub your_turn_tint_rgba: u32,
    pub your_turn_badge_rgba: u32,
    pub your_turn_hover_rgba: u32,
    pub working_tint_rgba: u32,
    pub service_tint_rgba: u32,
    pub bridged_tint_rgba: u32,
    pub bridged_badge_rgba: u32,
    pub bridged_hover_rgba: u32,
    pub transparent_rgba: u32,
}

impl CliSessionsPalette {
    pub fn from_system(system: SystemPalette) -> Self {
        Self {
            panel_bg: system.surface_elevated,
            chrome_bg: system.surface_canvas,
            border: system.border_subtle,
            divider: mix_rgb(system.surface_elevated, system.border_subtle, 0.5),
            text_primary: system.text_primary,
            text_heading: system.text_secondary,
            text_secondary: system.text_muted,
            text_muted: system.text_muted,
            text_faint: system.text_faint,
            keycap_bg_rgba: with_alpha(system.text_primary, 0x14),
            selection_border: system.accent,
            selection_bg: system.accent_fill,
            needs_you: system.danger,
            your_turn: system.warning,
            working: system.success,
            service: system.info,
            bridged: system.accent_ink,
            unknown: system.text_faint,
            needs_you_tint_rgba: with_alpha(system.danger, 0x22),
            your_turn_tint_rgba: with_alpha(system.warning, 0x22),
            your_turn_badge_rgba: with_alpha(system.warning, 0x33),
            your_turn_hover_rgba: with_alpha(system.warning, 0x55),
            working_tint_rgba: with_alpha(system.success, 0x1e),
            service_tint_rgba: with_alpha(system.info, 0x14),
            bridged_tint_rgba: with_alpha(system.accent, 0x1e),
            bridged_badge_rgba: with_alpha(system.accent, 0x33),
            bridged_hover_rgba: with_alpha(system.accent, 0x55),
            transparent_rgba: 0x00000000,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoveAppPalette {
    pub panel_bg: u32,
    pub chrome_bg: u32,
    pub border: u32,
    pub border_strong: u32,
    pub text_primary: u32,
    pub text_heading: u32,
    pub text_secondary: u32,
    pub text_muted: u32,
    pub accent: u32,
    pub success: u32,
    pub danger: u32,
    pub warning: u32,
    pub selection_bg_rgba: u32,
    pub transparent_rgba: u32,
    pub keycap_bg_rgba: u32,
    pub warning_banner_rgba: u32,
}

impl RemoveAppPalette {
    pub fn from_system(system: SystemPalette) -> Self {
        Self {
            panel_bg: system.surface_elevated,
            chrome_bg: system.surface_canvas,
            border: mix_rgb(system.surface_elevated, system.border_subtle, 0.5),
            border_strong: system.border_subtle,
            text_primary: system.text_primary,
            text_heading: system.text_secondary,
            text_secondary: system.text_muted,
            text_muted: system.text_faint,
            accent: system.accent,
            success: system.success,
            danger: system.danger,
            warning: system.warning,
            selection_bg_rgba: with_alpha(system.accent, 0x14),
            transparent_rgba: 0x00000000,
            keycap_bg_rgba: with_alpha(system.text_primary, 0x14),
            warning_banner_rgba: with_alpha(system.warning, 0x1a),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShotSelectorPalette {
    pub backdrop_rgba: u32,
    pub panel_bg_rgba: u32,
    pub panel_border_rgba: u32,
    pub text_primary: u32,
    pub text_subtitle_rgba: u32,
    pub label_text_rgba: u32,
    pub selection_outer: u32,
    pub selection_inner: u32,
    pub chip_ok_border_rgba: u32,
    pub chip_ok_text_rgba: u32,
    pub chip_low_border_rgba: u32,
    pub chip_low_text_rgba: u32,
    pub chip_critical_border_rgba: u32,
    pub chip_critical_text_rgba: u32,
}

impl ShotSelectorPalette {
    pub fn from_theme(reference: ReferencePalette, system: SystemPalette) -> Self {
        Self {
            backdrop_rgba: with_alpha(system.info, 0x24),
            panel_bg_rgba: with_alpha(reference.black, 0xc7),
            panel_border_rgba: with_alpha(reference.white, 0xdb),
            text_primary: reference.white,
            text_subtitle_rgba: with_alpha(reference.white, 0xc7),
            label_text_rgba: with_alpha(reference.white, 0xf5),
            selection_outer: reference.white,
            selection_inner: system.danger,
            chip_ok_border_rgba: with_alpha(reference.white, 0xdb),
            chip_ok_text_rgba: with_alpha(reference.white, 0xff),
            chip_low_border_rgba: with_alpha(system.warning, 0xff),
            chip_low_text_rgba: with_alpha(mix_rgb(system.warning, reference.white, 0.35), 0xff),
            chip_critical_border_rgba: with_alpha(system.danger, 0xff),
            chip_critical_text_rgba: with_alpha(
                mix_rgb(system.danger, reference.white, 0.35),
                0xff,
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShotPreviewPalette {
    pub window_bg: u32,
    pub thumb_border: u32,
    pub label_text: u32,
    pub action_glyph: u32,
    pub action_bg: u32,
    pub action_bg_selected: u32,
    pub action_border: u32,
    pub action_border_selected: u32,
    pub state_on: u32,
    pub state_off: u32,
}

impl ShotPreviewPalette {
    pub fn from_system(system: SystemPalette) -> Self {
        Self {
            window_bg: system.surface_elevated,
            thumb_border: system.border_subtle,
            label_text: system.text_secondary,
            action_glyph: system.text_primary,
            action_bg: system.surface_raised,
            action_bg_selected: mix_rgb(system.surface_raised, system.accent, 0.28),
            action_border: system.border_subtle,
            action_border_selected: system.accent,
            state_on: system.success,
            state_off: system.danger,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToastPalette {
    pub window_bg: u32,
    pub border: u32,
    pub text_primary: u32,
    pub text_secondary: u32,
    pub info: u32,
    pub success: u32,
    pub warning: u32,
    pub danger: u32,
}

impl ToastPalette {
    pub fn from_system(system: SystemPalette) -> Self {
        Self {
            window_bg: system.surface_elevated,
            border: system.border_subtle,
            text_primary: system.text_primary,
            text_secondary: system.text_secondary,
            info: system.info,
            success: system.success,
            warning: system.warning,
            danger: system.danger,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettingsPanelPalette {
    pub window_bg: u32,
    pub panel_border: u32,
    pub label_text: u32,
    pub section_text: u32,
    pub row_bg_selected: u32,
    pub row_border_selected: u32,
    pub rail_bg: u32,
    pub rail_text: u32,
    pub rail_text_muted: u32,
    pub rail_active_text: u32,
    pub dropdown_bg: u32,
    pub state_on: u32,
    pub state_off: u32,
    pub status_accent: u32,
    pub status_success: u32,
    pub status_danger: u32,
    pub status_warning: u32,
    pub status_muted: u32,
    pub qr_dark: u32,
    pub qr_light: u32,
    pub live_color_fallback: u32,
    pub transparent_rgba: u32,
}

impl SettingsPanelPalette {
    pub fn from_theme(mode: ThemeMode, system: SystemPalette) -> Self {
        let (rail_bg, rail_text, rail_text_muted) = match mode {
            ThemeMode::Light => (
                system.text_primary,
                system.surface_elevated,
                system.border_subtle,
            ),
            ThemeMode::Dark => (
                system.surface_canvas,
                system.text_primary,
                system.text_muted,
            ),
        };
        Self {
            window_bg: system.surface_elevated,
            panel_border: system.border_subtle,
            label_text: system.text_secondary,
            section_text: system.text_primary,
            row_bg_selected: system.accent_fill,
            row_border_selected: system.accent,
            rail_bg,
            rail_text,
            rail_text_muted,
            rail_active_text: system.surface_raised,
            dropdown_bg: system.surface_raised,
            state_on: system.success,
            state_off: system.danger,
            status_accent: system.accent_ink,
            status_success: system.success,
            status_danger: system.danger,
            status_warning: system.warning,
            status_muted: system.text_muted,
            qr_dark: DARK_TRAY_INTERNAL.config_qr_dark,
            qr_light: DARK_TRAY_INTERNAL.config_qr_light,
            live_color_fallback: DARK_TRAY_INTERNAL.config_live_color_fallback,
            transparent_rgba: 0x00000000,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AltTabPreviewPlanePalette {
    pub backdrop_rgba: u32,
    pub label_text: u32,
    pub card_bg_rgba: u32,
    pub card_border_rgba: u32,
    pub card_selected_bg_rgba: u32,
    pub card_selected_border_rgba: u32,
}

impl AltTabPreviewPlanePalette {
    pub fn from_theme(reference: ReferencePalette, system: SystemPalette) -> Self {
        Self {
            backdrop_rgba: with_alpha(reference.black, 0x1c),
            label_text: system.text_primary,
            card_bg_rgba: with_alpha(system.surface_elevated, 0xc8),
            card_border_rgba: with_alpha(system.text_secondary, 0xb4),
            card_selected_bg_rgba: with_alpha(
                mix_rgb(system.surface_raised, system.accent, 0.28),
                0xd2,
            ),
            card_selected_border_rgba: with_alpha(
                mix_rgb(system.accent, system.text_primary, 0.3),
                0xff,
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LauncherPalette {
    pub bg: u32,
    pub bg_selected: u32,
    pub bg_trail_hot: u32,
    pub bg_trail: u32,
    pub bg_near: u32,
    pub bg_edge: u32,
    pub bg_badge: u32,
    pub text: u32,
    pub text_selected: u32,
    pub text_dim: u32,
    pub text_muted: u32,
    pub text_faint: u32,
    pub highlight: u32,
    pub highlight_warm: u32,
    pub highlight_hot: u32,
    pub highlight_cool: u32,
    pub border: u32,
    pub border_selected: u32,
    pub momentum_up: [u32; 5],
    pub momentum_down: [u32; 5],
    pub compass_up: [u32; 3],
    pub compass_down: [u32; 3],
    pub semantic_prefix: u32,
    pub semantic_contains: u32,
    pub semantic_fuzzy: u32,
    pub semantic_freq: u32,
    pub boost_bg: u32,
}

impl LauncherPalette {
    pub fn from_system(system: SystemPalette) -> Self {
        Self {
            bg: system.surface_elevated,
            bg_selected: system.accent_fill,
            bg_trail_hot: mix_rgb(system.surface_raised, system.accent, 0.14),
            bg_trail: mix_rgb(system.surface_raised, system.accent, 0.09),
            bg_near: system.surface_hovered,
            bg_edge: mix_rgb(system.surface_elevated, system.surface_raised, 0.5),
            bg_badge: system.surface_raised,
            text: system.text_secondary,
            text_selected: system.text_primary,
            text_dim: system.text_muted,
            text_muted: system.text_faint,
            text_faint: mix_rgb(system.text_faint, system.surface_canvas, 0.24),
            highlight: system.accent_ink,
            highlight_warm: mix_rgb(system.accent_ink, system.warning, 0.36),
            highlight_hot: mix_rgb(system.accent_ink, system.text_primary, 0.22),
            highlight_cool: mix_rgb(system.accent_ink, system.info, 0.28),
            border: system.border_subtle,
            border_selected: system.accent,
            momentum_up: ramp(system.surface_raised, system.info),
            momentum_down: ramp(system.surface_raised, system.danger),
            compass_up: [
                mix_rgb(system.text_muted, system.info, 0.2),
                mix_rgb(system.text_muted, system.info, 0.45),
                mix_rgb(system.text_muted, system.info, 0.75),
            ],
            compass_down: [
                mix_rgb(system.text_muted, system.danger, 0.2),
                mix_rgb(system.text_muted, system.danger, 0.45),
                mix_rgb(system.text_muted, system.danger, 0.75),
            ],
            semantic_prefix: system.text_muted,
            semantic_contains: system.text_muted,
            semantic_fuzzy: system.text_muted,
            semantic_freq: system.text_muted,
            boost_bg: mix_rgb(system.surface_raised, system.surface_hovered, 0.5),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PickerSurfacePalette {
    pub panel_bg: u32,
    pub header_bg: u32,
    pub header_border: u32,
    pub card_bg: u32,
    pub card_hover_bg: u32,
    pub card_selected_bg: u32,
    pub card_selected_border: u32,
    pub card_bg_rgba: u32,
    pub card_selected_rgba: u32,
    pub caption_divider: u32,
    pub preview_icon_border: u32,
    pub preview_icon_selected_border: u32,
    pub header_left_text: u32,
    pub header_right_text: u32,
    pub grid_empty_text: u32,
    pub label_text: u32,
    pub label_selected_text: u32,
    pub placeholder_text: u32,
    pub placeholder_bg: u32,
    pub placeholder_border: u32,
}

impl PickerSurfacePalette {
    pub fn themed(system: SystemPalette, card_override: Option<u32>, opacity: f32) -> Self {
        let card_bg = card_override.unwrap_or(system.surface_raised);
        let opacity = clamp_unit(opacity);
        let (card_hover_bg, card_selected_bg, card_selected_border) = match card_override {
            Some(_) => (
                mix_rgb(card_bg, system.text_primary, 0.07),
                mix_rgb(card_bg, system.text_primary, 0.13),
                mix_rgb(card_bg, system.text_primary, 0.36),
            ),
            None => (system.surface_hovered, system.accent_fill, system.accent),
        };
        Self {
            panel_bg: mix_rgb(card_bg, system.surface_canvas, 0.56),
            header_bg: mix_rgb(card_bg, system.surface_canvas, 0.35),
            header_border: mix_rgb(card_bg, system.text_primary, 0.08),
            card_bg,
            card_hover_bg,
            card_selected_bg,
            card_selected_border,
            card_bg_rgba: rgba_from_rgb(card_bg, opacity),
            card_selected_rgba: rgba_from_rgb(card_selected_bg, opacity.max(0.92)),
            caption_divider: rgba_from_rgb(mix_rgb(card_bg, system.text_primary, 0.12), 0.58),
            preview_icon_border: rgba_from_rgb(mix_rgb(card_bg, system.text_primary, 0.12), 0.48),
            preview_icon_selected_border: rgba_from_rgb(
                mix_rgb(card_bg, system.text_primary, 0.18),
                0.52,
            ),
            header_left_text: system.text_muted,
            header_right_text: system.text_secondary,
            grid_empty_text: system.text_muted,
            label_text: system.text_secondary,
            label_selected_text: system.text_primary,
            placeholder_text: system.text_faint,
            placeholder_bg: system.surface_elevated,
            placeholder_border: system.border_subtle,
        }
    }
}

pub fn launcher_runtime() -> LauncherPalette {
    runtime_theme().components.launcher
}

pub fn cli_sessions_runtime() -> CliSessionsPalette {
    runtime_theme().components.cli_sessions
}

pub fn remove_app_runtime() -> RemoveAppPalette {
    runtime_theme().components.remove_app
}

pub fn shot_selector_runtime() -> ShotSelectorPalette {
    runtime_theme().components.shot_selector
}

pub fn shot_preview_runtime() -> ShotPreviewPalette {
    runtime_theme().components.shot_preview
}

pub fn toast_runtime() -> ToastPalette {
    runtime_theme().components.toast
}

pub fn settings_panel_runtime() -> SettingsPanelPalette {
    runtime_theme().components.settings_panel
}

pub fn alt_tab_preview_plane_runtime() -> AltTabPreviewPlanePalette {
    runtime_theme().components.alt_tab_preview_plane
}

pub fn picker_surface_runtime() -> PickerSurfacePalette {
    runtime_theme().components.picker_surface
}

pub fn resolve_surface_override(
    color_hex: &str,
    brightness: f32,
    opacity: f32,
) -> Option<(u32, f32)> {
    let trimmed = color_hex.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case(THEME_COLOR_SENTINEL) {
        return None;
    }
    let color = parse_rgb24(trimmed)?;
    Some((scale_rgb(color, brightness), clamp_unit(opacity)))
}

fn parse_rgb24(hex: &str) -> Option<u32> {
    let (red, green, blue) = parse_hex_color(hex)?;
    Some(rgb24(red, green, blue))
}

fn ramp(base: u32, target: u32) -> [u32; 5] {
    [
        mix_rgb(base, target, 0.08),
        mix_rgb(base, target, 0.14),
        mix_rgb(base, target, 0.2),
        mix_rgb(base, target, 0.26),
        mix_rgb(base, target, 0.32),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slate_native_key_yields_dark_canvas() {
        let theme = theme_for_native_key(Some("slate"), None);
        assert_eq!(theme.system.surface_canvas, DARK_SYSTEM.surface_canvas);
    }

    #[test]
    fn missing_unknown_or_bone_native_key_yields_light_canvas() {
        for native in [None, Some("bone"), Some("garbage")] {
            let theme = theme_for_native_key(native, None);
            assert_eq!(theme.system.surface_canvas, LIGHT_SYSTEM.surface_canvas);
        }
    }

    #[test]
    fn accent_key_changes_accent_on_dark_path() {
        let plain = theme_for_native_key(Some("slate"), None);
        let accented = theme_for_native_key(Some("slate"), Some("blue"));
        assert_eq!(
            accented.system.accent,
            dark_accent_preset("blue").unwrap().rgb
        );
        assert_ne!(accented.system.accent, plain.system.accent);
    }

    #[test]
    fn accent_key_changes_accent_on_light_path() {
        let plain = theme_for_native_key(None, None);
        let accented = theme_for_native_key(None, Some("blue"));
        assert_eq!(
            accented.system.accent,
            light_accent_preset("blue").unwrap().rgb
        );
        assert_ne!(accented.system.accent, plain.system.accent);
    }

    #[test]
    fn runtime_theme_override_switches_theme_and_accent() {
        set_runtime_theme_override(Some("slate"), Some("blue"));
        let theme = runtime_theme();
        assert_eq!(theme.system.surface_canvas, DARK_SYSTEM.surface_canvas);
        assert_eq!(theme.system.accent, dark_accent_preset("blue").unwrap().rgb);
        set_runtime_theme_override(Some("bone"), None);
        let theme = runtime_theme();
        assert_eq!(theme.system.surface_canvas, LIGHT_SYSTEM.surface_canvas);
    }
}
