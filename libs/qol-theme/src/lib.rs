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
}

pub const PROD_ACCENT_KEY: &str = "amber";
pub const DEV_ACCENT_KEY: &str = "green";

pub const DARK_ACCENT_PRESETS: [AccentPreset; 5] = [
    AccentPreset {
        key: "amber",
        label: "Amber",
        rgb: DARK_REFERENCE.orange_400,
        hover: 0xffc77a,
    },
    AccentPreset {
        key: "green",
        label: "Green",
        rgb: 0x46e08a,
        hover: 0x7ff0ab,
    },
    AccentPreset {
        key: "cyan",
        label: "Cyan",
        rgb: 0x56d6e0,
        hover: 0x8fe8f0,
    },
    AccentPreset {
        key: "magenta",
        label: "Magenta",
        rgb: 0xe879c6,
        hover: 0xf49ad6,
    },
    AccentPreset {
        key: "blue",
        label: "Blue",
        rgb: DARK_TRAY_RAMP.blue_500,
        hover: 0x68b0ff,
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
        let components = ComponentPalettes::new(reference, system);
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
    pub success: u32,
    pub danger: u32,
    pub info: u32,
    pub warning: u32,
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
            success: reference.green_400,
            danger: reference.red_500,
            info: reference.blue_400,
            warning: reference.amber_500,
        }
    }
}

pub const DARK_SYSTEM: SystemPalette = SystemPalette::from_reference(DARK_REFERENCE);

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
pub struct ComponentPalettes {
    pub cli_sessions: CliSessionsPalette,
    pub launcher: LauncherPalette,
    pub remove_app: RemoveAppPalette,
    pub shot_selector: ShotSelectorPalette,
    pub shot_preview: ShotPreviewPalette,
    pub alt_tab_preview_plane: AltTabPreviewPlanePalette,
}

