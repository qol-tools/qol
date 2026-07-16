use qol_color::{mix_rgb, with_alpha};
use qol_theme::{
    alt_tab_preview_plane_dark, cli_sessions_dark, css, dark_accent_preset, dark_theme,
    dark_theme_with_accent_key, launcher_dark, remove_app_dark, resolve_surface_color,
    runtime_dark_theme, shot_preview_dark, shot_selector_dark, PickerSurfacePalette, ThemeMode,
    DARK_ACCENT_PRESETS, DARK_REFERENCE, DARK_SYSTEM, DARK_TRAY_INTERNAL, DEV_ACCENT_KEY,
    PROD_ACCENT_KEY,
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
    assert_eq!(theme.components.launcher, launcher_dark());
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
    let palette = launcher_dark();
    assert_eq!(palette.bg, DARK_SYSTEM.surface_elevated);
    assert_eq!(palette.bg_badge, DARK_SYSTEM.surface_raised);
    assert_eq!(palette.text_selected, DARK_SYSTEM.text_primary);
    assert_eq!(palette.text, DARK_SYSTEM.text_secondary);
    assert_eq!(palette.highlight, DARK_SYSTEM.accent);
    assert_eq!(palette.border, DARK_SYSTEM.border_subtle);
    assert_eq!(palette.momentum_up.len(), 5);
    assert_eq!(palette.momentum_down.len(), 5);
    assert_eq!(palette.compass_up.len(), 3);
    assert_eq!(palette.compass_down.len(), 3);
}

#[test]
fn cli_sessions_palette_derives_from_system_roles() {
    let palette = cli_sessions_dark();
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
        with_alpha(DARK_REFERENCE.white, 0x0f)
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
    assert_eq!(palette.claude, 0xd97757);
    assert_eq!(palette.codex, 0x10a37f);
}

#[test]
fn remove_app_palette_derives_from_system_roles() {
    let palette = remove_app_dark();
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
        with_alpha(DARK_REFERENCE.white, 0x0f)
    );
    assert_eq!(
        palette.warning_banner_rgba,
        with_alpha(DARK_SYSTEM.warning, 0x1a)
    );
}

#[test]
fn shot_preview_palette_derives_from_system_roles() {
    let palette = shot_preview_dark();
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
fn alt_tab_preview_plane_palette_derives_from_system_roles() {
    let palette = alt_tab_preview_plane_dark();
    assert_eq!(
        palette.backdrop_rgba,
        with_alpha(DARK_REFERENCE.black, 0x1c)
    );
    assert_eq!(palette.label_text, DARK_SYSTEM.text_primary);
    assert_eq!(
        palette.card_bg_rgba,
        with_alpha(DARK_SYSTEM.surface_elevated, 0xc8)
    );
    assert_eq!(
        palette.card_border_rgba,
        with_alpha(DARK_SYSTEM.text_secondary, 0xb4)
    );
    assert_eq!(
        palette.card_selected_bg_rgba,
        with_alpha(
            mix_rgb(DARK_SYSTEM.surface_raised, DARK_SYSTEM.accent, 0.28),
            0xd2
        )
    );
    assert_eq!(
        palette.card_selected_border_rgba,
        with_alpha(mix_rgb(DARK_SYSTEM.accent, DARK_REFERENCE.white, 0.3), 0xff)
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
            "    labelText: \"#edf2fb\",\n",
            "    cardBg: \"rgba(20, 24, 31, 200)\",\n",
            "    cardBorder: \"rgba(184, 192, 208, 180)\",\n",
            "    cardSelectedBg: \"rgba(88, 71, 51, 210)\",\n",
            "    cardSelectedBorder: \"rgba(255, 203, 135, 255)\",\n",
            "};\n",
        )
    );
}

