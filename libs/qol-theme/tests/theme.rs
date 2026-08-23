use qol_color::{mix_rgb, rgba_from_rgb, with_alpha};
use qol_theme::{
    alt_tab_preview_plane_runtime, css, css_rgba_milli, dark_accent_preset, dark_theme,
    dark_theme_with_accent_key, resolve_surface_override, runtime_dark_theme, PickerSurfacePalette,
    SettingsPanelPalette, ThemeMode, WashPalette, DARK_ACCENT_PRESETS, DARK_REFERENCE, DARK_SYSTEM,
    DARK_TRAY_INTERNAL, HEIGHT_BAND, HEIGHT_CONTROL, HEIGHT_HINT_BAR, HEIGHT_INLINE, HEIGHT_LADDER,
    HEIGHT_RULE_ROW, HEIGHT_SETTING_ROW, LIGHT_ACCENT_PRESETS, LIGHT_REFERENCE, LIGHT_SYSTEM,
    LIST_ENTRY_HEIGHTS, PROD_ACCENT_KEY, RADIUS_LADDER, SPACE_GUTTER, TEXT_SCALE,
    THEME_COLOR_SENTINEL,
};
use std::{
    fs,
    path::Path,
    sync::{Mutex, OnceLock},
};

#[test]
fn dark_theme_has_explicit_reference_system_and_component_layers() {
    let theme = dark_theme();
    assert_eq!(theme.mode, ThemeMode::Dark);
    assert_eq!(theme.reference, DARK_REFERENCE);
    assert_eq!(theme.system, DARK_SYSTEM);
    assert_eq!(
        theme.system.accent,
        dark_accent_preset(PROD_ACCENT_KEY).unwrap().rgb
    );
}

#[test]
fn runtime_dark_theme_uses_valid_injected_accent_key() {
    let _env = ThemeAccentEnvGuard::set("blue");

    let theme = runtime_dark_theme();
    let expected = dark_accent_preset("blue").unwrap().rgb;

    assert_eq!(theme.system.accent, expected);
    assert_eq!(theme.components.launcher.highlight, expected);
    assert_eq!(theme.components.cli_sessions.selection_border, expected);
    assert_eq!(theme.components.remove_app.accent, expected);
}

#[test]
fn runtime_dark_theme_falls_back_for_unknown_injected_accent_key() {
    let _env = ThemeAccentEnvGuard::set("not-a-preset");

    let theme = runtime_dark_theme();

    assert_eq!(theme.system.accent, DARK_SYSTEM.accent);
}

#[test]
fn css_color_serializers_are_shared() {
    assert_eq!(css::rgb_string(0xffb454), "255, 180, 84");
    assert_eq!(css::hex_string(0xffc77a), "#ffc77a");
}

#[test]
fn launcher_palette_derives_from_system_roles() {
    let palette = dark_theme().components.launcher;
    assert_eq!(palette.bg, DARK_SYSTEM.surface_elevated);
    assert_eq!(palette.bg_badge, DARK_SYSTEM.surface_raised);
    assert_eq!(palette.text_selected, DARK_SYSTEM.text_primary);
    assert_eq!(palette.text, DARK_SYSTEM.text_secondary);
    assert_eq!(palette.highlight, DARK_SYSTEM.accent_ink);
    assert_eq!(palette.border, DARK_SYSTEM.border_subtle);
    assert_eq!(palette.momentum_up.len(), 5);
    assert_eq!(palette.momentum_down.len(), 5);
    assert_eq!(palette.compass_up.len(), 3);
    assert_eq!(palette.compass_down.len(), 3);
}

#[test]
fn cli_sessions_palette_derives_from_system_roles() {
    let palette = dark_theme().components.cli_sessions;
    assert_eq!(palette.panel_bg, DARK_SYSTEM.surface_elevated);
    assert_eq!(palette.chrome_bg, DARK_SYSTEM.surface_canvas);
    assert_eq!(palette.border, DARK_SYSTEM.border_subtle);
    assert_eq!(
        palette.divider,
        mix_rgb(DARK_SYSTEM.surface_elevated, DARK_SYSTEM.border_subtle, 0.5)
    );
    assert_eq!(palette.text_primary, DARK_SYSTEM.text_primary);
    assert_eq!(palette.text_heading, DARK_SYSTEM.text_secondary);
    assert_eq!(palette.text_secondary, DARK_SYSTEM.text_muted);
    assert_eq!(palette.text_muted, DARK_SYSTEM.text_muted);
    assert_eq!(palette.text_faint, DARK_SYSTEM.text_faint);
    assert_eq!(
        palette.keycap_bg_rgba,
        with_alpha(DARK_SYSTEM.text_primary, 0x14)
    );
    assert_eq!(palette.selection_border, DARK_SYSTEM.accent);
    assert_eq!(palette.needs_you, DARK_SYSTEM.danger);
    assert_eq!(palette.your_turn, DARK_SYSTEM.warning);
    assert_eq!(palette.working, DARK_SYSTEM.success);
    assert_eq!(palette.service, DARK_SYSTEM.info);
    assert_eq!(palette.unknown, DARK_SYSTEM.text_faint);
    assert_eq!(
        palette.needs_you_tint_rgba,
        with_alpha(DARK_SYSTEM.danger, 0x22)
    );
    assert_eq!(
        palette.your_turn_tint_rgba,
        with_alpha(DARK_SYSTEM.warning, 0x22)
    );
    assert_eq!(
        palette.your_turn_badge_rgba,
        with_alpha(DARK_SYSTEM.warning, 0x33)
    );
    assert_eq!(
        palette.your_turn_hover_rgba,
        with_alpha(DARK_SYSTEM.warning, 0x55)
    );
    assert_eq!(
        palette.working_tint_rgba,
        with_alpha(DARK_SYSTEM.success, 0x1e)
    );
    assert_eq!(
        palette.service_tint_rgba,
        with_alpha(DARK_SYSTEM.info, 0x14)
    );
    assert_eq!(palette.transparent_rgba, 0x00000000);
}

#[test]
fn remove_app_palette_derives_from_system_roles() {
    let palette = dark_theme().components.remove_app;
    assert_eq!(palette.panel_bg, DARK_SYSTEM.surface_elevated);
    assert_eq!(palette.chrome_bg, DARK_SYSTEM.surface_canvas);
    assert_eq!(
        palette.border,
        mix_rgb(DARK_SYSTEM.surface_elevated, DARK_SYSTEM.border_subtle, 0.5)
    );
    assert_eq!(palette.border_strong, DARK_SYSTEM.border_subtle);
    assert_eq!(palette.text_primary, DARK_SYSTEM.text_primary);
    assert_eq!(palette.text_heading, DARK_SYSTEM.text_secondary);
    assert_eq!(palette.text_secondary, DARK_SYSTEM.text_muted);
    assert_eq!(palette.text_muted, DARK_SYSTEM.text_faint);
    assert_eq!(palette.accent, DARK_SYSTEM.accent);
    assert_eq!(palette.success, DARK_SYSTEM.success);
    assert_eq!(palette.danger, DARK_SYSTEM.danger);
    assert_eq!(palette.warning, DARK_SYSTEM.warning);
    assert_eq!(
        palette.selection_bg_rgba,
        with_alpha(DARK_SYSTEM.accent, 0x14)
    );
    assert_eq!(palette.transparent_rgba, 0x00000000);
    assert_eq!(
        palette.keycap_bg_rgba,
        with_alpha(DARK_SYSTEM.text_primary, 0x14)
    );
    assert_eq!(
        palette.warning_banner_rgba,
        with_alpha(DARK_SYSTEM.warning, 0x1a)
    );
}

#[test]
fn shot_preview_palette_derives_from_system_roles() {
    let palette = dark_theme().components.shot_preview;
    assert_eq!(palette.window_bg, DARK_SYSTEM.surface_elevated);
    assert_eq!(palette.thumb_border, DARK_SYSTEM.border_subtle);
    assert_eq!(palette.label_text, DARK_SYSTEM.text_secondary);
    assert_eq!(palette.action_glyph, DARK_SYSTEM.text_primary);
    assert_eq!(palette.action_bg, DARK_SYSTEM.surface_raised);
    assert_eq!(
        palette.action_bg_selected,
        mix_rgb(DARK_SYSTEM.surface_raised, DARK_SYSTEM.accent, 0.28)
    );
    assert_eq!(palette.action_border, DARK_SYSTEM.border_subtle);
    assert_eq!(palette.action_border_selected, DARK_SYSTEM.accent);
}