impl ComponentPalettes {
    pub fn new(reference: ReferencePalette, system: SystemPalette) -> Self {
        Self {
            cli_sessions: CliSessionsPalette::from_theme(reference, system),
            launcher: LauncherPalette::from_system(system),
            remove_app: RemoveAppPalette::from_theme(reference, system),
            shot_selector: ShotSelectorPalette::dark(),
            shot_preview: ShotPreviewPalette::dark(),
            alt_tab_preview_plane: AltTabPreviewPlanePalette::dark(),
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
    pub needs_you: u32,
    pub your_turn: u32,
    pub working: u32,
    pub service: u32,
    pub unknown: u32,
    pub needs_you_tint_rgba: u32,
    pub your_turn_tint_rgba: u32,
    pub your_turn_badge_rgba: u32,
    pub your_turn_hover_rgba: u32,
    pub working_tint_rgba: u32,
    pub service_tint_rgba: u32,
    pub transparent_rgba: u32,
    pub claude: u32,
    pub codex: u32,
}

impl CliSessionsPalette {
    pub fn from_theme(reference: ReferencePalette, system: SystemPalette) -> Self {
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
            keycap_bg_rgba: with_alpha(reference.white, 0x0f),
            selection_border: system.accent,
            needs_you: system.danger,
            your_turn: system.warning,
            working: system.success,
            service: system.info,
            unknown: system.text_faint,
            needs_you_tint_rgba: with_alpha(system.danger, 0x22),
            your_turn_tint_rgba: with_alpha(system.warning, 0x22),
            your_turn_badge_rgba: with_alpha(system.warning, 0x33),
            your_turn_hover_rgba: with_alpha(system.warning, 0x55),
            working_tint_rgba: with_alpha(system.success, 0x1e),
            service_tint_rgba: with_alpha(system.info, 0x14),
            transparent_rgba: 0x00000000,
            claude: 0xd97757,
            codex: 0x10a37f,
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
    pub fn from_theme(reference: ReferencePalette, system: SystemPalette) -> Self {
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
            keycap_bg_rgba: with_alpha(reference.white, 0x0f),
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
    pub const fn dark() -> Self {
        Self {
            backdrop_rgba: 0x2f80ed24,
            panel_bg_rgba: 0x000000c7,
            panel_border_rgba: 0xffffffdb,
            text_primary: 0xffffff,
            text_subtitle_rgba: 0xffffffc7,
            label_text_rgba: 0xfffffff5,
            selection_outer: 0xffffff,
            selection_inner: 0xff4d4d,
            chip_ok_border_rgba: 0xffffffdb,
            chip_ok_text_rgba: 0xffffffff,
            chip_low_border_rgba: 0xf5a623ff,
            chip_low_text_rgba: 0xf7c66bff,
            chip_critical_border_rgba: 0xff4d4dff,
            chip_critical_text_rgba: 0xff9a9aff,
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
}

impl ShotPreviewPalette {
    pub const fn dark() -> Self {
        Self {
            window_bg: 0x14141c,
            thumb_border: 0x2a2a3a,
            label_text: 0xc8c8e0,
            action_glyph: 0xe8e8f4,
            action_bg: 0x1d1d28,
            action_bg_selected: 0x2a2a52,
            action_border: 0x33333f,
            action_border_selected: 0x8a8aff,
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
    pub const fn dark() -> Self {
        Self {
            backdrop_rgba: 0x0000001c,
            label_text: 0xedf3ff,
            card_bg_rgba: 0x14181ec8,
            card_border_rgba: 0xa0aabeb4,
            card_selected_bg_rgba: 0x28374ed2,
            card_selected_border_rgba: 0xb4d7ffff,
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
            bg_selected: mix_rgb(system.surface_raised, system.accent, 0.28),
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
            highlight: system.accent,
            highlight_warm: mix_rgb(system.accent, system.warning, 0.36),
            highlight_hot: mix_rgb(system.accent, system.text_primary, 0.22),
            highlight_cool: mix_rgb(system.accent, system.info, 0.28),
            border: system.border_subtle,
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
            semantic_prefix: system.info,
            semantic_contains: mix_rgb(system.info, system.accent, 0.45),
            semantic_fuzzy: system.text_muted,
            semantic_freq: mix_rgb(system.accent, system.danger, 0.45),
            boost_bg: mix_rgb(system.surface_raised, system.success, 0.22),
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
    pub fn from_card_color(card_bg: u32, opacity: f32) -> Self {
        Self::from_card_color_with_reference(card_bg, opacity, DARK_REFERENCE)
    }

    pub fn from_card_color_with_reference(
        card_bg: u32,
        opacity: f32,
        reference: ReferencePalette,
    ) -> Self {
        let opacity = clamp_unit(opacity);
        let selected_bg = mix_rgb(card_bg, reference.white, 0.13);
        Self {
            panel_bg: mix_rgb(card_bg, reference.black, 0.56),
            header_bg: mix_rgb(card_bg, reference.black, 0.35),
            header_border: mix_rgb(card_bg, reference.white, 0.08),
            card_bg,
            card_hover_bg: mix_rgb(card_bg, reference.white, 0.07),
            card_selected_bg: selected_bg,
            card_selected_border: mix_rgb(card_bg, reference.warm_slate_300, 0.36),
            card_bg_rgba: rgba_from_rgb(card_bg, opacity),
            card_selected_rgba: rgba_from_rgb(selected_bg, opacity.max(0.92)),
            caption_divider: rgba_from_rgb(mix_rgb(card_bg, reference.white, 0.12), 0.58),
            preview_icon_border: rgba_from_rgb(mix_rgb(card_bg, reference.white, 0.12), 0.48),
            preview_icon_selected_border: rgba_from_rgb(
                mix_rgb(card_bg, reference.white, 0.18),
                0.52,
            ),
            header_left_text: reference.slate_550,
            header_right_text: reference.slate_700,
            grid_empty_text: reference.slate_550,
            label_text: reference.slate_200,
            label_selected_text: reference.slate_050,
            placeholder_text: reference.slate_650,
            placeholder_bg: reference.night_800,
            placeholder_border: reference.slate_700,
        }
    }
}

pub fn launcher_dark() -> LauncherPalette {
    dark_theme().components.launcher
}

pub fn cli_sessions_dark() -> CliSessionsPalette {
    dark_theme().components.cli_sessions
}

pub fn remove_app_dark() -> RemoveAppPalette {
    dark_theme().components.remove_app
}

pub fn shot_selector_dark() -> ShotSelectorPalette {
    dark_theme().components.shot_selector
}

pub fn shot_preview_dark() -> ShotPreviewPalette {
    dark_theme().components.shot_preview
}

pub fn alt_tab_preview_plane_dark() -> AltTabPreviewPlanePalette {
    dark_theme().components.alt_tab_preview_plane
}

pub fn resolve_surface_color(
    color_hex: &str,
    fallback_hex: &str,
    brightness: f32,
    opacity: f32,
) -> (u32, f32) {
    let fallback = parse_rgb24(fallback_hex).unwrap_or(DARK_SYSTEM.surface_raised);
    let color = parse_rgb24(color_hex).unwrap_or(fallback);
    (scale_rgb(color, brightness), clamp_unit(opacity))
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
