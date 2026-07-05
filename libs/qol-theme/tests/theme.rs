use qol_color::{mix_rgb, with_alpha};
use qol_theme::{
    alt_tab_preview_plane_dark, cli_sessions_dark, css, dark_accent_preset, dark_theme,
    launcher_dark, remove_app_dark, resolve_surface_color, shot_preview_dark, shot_selector_dark,
    PickerSurfacePalette, ThemeMode, DARK_ACCENT_PRESETS, DARK_REFERENCE, DARK_SYSTEM,
    DEV_ACCENT_KEY, PROD_ACCENT_KEY,
};
use std::{fs, path::Path};

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

fn has_raw_css_color(line: &str) -> bool {
    has_hex_color(line) || has_numeric_rgb(line)
}

fn has_hex_color(line: &str) -> bool {
    let bytes = line.as_bytes();
    for index in 0..bytes.len().saturating_sub(6) {
        if bytes[index] != b'#' {
            continue;
        }
        if !bytes[index + 1..index + 7]
            .iter()
            .all(|byte| byte.is_ascii_hexdigit())
        {
            continue;
        }
        if bytes
            .get(index + 7)
            .is_some_and(|byte| byte.is_ascii_hexdigit())
        {
            continue;
        }
        return true;
    }
    false
}

fn has_numeric_rgb(line: &str) -> bool {
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
            cursor = after_start;
            if cursor >= line.len() {
                return false;
            }
            cursor += 1;
        }
        false
    })
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