#[test]
fn toast_palette_derives_from_system_roles() {
    let palette = dark_theme().components.toast;
    assert_eq!(palette.window_bg, DARK_SYSTEM.surface_elevated);
    assert_eq!(palette.border, DARK_SYSTEM.border_subtle);
    assert_eq!(palette.text_primary, DARK_SYSTEM.text_primary);
    assert_eq!(palette.text_secondary, DARK_SYSTEM.text_secondary);
    assert_eq!(palette.info, DARK_SYSTEM.info);
    assert_eq!(palette.success, DARK_SYSTEM.success);
    assert_eq!(palette.warning, DARK_SYSTEM.warning);
    assert_eq!(palette.danger, DARK_SYSTEM.danger);
}

#[test]
fn settings_panel_palette_derives_status_tones_from_system_roles() {
    let palette = dark_theme().components.settings_panel;
    assert_eq!(palette.status_accent, DARK_SYSTEM.accent_ink);
    assert_eq!(palette.status_success, DARK_SYSTEM.success);
    assert_eq!(palette.status_danger, DARK_SYSTEM.danger);
    assert_eq!(palette.status_warning, DARK_SYSTEM.warning);
    assert_eq!(palette.status_muted, DARK_SYSTEM.text_muted);
    assert_eq!(palette.transparent_rgba, 0x00000000);
    assert_eq!(palette.qr_dark, DARK_REFERENCE.black);
    assert_eq!(palette.qr_light, DARK_REFERENCE.white);
    assert_eq!(palette.live_color_fallback, DARK_REFERENCE.white);
}

#[test]
fn alt_tab_preview_plane_palette_derives_from_system_roles() {
    let palette = alt_tab_preview_plane_runtime();
    assert_eq!(
        palette.backdrop_rgba,
        with_alpha(LIGHT_REFERENCE.black, 0x1c)
    );
    assert_eq!(palette.label_text, LIGHT_SYSTEM.text_primary);
    assert_eq!(
        palette.card_bg_rgba,
        with_alpha(LIGHT_SYSTEM.surface_elevated, 0xc8)
    );
    assert_eq!(
        palette.card_border_rgba,
        with_alpha(LIGHT_SYSTEM.text_secondary, 0xb4)
    );
    assert_eq!(
        palette.card_selected_bg_rgba,
        with_alpha(
            mix_rgb(LIGHT_SYSTEM.surface_raised, LIGHT_SYSTEM.accent, 0.28),
            0xd2
        )
    );
    assert_eq!(
        palette.card_selected_border_rgba,
        with_alpha(
            mix_rgb(LIGHT_SYSTEM.accent, LIGHT_SYSTEM.text_primary, 0.3),
            0xff
        )
    );
}

#[test]
fn alt_tab_cinnamon_js_emits_clutter_color_strings() {
    assert_eq!(
        css::alt_tab_cinnamon_js(),
        concat!(
            "/* @generated by qol-theme-css; do not edit by hand. */\n",
            "module.exports = {\n",
            "    backdrop: \"rgba(0, 0, 0, 28)\",\n",
            "    labelText: \"#1a1815\",\n",
            "    cardBg: \"rgba(255, 254, 251, 200)\",\n",
            "    cardBorder: \"rgba(79, 75, 67, 180)\",\n",
            "    cardSelectedBg: \"rgba(232, 216, 178, 210)\",\n",
            "    cardSelectedBorder: \"rgba(137, 101, 14, 255)\",\n",
            "};\n",
        )
    );
}

#[test]
fn shot_selector_palette_derives_from_system_roles() {
    let palette = dark_theme().components.shot_selector;
    assert_eq!(palette.backdrop_rgba, with_alpha(DARK_SYSTEM.info, 0x24));
    assert_eq!(
        palette.panel_bg_rgba,
        with_alpha(DARK_REFERENCE.black, 0xc7)
    );
    assert_eq!(
        palette.panel_border_rgba,
        with_alpha(DARK_REFERENCE.white, 0xdb)
    );
    assert_eq!(palette.text_primary, DARK_REFERENCE.white);
    assert_eq!(
        palette.text_subtitle_rgba,
        with_alpha(DARK_REFERENCE.white, 0xc7)
    );
    assert_eq!(
        palette.label_text_rgba,
        with_alpha(DARK_REFERENCE.white, 0xf5)
    );
    assert_eq!(palette.selection_outer, DARK_REFERENCE.white);
    assert_eq!(palette.selection_inner, DARK_SYSTEM.danger);
    assert_eq!(
        palette.chip_ok_border_rgba,
        with_alpha(DARK_REFERENCE.white, 0xdb)
    );
    assert_eq!(
        palette.chip_ok_text_rgba,
        with_alpha(DARK_REFERENCE.white, 0xff)
    );
    assert_eq!(
        palette.chip_low_border_rgba,
        with_alpha(DARK_SYSTEM.warning, 0xff)
    );
    assert_eq!(
        palette.chip_low_text_rgba,
        with_alpha(
            mix_rgb(DARK_SYSTEM.warning, DARK_REFERENCE.white, 0.35),
            0xff
        )
    );
    assert_eq!(
        palette.chip_critical_border_rgba,
        with_alpha(DARK_SYSTEM.danger, 0xff)
    );
    assert_eq!(
        palette.chip_critical_text_rgba,
        with_alpha(
            mix_rgb(DARK_SYSTEM.danger, DARK_REFERENCE.white, 0.35),
            0xff
        )
    );
}

#[test]
fn picker_surface_palette_themed_none_uses_system_roles() {
    let system = DARK_SYSTEM;
    let palette = PickerSurfacePalette::themed(system, None, 1.0);
    assert_eq!(palette.card_bg, system.surface_raised);
    assert_eq!(palette.card_hover_bg, system.surface_hovered);
    assert_eq!(palette.card_selected_bg, system.accent_fill);
    assert_eq!(palette.card_selected_border, system.accent);
    assert_eq!(
        palette.panel_bg,
        mix_rgb(system.surface_raised, system.surface_canvas, 0.56)
    );
    assert_eq!(
        palette.header_bg,
        mix_rgb(system.surface_raised, system.surface_canvas, 0.35)
    );
    assert_eq!(palette.header_left_text, system.text_muted);
    assert_eq!(palette.header_right_text, system.text_secondary);
    assert_eq!(palette.label_text, system.text_secondary);
    assert_eq!(palette.placeholder_bg, system.surface_elevated);
    assert_eq!(palette.placeholder_border, system.border_subtle);
}

#[test]
fn picker_surface_palette_themed_override_mixes_toward_text_primary() {
    let system = DARK_SYSTEM;
    let card = 0x203040u32;
    let palette = PickerSurfacePalette::themed(system, Some(card), 0.85);
    assert_eq!(palette.card_bg, card);
    assert_eq!(
        palette.card_hover_bg,
        mix_rgb(card, system.text_primary, 0.07)
    );
    assert_eq!(
        palette.card_selected_bg,
        mix_rgb(card, system.text_primary, 0.13)
    );
    assert_eq!(
        palette.card_selected_border,
        mix_rgb(card, system.text_primary, 0.36)
    );
    assert_eq!(palette.card_bg_rgba, rgba_from_rgb(card, 0.85));
}

#[test]
fn resolve_surface_override_returns_none_for_theme_sentinel_and_invalid() {
    for input in ["theme", "THEME", "", "  ", "nope"] {
        assert_eq!(
            resolve_surface_override(input, 1.0, 1.0),
            None,
            "{input:?} must resolve to None"
        );
    }
    assert_eq!(THEME_COLOR_SENTINEL, "theme");
}

#[test]
fn resolve_surface_override_scales_rgb_and_clamps_unit() {
    let (color, opacity) = resolve_surface_override("#ff8040", 1.0, 1.0).unwrap();
    assert_eq!(color, 0xff8040);
    assert_eq!(opacity, 1.0);
    let (color, opacity) = resolve_surface_override("#ff8040", 0.25, 2.0).unwrap();
    assert_eq!(color, 0x402010);
    assert_eq!(opacity, 1.0);
}