#[test]
fn shot_selector_palette_derives_from_system_roles() {
    let palette = shot_selector_dark();
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
fn picker_surface_palette_derives_documented_values_from_default_card() {
    let palette = PickerSurfacePalette::from_card_color(0x202322, 0.85);
    assert_eq!(palette.panel_bg, 0x0e0f0f);
    assert_eq!(palette.header_bg, 0x151716);
    assert_eq!(palette.header_border, 0x323534);
    assert_eq!(palette.card_bg, 0x202322);
    assert_eq!(palette.card_hover_bg, 0x303231);
    assert_eq!(palette.card_selected_bg, 0x3d403f);
    assert_eq!(palette.card_selected_border, 0x5c615e);
    assert_eq!(palette.card_bg_rgba, 0x202322d9);
    assert_eq!(palette.card_selected_rgba, 0x3d403feb);
    assert_eq!(palette.caption_divider, 0x3b3d3d94);
    assert_eq!(palette.preview_icon_border, 0x3b3d3d7a);
    assert_eq!(palette.preview_icon_selected_border, 0x484b4a85);
    assert_eq!(palette.header_left_text, 0x5e6a84);
    assert_eq!(palette.header_right_text, 0x3a4252);
    assert_eq!(palette.grid_empty_text, 0x5e6a84);
    assert_eq!(palette.label_text, 0xd4dbea);
    assert_eq!(palette.label_selected_text, 0xf8fbff);
    assert_eq!(palette.placeholder_text, 0x4a5268);
    assert_eq!(palette.placeholder_bg, 0x1f2531);
    assert_eq!(palette.placeholder_border, 0x3a4252);
}

#[test]
fn resolve_surface_color_matches_alt_tab_config_fallbacks() {
    let cases = [
        ("#203040", "#202322", 1.0, 1.2, 0x203040, 1.0),
        ("#ff8040", "#202322", 0.25, 0.85, 0x402010, 0.85),
        ("#102030", "#202322", 2.0, 0.85, 0x102030, 0.85),
        ("#102030", "#202322", -1.0, 0.85, 0x000000, 0.85),
        ("nope", "#202322", 1.0, -1.0, 0x202322, 0.0),
        (
            "nope",
            "also-nope",
            1.0,
            0.5,
            DARK_SYSTEM.surface_raised,
            0.5,
        ),
    ];
    for (input, fallback, brightness, opacity, expected_color, expected_opacity) in cases {
        assert_eq!(
            resolve_surface_color(input, fallback, brightness, opacity),
            (expected_color, expected_opacity)
        );
    }
}

#[test]
fn dark_css_emits_stable_token_names() {
    assert_eq!(
        css::dark_css(),
        concat!(
            "/* @generated by qol-theme-css; do not edit by hand. */\n",
            ":root {\n",
            "    --qol-system-accent-rgb: 255, 180, 84;\n",
            "    --qol-system-success-rgb: 74, 222, 128;\n",
            "    --qol-system-danger-rgb: 255, 107, 107;\n",
            "    --qol-system-info-rgb: 104, 176, 255;\n",
            "    --qol-system-warning-rgb: 255, 193, 7;\n",
            "    --qol-system-ink-rgb: 0, 0, 0;\n",
            "    --qol-system-paper-rgb: 255, 255, 255;\n",
            "    --qol-system-surface-canvas: #0c0e13;\n",
            "    --qol-system-surface-elevated: #14181f;\n",
            "    --qol-system-surface-raised: #171c26;\n",
            "    --qol-system-surface-hovered: #1f2531;\n",
            "    --qol-system-text-primary: #edf2fb;\n",
            "    --qol-system-text-secondary: #b8c0d0;\n",
            "    --qol-system-text-muted: #67748f;\n",
            "    --qol-system-text-faint: #4d5870;\n",
            "    --qol-system-border-subtle: #2f3644;\n",
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

    let tray = css::tray_css();
    assert!(tray.starts_with(&core));
    assert!(tray.contains("    --qol-reference-slate-750: #2f3644;\n"));
    assert!(tray.contains("    --qol-tray-blue-500: #4a9eff;\n"));
    assert!(tray.contains("    --qol-tray-border-default-2: #3e485b;\n"));
    assert!(tray.contains("    --qol-atmosphere-wood-bg: #120a05;\n"));
    assert!(tray.contains("    --qol-minimap-active-text: rgba(255, 255, 255, 0.98);\n"));
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
    assert!(js.contains(&format!("QOL_DEV_ACCENT = \"{DEV_ACCENT_KEY}\"")));
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
        !theme_tokens.contains("255, 180, 84"),
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
        "apps/qol-tray/ui/fx/dissolve/glitch-squares.js",
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
            "plugins/plugin-lights/ui/generated-theme-tokens.css",
            css::plugin_lights_css(),
        ),
        (
            "plugins/plugin-keyremap/ui/generated-theme-tokens.css",
            css::plugin_keyremap_css(),
        ),
        (
            "plugins/plugin-alt-tab/shell/cinnamon/qol-alt-tab-preview-plane@qol-tools/generated-theme-tokens.js",
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
            "plugins/plugin-lights/ui/style.css",
            Some("rgb(var(--qol-system-success-rgb))"),
            "var(--qol-lights-accent-btn-shadow)",
        ),
        (
            "plugin-keyremap",
            "plugins/plugin-keyremap/ui/style.css",
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
        ("plugin-lights", "plugins/plugin-lights/ui/index.html"),
        ("plugin-keyremap", "plugins/plugin-keyremap/ui/index.html"),
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
    let extension_path = workspace.join(
        "plugins/plugin-alt-tab/shell/cinnamon/qol-alt-tab-preview-plane@qol-tools/extension.js",
    );
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
        "plugins/plugin-lights/ui/style.css",
        "plugins/plugin-lights/ui/components/hue-wheel.js",
        "plugins/plugin-keyremap/ui/style.css",
        "plugins/plugin-alt-tab/shell/cinnamon/qol-alt-tab-preview-plane@qol-tools/extension.js",
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
        "plugins/plugin-cli-sessions/src/ui/render.rs",
        "plugins/plugin-launcher/src/ui/view.rs",
        "plugins/plugin-alt-tab/src/app/render.rs",
        "plugins/plugin-removeapp/src/ui/mod.rs",
        "plugins/qol-shot/src/region_selector.rs",
        "plugins/qol-shot/src/preview.rs",
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
                ratio >= 1.04,
                "{}: surfaces {} vs {} too close ({ratio:.3})",
                preset.key,
                pair[0].0,
                pair[1].0
            );
        }
    }
}