#[test]
fn dark_css_emits_stable_token_names() {
    assert_eq!(
        css::dark_css(),
        concat!(
            "/* @generated by qol-theme-css; do not edit by hand. */\n",
            ":root {\n",
            "    --qol-system-accent-rgb: 224, 172, 63;\n",
            "    --qol-system-success-rgb: 74, 222, 128;\n",
            "    --qol-system-danger-rgb: 255, 107, 107;\n",
            "    --qol-system-warning-rgb: 255, 193, 7;\n",
            "    --qol-system-ink-rgb: 0, 0, 0;\n",
            "    --qol-system-paper-rgb: 255, 255, 255;\n",
            "    --qol-system-surface-canvas: #0e0f12;\n",
            "    --qol-system-surface-elevated: #16171a;\n",
            "    --qol-system-surface-raised: #1d1e22;\n",
            "    --qol-system-surface-hovered: #25262b;\n",
            "    --qol-system-text-primary: #f3f2f0;\n",
            "    --qol-system-text-secondary: #b3b1ac;\n",
            "    --qol-system-text-muted: #8b8880;\n",
            "    --qol-system-text-faint: #6f6c65;\n",
            "    --qol-system-border-subtle: #2a2b31;\n",
            "}\n",
        )
    );
}

#[test]
fn tray_css_layers_tray_tokens_without_polluting_core() {
    let core = css::dark_css();
    assert!(!core.contains("--qol-tray-"));
    assert!(!core.contains("--qol-accent-"));
    assert!(!core.contains("--qol-reference-"));
    assert!(!core.contains("--qol-system-overlay-"));

    let tray = css::tray_css();
    for line in core
        .lines()
        .filter(|line| line.contains("--qol-system-") && !line.contains("-surface-"))
    {
        assert!(tray.contains(line), "tray css must carry core token {line}");
    }
    assert!(
        tray.contains("    --qol-system-surface-canvas: #0b0d12;\n"),
        "tray css must use the retuned slate surfaces"
    );
    assert!(tray.contains("    --qol-system-overlay-surface-rgb: 18, 22, 30;\n"));
    assert!(tray.contains("    --qol-reference-slate-750: #2a2b31;\n"));
    assert!(tray.contains("    --qol-tray-blue-500: #4a9eff;\n"));
    assert!(tray.contains("    --qol-tray-border-default-2: #3e485b;\n"));
    assert!(tray.contains("    --qol-atmosphere-wood-bg: #120a05;\n"));
    assert!(tray.contains("    --qol-accent-amber-hover: #ffc77a;\n"));
}

#[test]
fn tray_theme_js_emits_accent_presets_from_theme() {
    let js = css::tray_theme_js();
    for preset in DARK_ACCENT_PRESETS {
        assert!(
            js.contains(&format!("key: \"{}\"", preset.key)),
            "tray JS must emit {} preset key",
            preset.key
        );
        assert!(
            js.contains(&format!("label: \"{}\"", preset.label)),
            "tray JS must emit {} preset label",
            preset.key
        );
    }
    assert!(js.contains(&format!("QOL_DEFAULT_ACCENT = \"{PROD_ACCENT_KEY}\"")));
    assert!(js.contains(&format!(
        "dissolveTargetColor: \"{}\"",
        hex6(DARK_TRAY_INTERNAL.dissolve_target)
    )));
    assert!(js.contains("minimapActiveText: \"rgba(255, 255, 255, 0.98)\""));
    assert!(js.contains("configColorThumbShadow: \"rgba(0, 0, 0, 0.5)\""));
}

#[test]
fn plugin_lights_css_emits_component_token_names() {
    let css = css::plugin_lights_css();
    assert!(css.contains("    --qol-system-success-rgb: 74, 222, 128;\n"));
    assert!(css.contains(&format!(
        "    --qol-lights-bg-start: {};\n",
        hex6(DARK_SYSTEM.surface_raised)
    )));
    assert!(css.contains(&format!(
        "    --qol-lights-warning: {};\n",
        hex6(DARK_SYSTEM.warning)
    )));
    assert!(css.contains(&format!(
        "    --qol-lights-accent-btn-shadow: rgba({}, 0.3);\n",
        rgb_triplet(DARK_SYSTEM.accent)
    )));
    assert!(css.contains("    --qol-lights-wheel-shadow: rgba(0, 0, 0, 0.5);\n"));
    assert!(css.contains("    --qol-lights-wheel-thumb-stroke: #ffffff;\n"));
    assert!(css.contains("    --qol-lights-wheel-brightness-floor: #0a0a0a;\n"));
}

#[test]
fn plugin_keyremap_css_emits_component_token_names() {
    let css = css::plugin_keyremap_css();
    assert!(css.contains("    --qol-system-success-rgb: 74, 222, 128;\n"));
    assert!(css.contains(&format!(
        "    --qol-keyremap-bg-start: {};\n",
        hex6(DARK_SYSTEM.surface_canvas)
    )));
    assert!(css.contains(&format!(
        "    --qol-keyremap-accent: {};\n",
        hex6(DARK_SYSTEM.accent)
    )));
    assert!(css.contains(&format!(
        "    --qol-keyremap-accent-bg: rgba({}, 0.3);\n",
        rgb_triplet(DARK_SYSTEM.accent)
    )));
    assert!(css.contains("    --qol-keyremap-shadow: rgba(0, 0, 0, 0.4);\n"));
}

#[test]
fn web_plugin_css_emits_diffed_accent_override_blocks() {
    let profiles = [
        (
            "plugin-lights",
            css::plugin_lights_css(),
            "--qol-lights-accent-btn",
        ),
        (
            "plugin-keyremap",
            css::plugin_keyremap_css(),
            "--qol-keyremap-accent",
        ),
    ];

    for preset in DARK_ACCENT_PRESETS {
        if preset.key == PROD_ACCENT_KEY {
            continue;
        }
        let theme = dark_theme_with_accent_key(preset.key);
        let expected_rgb = rgb_triplet(theme.system.accent);
        let expected_hex = hex6(theme.system.accent);

        for (profile, css, accent_token) in &profiles {
            let block = accent_block(css, preset.key)
                .unwrap_or_else(|| panic!("{profile} missing {} override block", preset.key));
            assert!(
                block.contains(&format!("    --qol-system-accent-rgb: {expected_rgb};\n")),
                "{profile} {} block must override system accent rgb",
                preset.key
            );
            assert!(
                block.contains(&format!("    {accent_token}: {expected_hex};\n")),
                "{profile} {} block must override component accent token",
                preset.key
            );
        }
    }
}

#[test]
fn web_plugin_css_skips_default_accent_override_block() {
    for (profile, css) in [
        ("plugin-lights", css::plugin_lights_css()),
        ("plugin-keyremap", css::plugin_keyremap_css()),
    ] {
        assert!(
            accent_block(&css, PROD_ACCENT_KEY).is_none(),
            "{profile} must not emit an override block for default accent"
        );
    }
}

#[test]
fn tray_theme_tokens_import_and_derive_generated_base() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let theme_tokens_path = workspace.join("apps/qol-tray/ui/styles/theme-tokens.css");
    let theme_tokens = fs::read_to_string(&theme_tokens_path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", theme_tokens_path.display()));
    assert!(
        theme_tokens.starts_with("@import \"./generated-theme-tokens.css\";\n\n:root {"),
        "theme-tokens.css must import generated-theme-tokens.css before declaring aliases"
    );

    let required_derivations = [
        "--accent-rgb: var(--qol-system-accent-rgb);",
        "--accent-hover: var(--qol-accent-amber-hover);",
        "--success-rgb: var(--qol-system-success-rgb);",
        "--danger-rgb: var(--qol-system-danger-rgb);",
        "--blue-400: var(--qol-reference-blue-400);",
        "--green-400: var(--qol-reference-green-400);",
        "--red-500: var(--qol-reference-red-500);",
        "--amber-500: var(--qol-reference-amber-500);",
        "--slate-750: var(--qol-reference-slate-750);",
        "--slate-975: var(--qol-tray-slate-975);",
        "--warning-rgb: var(--qol-system-warning-rgb);",
        "--surface-canvas: var(--qol-system-surface-canvas);",
        "--surface-elevated: var(--qol-system-surface-elevated);",
        "--surface-raised: var(--qol-system-surface-raised);",
        "--surface-hovered: var(--qol-system-surface-hovered);",
        "--text-strong: var(--qol-system-text-primary);",
        "--text-default: var(--qol-system-text-secondary);",
        "--text-muted-2: var(--qol-system-text-muted);",
        "--text-subtle: var(--qol-system-text-faint);",
        "--border-weak: var(--qol-system-border-subtle);",
        "--border-default-2: var(--qol-tray-border-default-2);",
        "--border-strong: var(--qol-tray-border-strong);",
        "--tui-bg-desktop: var(--qol-tray-tui-bg-desktop);",
        "--tui-bg-panel: var(--qol-tray-tui-bg-panel);",
        "--tui-bg-screen: var(--qol-tray-tui-bg-screen);",
        "--tui-bg-card: var(--qol-tray-tui-bg-card);",
    ];

    for derivation in required_derivations {
        assert!(
            theme_tokens.contains(derivation),
            "theme-tokens.css must derive `{derivation}` from generated theme tokens"
        );
    }

    assert!(
        !theme_tokens.contains("224, 172, 63"),
        "theme-tokens.css must not hand-copy the default accent rgb fallback"
    );
    assert!(
        !theme_tokens.contains("#ffc77a"),
        "theme-tokens.css must not hand-copy the default accent hover"
    );
    assert!(
        !theme_tokens.contains("#3e485b"),
        "theme-tokens.css must not hand-copy tray border colors"
    );
}

#[test]
fn themed_tray_internals_do_not_use_raw_color_literals() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let files = [
        "apps/qol-tray/ui/styles/theme-tokens.css",
        "apps/qol-tray/ui/fx/atmosphere/atmosphere.css",
        "apps/qol-tray/ui/fx/dissolve/engine.js",
        "apps/qol-tray/ui/fx/dissolve/gpu.js",
        "apps/qol-tray/ui/fx/dissolve/index.js",
        "apps/qol-tray/ui/fx/dissolve/worker.js",
        "apps/qol-tray/ui/lib/minimap-draw.js",
        "apps/qol-tray/ui/styles/plugin-config.css",
        "apps/qol-tray/ui/views/plugin-config/fields/QrCodeField.js",
        "apps/qol-tray/ui/views/plugin-config/fields/SliderField.js",
        "apps/qol-tray/ui/views/plugin-config/fields/ColorField.js",
    ];
    let mut violations = Vec::new();

    for file in files {
        let path = workspace.join(file);
        let contents = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        for (index, line) in contents.lines().enumerate() {
            if has_raw_css_color(line) {
                violations.push(format!("{file}:{}", index + 1));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "themed tray internals must use generated theme tokens, not raw color literals:\n{}",
        violations.join("\n")
    );
}

#[test]
fn generated_artifacts_are_current() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let artifacts = [
        (
            "apps/qol-tray/ui/styles/generated-theme-tokens.css",
            css::tray_css(),
        ),
        (
            "plugins/lights/ui/generated-theme-tokens.css",
            css::plugin_lights_css(),
        ),
        (
            "plugins/keyremap/ui/generated-theme-tokens.css",
            css::plugin_keyremap_css(),
        ),
        (
            "plugins/alt-tab/shell/cinnamon/qol-alt-tab-preview-plane@qol-tools/generated-theme-tokens.js",
            css::alt_tab_cinnamon_js(),
        ),
        (
            "apps/qol-tray/ui/lib/generated-theme-tokens.js",
            css::tray_theme_js(),
        ),
    ];

    for (artifact_path, expected) in artifacts {
        let path = workspace.join(artifact_path);
        let actual = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        assert_eq!(
            actual, expected,
            "{artifact_path} must be regenerated with qol-theme-css"
        );
    }
}

#[test]
fn plugin_css_profiles_exclude_tray_only_tokens() {
    for (profile, css) in [
        ("plugin-lights", css::plugin_lights_css()),
        ("plugin-keyremap", css::plugin_keyremap_css()),
    ] {
        assert!(
            !css.contains("--qol-tray-"),
            "{profile} generated CSS must not include tray ramp tokens"
        );
        assert!(
            !css.contains("--qol-accent-"),
            "{profile} generated CSS must not include tray accent preset tokens"
        );
        assert!(
            !css.contains("--qol-reference-"),
            "{profile} generated CSS must not include tray reference aliases"
        );
    }
}

#[test]
fn migrated_web_plugin_css_imports_generated_base() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let migrated_plugins = [
        (
            "plugin-lights",
            "plugins/lights/ui/style.css",
            Some("rgb(var(--qol-system-success-rgb))"),
            "var(--qol-lights-accent-btn-shadow)",
        ),
        (
            "plugin-keyremap",
            "plugins/keyremap/ui/style.css",
            None,
            "var(--qol-keyremap-card-bg)",
        ),
    ];

    for (plugin, file, system_derivation, component_derivation) in migrated_plugins {
        let plugin_css_path = workspace.join(file);
        let plugin_css = fs::read_to_string(&plugin_css_path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", plugin_css_path.display()));
        assert!(
            plugin_css.starts_with("@import \"./generated-theme-tokens.css\";\n\n"),
            "{plugin} CSS must import generated theme tokens before declarations"
        );
        if let Some(system_derivation) = system_derivation {
            assert!(
                plugin_css.contains(system_derivation),
                "{plugin} CSS must derive matching system colors from generated theme tokens"
            );
        }
        assert!(
            plugin_css.contains(component_derivation),
            "{plugin} CSS must derive component colors from generated theme tokens"
        );
    }
}

#[test]
fn web_plugin_indexes_bootstrap_theme_accent() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let files = [
        ("plugin-lights", "plugins/lights/ui/index.html"),
        ("plugin-keyremap", "plugins/keyremap/ui/index.html"),
    ];

    for (plugin, file) in files {
        let index_path = workspace.join(file);
        let index = fs::read_to_string(&index_path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", index_path.display()));
        assert!(
            index.contains("fetch('/api/theme/accent')"),
            "{plugin} index.html must fetch current tray theme accent"
        );
        assert!(
            index.contains("document.documentElement.dataset.qolAccent = body.key"),
            "{plugin} index.html must write the accent key to the document dataset"
        );
    }
}

#[test]
fn cinnamon_extension_requires_generated_tokens() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let extension_path = workspace
        .join("plugins/alt-tab/shell/cinnamon/qol-alt-tab-preview-plane@qol-tools/extension.js");
    let extension = fs::read_to_string(&extension_path)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", extension_path.display()));
    assert!(
        extension.contains("require(\"./generated-theme-tokens\")"),
        "extension.js must require the generated theme tokens module"
    );
}

#[test]
fn migrated_css_and_js_have_no_raw_color_literals() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut violations = Vec::new();
    let files = [
        "plugins/lights/ui/style.css",
        "plugins/lights/ui/components/hue-wheel.js",
        "plugins/keyremap/ui/style.css",
        "plugins/alt-tab/shell/cinnamon/qol-alt-tab-preview-plane@qol-tools/extension.js",
    ];

    for file in files {
        let path = workspace.join(file);
        let contents = fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        for (index, line) in contents.lines().enumerate() {
            if has_raw_css_color(line) {
                violations.push(format!("{file}:{}", index + 1));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "migrated CSS/JS must use generated tokens, not raw color literals:\n{}",
        violations.join("\n")
    );
}

fn hex6(color: u32) -> String {
    format!("#{:06x}", color & 0x00ff_ffff)
}

fn rgb_triplet(color: u32) -> String {
    let red = (color >> 16) & 0xff;
    let green = (color >> 8) & 0xff;
    let blue = color & 0xff;
    format!("{red}, {green}, {blue}")
}

fn accent_block<'a>(css: &'a str, key: &str) -> Option<&'a str> {
    let selector = format!(":root[data-qol-accent=\"{key}\"] {{\n");
    let start = css.find(&selector)? + selector.len();
    let rest = &css[start..];
    let end = rest.find("}\n")?;
    Some(&rest[..end])
}

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

struct ThemeAccentEnvGuard {
    previous: Option<std::ffi::OsString>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl ThemeAccentEnvGuard {
    fn set(value: &str) -> Self {
        let lock = env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let previous = std::env::var_os(qol_conventions::ENV_THEME_ACCENT);
        std::env::set_var(qol_conventions::ENV_THEME_ACCENT, value);
        Self {
            previous,
            _lock: lock,
        }
    }
}

impl Drop for ThemeAccentEnvGuard {
    fn drop(&mut self) {
        match &self.previous {
            Some(value) => std::env::set_var(qol_conventions::ENV_THEME_ACCENT, value),
            None => std::env::remove_var(qol_conventions::ENV_THEME_ACCENT),
        }
    }
}

fn has_raw_css_color(line: &str) -> bool {
    has_hex_color(line) || has_raw_rgb_call(line) || has_named_color(line)
}

fn has_hex_color(line: &str) -> bool {
    let bytes = line.as_bytes();
    for index in 0..bytes.len() {
        if bytes[index] != b'#' {
            continue;
        }
        let hex_len = bytes[index + 1..]
            .iter()
            .take_while(|byte| byte.is_ascii_hexdigit())
            .count();
        if !matches!(hex_len, 3 | 4 | 6 | 8) {
            continue;
        }
        if bytes
            .get(index + 1 + hex_len)
            .is_some_and(|byte| byte.is_ascii_hexdigit())
        {
            continue;
        }
        return true;
    }
    false
}

fn has_raw_rgb_call(line: &str) -> bool {
    ["rgb(", "rgba("].iter().any(|prefix| {
        let mut cursor = 0;
        while let Some(index) = line[cursor..].find(prefix) {
            let after_start = cursor + index + prefix.len();
            let after = line[after_start..].trim_start();
            if after
                .as_bytes()
                .first()
                .is_some_and(|byte| byte.is_ascii_digit())
            {
                return true;
            }
            if is_rgb_indirection(after) {
                return true;
            }
            cursor = after_start;
            if cursor >= line.len() {
                return false;
            }
            cursor += 1;
        }
        false
    })
}

fn is_rgb_indirection(after_rgb_prefix: &str) -> bool {
    if after_rgb_prefix.starts_with("var(") {
        return false;
    }
    after_rgb_prefix
        .as_bytes()
        .first()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
}

fn has_named_color(line: &str) -> bool {
    ["black", "white"]
        .iter()
        .any(|color| contains_css_word(line, color))
}

fn contains_css_word(line: &str, word: &str) -> bool {
    let mut cursor = 0;
    while let Some(index) = line[cursor..].find(word) {
        let start = cursor + index;
        let end = start + word.len();
        let before = start
            .checked_sub(1)
            .and_then(|pos| line.as_bytes().get(pos));
        let after = line.as_bytes().get(end);
        if before.is_none_or(|byte| !is_css_word_byte(*byte))
            && after.is_none_or(|byte| !is_css_word_byte(*byte))
        {
            return true;
        }
        cursor = end;
    }
    false
}

fn is_css_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
}

#[test]
fn raw_color_scanner_catches_hardened_forms() {
    for line in [
        "color: #fff;",
        "color: #ffff;",
        "color: #ffffffff;",
        "strokeStyle = 'white';",
        "shadowColor = 'black';",
        "background: rgb(1, 2, 3);",
        "background: rgba(ACCENT_FALLBACK, 0.5);",
        "background: rgb(DEFAULT_COLOR);",
    ] {
        assert!(has_raw_css_color(line), "scanner missed `{line}`");
    }
}

#[test]
fn raw_color_scanner_allows_tokenized_and_dynamic_forms() {
    for line in [
        "color: var(--text-primary);",
        "background: rgba(var(--accent-rgb), 0.2);",
        "ctx.fillStyle = `rgba(${accent}, 0.18)`;",
        "background: transparent;",
        "const colorHex = QOL_TRAY_INTERNAL_COLORS.configQrLight;",
    ] {
        assert!(
            !has_raw_css_color(line),
            "scanner false-positive for `{line}`"
        );
    }
}

#[test]
fn generated_css_tokens_are_namespaced() {
    let profiles = [
        ("core", css::dark_css()),
        ("tray-css", css::tray_css()),
        ("plugin-lights", css::plugin_lights_css()),
        ("plugin-keyremap", css::plugin_keyremap_css()),
    ];

    for (profile, css) in profiles {
        for line in css.lines() {
            let Some((name, _)) = line.trim().split_once(':') else {
                continue;
            };
            if !name.starts_with("--") {
                continue;
            }
            assert!(
                name.starts_with("--qol-"),
                "generated CSS token `{name}` in {profile} must be namespaced"
            );
        }
    }
}

#[test]
fn themed_gpui_surfaces_do_not_use_inline_color_literals() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let files = [
        "plugins/cli-sessions/src/ui/render.rs",
        "plugins/launcher/src/ui/view.rs",
        "plugins/alt-tab/src/app/render.rs",
        "plugins/removeapp/src/ui/mod.rs",
        "plugins/qol-shot/src/ui/region_selector/mod.rs",
        "plugins/qol-shot/src/ui/preview.rs",
    ];
    let mut violations = Vec::new();

    for file in files {
        let path = workspace.join(file);
        let contents = fs::read_to_string(&path).unwrap_or_else(|err| {
            panic!("failed to read themed surface {}: {err}", path.display())
        });
        for (index, line) in contents.lines().enumerate() {
            let compact = line
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect::<String>();
            if compact.contains("rgb(0x") || compact.contains("rgba(0x") {
                violations.push(format!("{file}:{}", index + 1));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "themed GPUI surfaces must use qol-theme tokens, not inline color literals:\n{}",
        violations.join("\n")
    );
}

fn relative_luminance(rgb: u32) -> f64 {
    let channel = |c: u32| {
        let c = c as f64 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let r = channel((rgb >> 16) & 0xff);
    let g = channel((rgb >> 8) & 0xff);
    let b = channel(rgb & 0xff);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

fn contrast_ratio(a: u32, b: u32) -> f64 {
    let (hi, lo) = {
        let (la, lb) = (relative_luminance(a), relative_luminance(b));
        if la > lb {
            (la, lb)
        } else {
            (lb, la)
        }
    };
    (hi + 0.05) / (lo + 0.05)
}

#[test]
fn tray_theme_presets_have_unique_keys_and_a_default() {
    let presets = qol_theme::tray_theme_presets();
    let mut keys: Vec<_> = presets.iter().map(|p| p.key).collect();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), presets.len(), "duplicate theme keys");
    assert!(qol_theme::tray_theme_preset(qol_theme::DEFAULT_TRAY_THEME_KEY).is_some());
    assert!(qol_theme::tray_theme_preset("nope").is_none());
}

#[test]
fn tray_theme_accents_reference_valid_presets() {
    for preset in qol_theme::tray_theme_presets() {
        let accent = qol_theme::dark_accent_preset(preset.accent_key)
            .unwrap_or_else(|| panic!("{}: unknown accent {}", preset.key, preset.accent_key));
        assert_eq!(
            preset.system.accent, accent.rgb,
            "{}: system.accent must match accent preset {}",
            preset.key, preset.accent_key
        );
    }
}

#[test]
fn tray_theme_identities_are_assigned() {
    for preset in qol_theme::tray_theme_presets() {
        let expected = if preset.key == "midnight" {
            "modern"
        } else {
            "retro"
        };
        assert_eq!(preset.identity.key, expected, "{}", preset.key);
    }
    assert_eq!(
        qol_theme::RETRO_IDENTITY.font_data,
        qol_theme::RETRO_IDENTITY.font_ui
    );
    assert_eq!(qol_theme::MODERN_IDENTITY.font_data, "var(--font-mono)");
}

#[test]
fn tray_css_emits_theme_override_blocks() {
    let css = css::tray_css();
    assert!(css.contains("--qol-system-overlay-surface-rgb: 18, 22, 30;"));
    assert!(css.contains("--qol-system-scrim-rgb: 4, 5, 8;"));
    assert!(
        !css.contains(":root[data-qol-theme=\"slate\"]"),
        "default theme needs no block"
    );
    let midnight_block = css
        .split(":root[data-qol-theme=\"midnight\"]")
        .nth(1)
        .unwrap();
    let midnight_block = midnight_block.split('}').next().unwrap();
    assert!(midnight_block.contains("--qol-system-surface-canvas: #090b19;"));
    assert!(
        !midnight_block.contains("--qol-system-accent-rgb"),
        "themes must not override accent"
    );
}

#[test]
fn tray_theme_js_emits_theme_metadata() {
    let js = css::tray_theme_js();
    assert!(js.contains("export const QOL_THEMES = ["));
    assert!(js.contains(
        "{ key: \"slate\", label: \"Slate\", accentKey: \"amber\", identityKey: \"retro\" },"
    ));
    assert!(js.contains(
        "{ key: \"midnight\", label: \"Midnight\", accentKey: \"violet\", identityKey: \"modern\" },"
    ));
    assert!(js.contains("export const QOL_DEFAULT_THEME = \"slate\";"));
}

#[test]
fn retro_identity_matches_legacy_hardcoded_values() {
    let cases = [
        ("--qol-identity-case-label", "uppercase"),
        ("--qol-identity-tracking-label", "var(--ls-md)"),
        ("--qol-identity-font-ui", "var(--font-mono)"),
        ("--qol-identity-radius-xs", "3px"),
        (
            "--qol-identity-frame-border",
            "var(--border-w-3) double var(--tui-line)",
        ),
        ("--qol-identity-frame-texture", "var(--tui-scanline)"),
        ("--qol-identity-line", "var(--tui-line)"),
        ("--qol-identity-line-soft", "var(--tui-line-soft)"),
        ("--qol-identity-surface-inset", "var(--tui-bg-screen)"),
        ("--qol-identity-surface-row", "var(--tui-bg-card)"),
        ("--qol-identity-desktop-bg", "var(--tui-desktop-bg)"),
        ("--qol-identity-prompt-display", "inline"),
        ("--qol-identity-frame-radius", "var(--radius-md)"),
        ("--qol-identity-frame-shadow", "none"),
        (
            "--qol-identity-card-border",
            "var(--border-w-1) solid var(--tui-line-soft)",
        ),
        ("--qol-identity-cover-bg", "var(--tui-screen-bg)"),
        ("--qol-identity-cover-scrim", "var(--ink-overlay-strong)"),
        ("--qol-identity-minimap-slab-radius", "3"),
    ];
    let css = css::tray_css();
    let base = css.split(":root[data-qol-theme").next().unwrap();
    for (name, value) in cases {
        assert!(base.contains(&format!("{name}: {value};")), "{name}");
    }
}

#[test]
fn tray_css_emits_identity_tokens_per_theme() {
    let css = css::tray_css();
    let base = css.split(":root[data-qol-theme").next().unwrap();
    assert!(base.contains("--qol-identity-font-ui: var(--font-mono);"));
    assert!(base.contains("--qol-identity-radius-md: 6px;"));
    let midnight = css
        .split(":root[data-qol-theme=\"midnight\"]")
        .nth(1)
        .unwrap();
    let midnight = midnight.split('}').next().unwrap();
    assert!(midnight.contains("--qol-identity-font-ui: var(--font-sans);"));
    assert!(midnight.contains("--qol-identity-radius-md: 10px;"));
    assert!(midnight.contains("--qol-identity-crt-band-display: none;"));
    assert!(midnight.contains("--qol-identity-line: var(--qol-system-border-subtle);"));
    assert!(midnight.contains("--qol-identity-line-soft: rgba(var(--paper-rgb), 0.07);"));
    assert!(midnight.contains("--qol-identity-surface-inset: var(--qol-system-surface-raised);"));
    assert!(midnight.contains("--qol-identity-surface-row: var(--qol-system-surface-raised);"));
    assert!(midnight.contains("--qol-identity-desktop-bg: var(--surface-canvas);"));
    assert!(midnight.contains("--qol-identity-prompt-display: none;"));
    assert!(midnight.contains("--qol-identity-frame-bg: var(--surface-elevated);"));
    assert!(midnight.contains("--qol-identity-cover-scrim: var(--qol-system-surface-raised);"));
    assert!(midnight.contains("--qol-identity-frame-radius: var(--radius-xl);"));
    assert!(midnight.contains(
        "--qol-identity-frame-shadow: 0 24px 60px rgba(0, 0, 0, 0.55), 0 0 0 1px rgba(var(--paper-rgb), 0.07);"
    ));
    assert_eq!(
        css.matches(":root[data-qol-theme=").count(),
        1,
        "only the modern theme emits an override block"
    );
}

#[test]
fn tray_theme_palettes_hold_contrast_floors() {
    for preset in qol_theme::tray_theme_presets() {
        let s = preset.system;
        let surfaces = [
            ("canvas", s.surface_canvas),
            ("elevated", s.surface_elevated),
            ("raised", s.surface_raised),
            ("hovered", s.surface_hovered),
        ];
        for (name, surface) in surfaces {
            let cases = [
                ("text_primary", s.text_primary, 6.5),
                ("text_secondary", s.text_secondary, 4.0),
                ("text_muted", s.text_muted, 2.4),
            ];
            for (text_name, text, floor) in cases {
                let ratio = contrast_ratio(text, surface);
                assert!(
                    ratio >= floor,
                    "{}: {text_name} on {name} = {ratio:.2}, floor {floor}",
                    preset.key
                );
            }
        }
        for pair in surfaces.windows(2) {
            let ratio = contrast_ratio(pair[0].1, pair[1].1);
            assert!(
                ratio >= 1.08,
                "{}: surfaces {} vs {} too close ({ratio:.3})",
                preset.key,
                pair[0].0,
                pair[1].0
            );
        }
    }
}

#[test]
fn light_system_palette_holds_contrast_floors() {
    let s = LIGHT_SYSTEM;
    let surfaces = [
        ("canvas", s.surface_canvas),
        ("elevated", s.surface_elevated),
        ("raised", s.surface_raised),
        ("hovered", s.surface_hovered),
    ];
    let ground = s.surface_elevated;
    let text_tiers = [
        ("text_primary", s.text_primary, 12.0),
        ("text_secondary", s.text_secondary, 8.0),
        ("text_muted", s.text_muted, 5.0),
        ("text_faint", s.text_faint, 3.3),
        ("accent_ink", s.accent_ink, 5.3),
    ];
    for (tier_name, tier, floor) in text_tiers {
        let ratio = contrast_ratio(tier, ground);
        assert!(
            ratio >= floor,
            "light: {tier_name} on elevated = {ratio:.2}, floor {floor}"
        );
    }
    for pair in surfaces.windows(2) {
        let ratio = contrast_ratio(pair[0].1, pair[1].1);
        assert!(
            ratio >= 1.04,
            "light: surfaces {} vs {} too close ({ratio:.3})",
            pair[0].0,
            pair[1].0
        );
    }

    let bg_selected = s.accent_fill;
    let cases = [
        (
            "launcher text_dim on bg_selected",
            s.text_muted,
            bg_selected,
            4.5,
        ),
        (
            "launcher text_selected on bg_selected",
            s.text_primary,
            bg_selected,
            4.5,
        ),
        (
            "launcher highlight on bg_selected",
            s.accent_ink,
            bg_selected,
            4.5,
        ),
        (
            "launcher text_faint tier on bg",
            mix_rgb(s.text_faint, s.surface_canvas, 0.24),
            s.surface_elevated,
            2.4,
        ),
        (
            "launcher semantic_prefix on bg",
            s.text_muted,
            s.surface_elevated,
            4.5,
        ),
        (
            "launcher semantic_contains on bg",
            s.text_muted,
            s.surface_elevated,
            4.5,
        ),
        (
            "launcher semantic_fuzzy on bg",
            s.text_muted,
            s.surface_elevated,
            4.5,
        ),
        (
            "launcher semantic_freq on bg",
            s.text_muted,
            s.surface_elevated,
            4.5,
        ),
        (
            "remove_app text_muted tier on chrome",
            s.text_faint,
            s.surface_canvas,
            2.7,
        ),
        (
            "toast border on window",
            s.border_subtle,
            s.surface_elevated,
            2.0,
        ),
    ];
    for (name, fg, bg, floor) in cases {
        let ratio = contrast_ratio(fg, bg);
        assert!(ratio >= floor, "light: {name} = {ratio:.2}, floor {floor}");
    }
}

const SURFACE_ROOTS: [&str; 3] = ["libs/qol-gpui/src", "plugins", "apps/qol-tray/src"];

fn surface_sources(workspace: &Path) -> Vec<(String, std::path::PathBuf)> {
    let mut found = Vec::new();
    for root in SURFACE_ROOTS {
        for path in rust_sources(&workspace.join(root)) {
            if path
                .components()
                .any(|part| part.as_os_str() == "examples" || part.as_os_str() == "tests")
            {
                continue;
            }
            let relative = path
                .strip_prefix(workspace)
                .unwrap_or(&path)
                .display()
                .to_string();
            found.push((relative, path));
        }
    }
    found.sort();
    found
}

fn px_arguments(compact: &str, method: &str) -> Vec<f32> {
    let needle = format!(".{method}(px(");
    let mut found = Vec::new();
    let mut rest = compact;
    while let Some(at) = rest.find(&needle) {
        rest = &rest[at + needle.len()..];
        let Some(end) = rest.find("))") else {
            break;
        };
        if let Ok(value) = rest[..end].parse::<f32>() {
            found.push(value);
        }
    }
    found
}

#[test]
fn gpui_surfaces_only_use_type_scale_sizes() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut violations = Vec::new();

    for (relative, path) in surface_sources(&workspace) {
        let contents = fs::read_to_string(&path).expect("read gpui source");
        for (index, line) in contents.lines().enumerate() {
            let compact = line
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect::<String>();
            for size in px_arguments(&compact, "text_size") {
                if !TEXT_SCALE.contains(&size) {
                    violations.push(format!("{relative}:{} uses {size}", index + 1));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "GPUI text sizes must come from the qol-theme type scale {TEXT_SCALE:?}:\n{}",
        violations.join("\n")
    );
}

const SURFACE_HEIGHT_FLOOR: f32 = 28.0;

const HEIGHT_METHODS: [&str; 4] = ["h", "min_h", "max_h", "size"];

const OFF_LADDER_HEIGHT_LITERAL_DEBT: [(&str, f32); 2] = [
    ("libs/qol-gpui/src/gamepad/view.rs", 34.0),
    ("libs/qol-gpui/src/gamepad/view.rs", 72.0),
];

#[test]
fn gpui_heights_at_or_above_the_surface_floor_stay_on_the_ladder() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let allowed = HEIGHT_LADDER
        .iter()
        .chain(LIST_ENTRY_HEIGHTS.iter())
        .copied()
        .collect::<Vec<f32>>();
    let mut problems = Vec::new();
    let mut seen = Vec::new();

    for (relative, path) in surface_sources(&workspace) {
        let contents = fs::read_to_string(&path).expect("read gpui source");
        for (index, line) in contents.lines().enumerate() {
            let compact = line
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect::<String>();
            for method in HEIGHT_METHODS {
                for value in px_arguments(&compact, method) {
                    if value < SURFACE_HEIGHT_FLOOR || allowed.contains(&value) {
                        continue;
                    }
                    seen.push((relative.clone(), value));
                    let recorded = OFF_LADDER_HEIGHT_LITERAL_DEBT
                        .iter()
                        .any(|(file, debt)| *file == relative && *debt == value);
                    if !recorded {
                        problems.push(format!("{relative}:{} sets {method} to {value}", index + 1));
                    }
                }
            }
        }
    }

    for (file, value) in OFF_LADDER_HEIGHT_LITERAL_DEBT {
        let still_there = seen
            .iter()
            .any(|(found, height)| found == file && *height == value);
        if !still_there {
            problems.push(format!(
                "{file} no longer draws a {value} height, so drop it from OFF_LADDER_HEIGHT_LITERAL_DEBT"
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "Anything {SURFACE_HEIGHT_FLOOR}px tall or more is a surface and belongs on the height ladder {allowed:?}. \
         Smaller literals are hairlines, dots and glyph boxes, which the ladder does not govern.\n{}",
        problems.join("\n")
    );
}

const RADIUS_METHODS: [&str; 9] = [
    "rounded",
    "rounded_t",
    "rounded_b",
    "rounded_l",
    "rounded_r",
    "rounded_tl",
    "rounded_tr",
    "rounded_bl",
    "rounded_br",
];

#[test]
fn gpui_corner_radii_given_in_px_stay_on_the_radius_ladder() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut problems = Vec::new();

    for (relative, path) in surface_sources(&workspace) {
        let contents = fs::read_to_string(&path).expect("read gpui source");
        for (index, line) in contents.lines().enumerate() {
            let compact = line
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect::<String>();
            for method in RADIUS_METHODS {
                for value in px_arguments(&compact, method) {
                    if !RADIUS_LADDER.contains(&value) {
                        problems.push(format!("{relative}:{} rounds by {value}", index + 1));
                    }
                }
            }
        }
    }

    assert!(
        problems.is_empty(),
        "A corner radius written in px must come from the qol-theme radius ladder {RADIUS_LADDER:?}:\n{}",
        problems.join("\n")
    );
}

#[test]
fn the_surface_tokens_added_for_v2_hold_their_contrast_floors() {
    let cases = [
        (
            "light button text on the solid fill",
            LIGHT_SYSTEM.solid_ink,
            LIGHT_SYSTEM.solid_fill,
        ),
        (
            "dark button text on the solid fill",
            DARK_SYSTEM.solid_ink,
            DARK_SYSTEM.solid_fill,
        ),
        (
            "light rail text on the rail",
            LIGHT_SYSTEM.text_rail,
            LIGHT_SYSTEM.surface_rail,
        ),
        (
            "dark rail text on the rail",
            DARK_SYSTEM.text_rail,
            DARK_SYSTEM.surface_rail,
        ),
        (
            "light warning text on the pane",
            LIGHT_SYSTEM.warning_ink,
            LIGHT_SYSTEM.surface_elevated,
        ),
        (
            "dark warning text on the pane",
            DARK_SYSTEM.warning_ink,
            DARK_SYSTEM.surface_elevated,
        ),
        (
            "light warning text on the rail",
            LIGHT_SYSTEM.warning_ink,
            LIGHT_SYSTEM.surface_rail,
        ),
        (
            "dark warning text on the rail",
            DARK_SYSTEM.warning_ink,
            DARK_SYSTEM.surface_rail,
        ),
    ];

    for (name, ink, surface) in cases {
        let ratio = contrast_ratio(ink, surface);
        assert!(
            ratio >= ACCENT_INK_FLOOR,
            "{name} = {ratio:.2}, floor {ACCENT_INK_FLOOR}"
        );
    }
}

#[test]
fn the_settings_panel_rail_stays_readable_in_both_themes() {
    let modes = [
        (
            "light",
            SettingsPanelPalette::from_theme(ThemeMode::Light, LIGHT_SYSTEM),
        ),
        (
            "dark",
            SettingsPanelPalette::from_theme(ThemeMode::Dark, DARK_SYSTEM),
        ),
    ];

    for (mode, rail) in modes {
        for (name, ink) in [
            ("rail text", rail.rail_text),
            ("inactive section", rail.rail_text_muted),
            ("active section", rail.rail_active_text),
        ] {
            let ratio = contrast_ratio(ink, rail.rail_bg);
            assert!(
                ratio >= ACCENT_INK_FLOOR,
                "{mode} {name} on the rail = {ratio:.2}, floor {ACCENT_INK_FLOOR}"
            );
        }
    }
}

#[test]
fn the_selected_row_fill_follows_the_active_accent() {
    for base in [LIGHT_SYSTEM, DARK_SYSTEM] {
        let brass = base.with_accent(0xb8860b);
        let moss = base.with_accent(0x5f8a42);

        assert_ne!(brass.accent_fill, moss.accent_fill);
        assert_ne!(brass.accent_fill, base.accent_fill_base);
        assert!(
            contrast_ratio(brass.accent_fill, base.text_muted) >= 4.5,
            "muted text has to stay legible on the selected row"
        );
    }
}

#[test]
fn washes_carry_their_alpha_and_follow_the_active_accent() {
    assert_eq!(css_rgba_milli(0xb8860b, 220).packed(), 0xb8860b38);
    assert_eq!(css_rgba_milli(0x000000, 1000).packed(), 0x000000ff);
    assert_eq!(css_rgba_milli(0xffffff, 0).packed(), 0xffffff00);

    let swapped = DARK_SYSTEM.with_accent(0x123456);
    let default_wash = WashPalette::dark(DARK_SYSTEM);
    let swapped_wash = WashPalette::dark(swapped);

    assert_eq!(default_wash.accent_border.rgb, DARK_SYSTEM.accent);
    assert_eq!(default_wash.accent_halo.rgb, DARK_SYSTEM.accent);
    assert_eq!(swapped_wash.accent_border.rgb, 0x123456);
    assert_eq!(swapped_wash.accent_halo.rgb, 0x123456);

    assert_eq!(swapped_wash.hairline, default_wash.hairline);
    assert_eq!(swapped_wash.separator, default_wash.separator);
    assert_eq!(swapped_wash.wash_selected, default_wash.wash_selected);
    assert_eq!(swapped_wash.cast, default_wash.cast);

    let light = WashPalette::light(LIGHT_SYSTEM);
    assert_eq!(light.halo_success.rgb, LIGHT_SYSTEM.success);
    assert_eq!(light.halo_attention.rgb, LIGHT_SYSTEM.warning);
    assert_eq!(light.halo_invalid.rgb, LIGHT_SYSTEM.danger);
    assert_eq!(light.edge_invalid.rgb, LIGHT_SYSTEM.danger);

    assert_eq!(WashPalette::for_mode(ThemeMode::Light, LIGHT_SYSTEM), light);
    assert_eq!(
        WashPalette::for_mode(ThemeMode::Dark, DARK_SYSTEM),
        default_wash
    );
}

const FOCUS_RING_FLOOR: f64 = 3.0;
const ACCENT_INK_FLOOR: f64 = 4.5;

const ACCENT_FLOOR_DEBT: [(&str, &str); 0] = [];

#[test]
fn every_accent_preset_clears_the_focus_ring_floor() {
    let modes = [
        ("light", LIGHT_ACCENT_PRESETS, LIGHT_SYSTEM.surface_elevated),
        ("dark", DARK_ACCENT_PRESETS, DARK_SYSTEM.surface_elevated),
    ];
    let mut problems = Vec::new();

    for (mode, presets, pane) in modes {
        for preset in presets {
            let edge = contrast_ratio(preset.rgb, pane);
            let ink = contrast_ratio(preset.ink, pane);
            let recorded = ACCENT_FLOOR_DEBT
                .iter()
                .any(|(debt_mode, key)| *debt_mode == mode && *key == preset.key);

            if edge < FOCUS_RING_FLOOR && !recorded {
                problems.push(format!(
                    "{mode}/{} ({}) draws the focus ring at {edge:.2} against the pane, floor {FOCUS_RING_FLOOR}",
                    preset.key, preset.label
                ));
            }
            if edge >= FOCUS_RING_FLOOR && recorded {
                problems.push(format!(
                    "{mode}/{} ({}) now clears the floor at {edge:.2}; delete its ACCENT_FLOOR_DEBT entry",
                    preset.key, preset.label
                ));
            }
            if ink < ACCENT_INK_FLOOR {
                problems.push(format!(
                    "{mode}/{} ({}) reads accent text at {ink:.2} against the pane, floor {ACCENT_INK_FLOOR}",
                    preset.key, preset.label
                ));
            }
        }
    }

    assert!(
        problems.is_empty(),
        "Bone and Amber derives the focus ring from whichever accent the user picked, so every \
preset has to carry it. A preset below the floor has no visible focus indicator.\n{}",
        problems.join("\n")
    );
}

const LADDER_GOVERNED_HEIGHTS: [(&str, &str, f32); 18] = [
    ("libs/qol-gpui/src/kit.rs", "HEADER_HEIGHT", HEIGHT_BAND),
    ("libs/qol-gpui/src/kit.rs", "SECTION_HEIGHT", HEIGHT_INLINE),
    ("libs/qol-gpui/src/kit.rs", "ROW_HEIGHT", HEIGHT_SETTING_ROW),
    (
        "libs/qol-gpui/src/kit.rs",
        "ROW_DESCRIBED_HEIGHT",
        HEIGHT_SETTING_ROW,
    ),
    ("libs/qol-gpui/src/kit.rs", "ROW_TIGHT_HEIGHT", 32.0),
    ("libs/qol-gpui/src/kit.rs", "GUTTER", SPACE_GUTTER),
    ("libs/qol-gpui/src/dropdown.rs", "ROW_H", HEIGHT_INLINE),
    (
        "libs/qol-gpui/src/settings_panel/mod.rs",
        "PANEL_ROW_HEIGHT",
        HEIGHT_SETTING_ROW,
    ),
    (
        "libs/qol-gpui/src/settings_panel/mod.rs",
        "PANEL_RAIL_ITEM_HEIGHT",
        HEIGHT_CONTROL,
    ),
    (
        "libs/qol-gpui/src/settings_panel/mod.rs",
        "PANEL_LIST_ITEM_HEIGHT",
        HEIGHT_RULE_ROW,
    ),
    (
        "libs/qol-gpui/src/settings_panel/mod.rs",
        "PANEL_OBJECT_ROW_HEIGHT",
        HEIGHT_RULE_ROW,
    ),
    (
        "libs/qol-gpui/src/settings_panel/mod.rs",
        "PANEL_BAND_HEIGHT",
        HEIGHT_BAND,
    ),
    (
        "libs/qol-gpui/src/settings_panel/mod.rs",
        "PANEL_GROUP_HEADER_HEIGHT",
        HEIGHT_CONTROL,
    ),
    (
        "libs/qol-gpui/src/settings_panel/mod.rs",
        "PANEL_HINT_BAR_HEIGHT",
        HEIGHT_HINT_BAR,
    ),
    (
        "libs/qol-gpui/src/settings_panel/mod.rs",
        "PANEL_FILTER_HEIGHT",
        HEIGHT_CONTROL,
    ),
    (
        "plugins/cli-sessions/src/ui/collapse.rs",
        "STRIP_HEIGHT",
        32.0,
    ),
    ("plugins/launcher/src/ui/layout.rs", "ROW_HEIGHT", 32.0),
    (
        "plugins/alt-tab/src/picker/layout.rs",
        "HOTKEY_HINTS_HEIGHT",
        HEIGHT_HINT_BAR,
    ),
];

const OFF_LADDER_DEBT: [(&str, &str, f32); 1] = [("libs/qol-gpui/src/dropdown.rs", "ROW_H", 26.0)];

fn declared_f32(workspace: &Path, file: &str, name: &str) -> Option<f32> {
    let contents = fs::read_to_string(workspace.join(file)).ok()?;
    let needle = format!("const {name}: f32 =");
    for line in contents.lines() {
        let Some(rest) = line.split(&needle).nth(1) else {
            continue;
        };
        let value = rest.trim().trim_end_matches(';').trim();
        if let Ok(parsed) = value.parse::<f32>() {
            return Some(parsed);
        }
        return ladder_constant(value.trim_start_matches("qol_theme::"));
    }
    None
}

fn ladder_constant(name: &str) -> Option<f32> {
    match name {
        "HEIGHT_INLINE" => Some(HEIGHT_INLINE),
        "HEIGHT_CONTROL" => Some(HEIGHT_CONTROL),
        "HEIGHT_HINT_BAR" => Some(HEIGHT_HINT_BAR),
        "HEIGHT_RULE_ROW" => Some(HEIGHT_RULE_ROW),
        "HEIGHT_SETTING_ROW" => Some(HEIGHT_SETTING_ROW),
        "HEIGHT_BAND" => Some(HEIGHT_BAND),
        "SPACE_GUTTER" => Some(SPACE_GUTTER),
        _ => None,
    }
}

#[test]
fn every_ladder_governed_height_is_on_its_rung_or_recorded_as_debt() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut problems = Vec::new();

    for (file, name, target) in LADDER_GOVERNED_HEIGHTS {
        let Some(actual) = declared_f32(&workspace, file, name) else {
            problems.push(format!(
                "{file} no longer declares {name}; update LADDER_GOVERNED_HEIGHTS"
            ));
            continue;
        };
        let debt = OFF_LADDER_DEBT
            .iter()
            .find(|(debt_file, debt_name, _)| *debt_file == file && *debt_name == name);

        match debt {
            None if actual != target => problems.push(format!(
                "{file}:{name} is {actual} but its rung is {target}; fix it or record it in OFF_LADDER_DEBT"
            )),
            Some((_, _, recorded)) if actual == target => problems.push(format!(
                "{file}:{name} now sits on its rung; delete its OFF_LADDER_DEBT entry (recorded {recorded})"
            )),
            Some((_, _, recorded)) if actual != *recorded => problems.push(format!(
                "{file}:{name} moved from the recorded {recorded} to {actual} without reaching its rung {target}"
            )),
            _ => {}
        }
    }

    assert!(
        problems.is_empty(),
        "The Bone and Amber height ladder is {HEIGHT_LADDER:?} with list entries {LIST_ENTRY_HEIGHTS:?}. \
Window sizes, image caps and aspect components are deliberately not governed by it.\n{}",
        problems.join("\n")
    );
}

fn rust_sources(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(rust_sources(&path));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
    found
}
