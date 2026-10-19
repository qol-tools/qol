use qol_theme::{
    contrast_ratio, count_disc, dark_accent_presets, dark_theme_with_accent_key,
    light_accent_presets, light_theme_with_accent_key, status_look, Alpha, Grounds, StatusTone,
    SystemPalette, ThemeMode, DARK_SYSTEM, LIGHT_SYSTEM,
};

const TEXT_FLOOR: f64 = 4.5;

fn palettes() -> Vec<(String, ThemeMode, SystemPalette)> {
    let mut palettes = vec![
        ("light/default".to_string(), ThemeMode::Light, LIGHT_SYSTEM),
        ("dark/default".to_string(), ThemeMode::Dark, DARK_SYSTEM),
    ];
    for preset in light_accent_presets() {
        palettes.push((
            format!("light/{}", preset.key),
            ThemeMode::Light,
            light_theme_with_accent_key(preset.key).system,
        ));
    }
    for preset in dark_accent_presets() {
        palettes.push((
            format!("dark/{}", preset.key),
            ThemeMode::Dark,
            dark_theme_with_accent_key(preset.key).system,
        ));
    }
    palettes
}

#[test]
fn every_status_ink_clears_its_wash_on_every_ground() {
    let mut problems = Vec::new();
    for (name, mode, palette) in palettes() {
        let grounds = Grounds::from_theme(mode, palette);
        for (ground_name, ground) in [
            ("pane", grounds.pane),
            ("band", grounds.band),
            ("band_hover", grounds.band_hover),
        ] {
            for tone in StatusTone::ALL {
                let look = status_look(tone, ground, palette);
                let ratio = contrast_ratio(look.ink, look.wash.over(ground.bg));
                if ratio < TEXT_FLOOR {
                    problems.push(format!(
                        "{name} {ground_name} {tone:?} reads at {ratio:.2}, floor {TEXT_FLOOR}"
                    ));
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn danger_is_tinted_like_warning_and_neutral_keeps_the_well() {
    for (name, mode, palette) in palettes() {
        let pane = Grounds::from_theme(mode, palette).pane;
        let danger = status_look(StatusTone::Danger, pane, palette);
        assert_eq!(danger.wash.rgb, palette.danger, "{name}");
        assert_eq!(danger.wash.alpha_milli, Alpha::Tint.milli(), "{name}");
        assert_eq!(danger.dot, palette.danger, "{name}");
        let warning = status_look(StatusTone::Warning, pane, palette);
        assert_eq!(warning.wash.rgb, palette.warning, "{name}");
        assert_eq!(warning.wash.alpha_milli, danger.wash.alpha_milli, "{name}");
        let neutral = status_look(StatusTone::Neutral, pane, palette);
        assert_eq!(neutral.wash, pane.well, "{name}");
        assert_eq!(neutral.ink, pane.soft, "{name}");
    }
}

#[test]
fn every_count_reads_on_its_disc() {
    let mut problems = Vec::new();
    for (name, mode, palette) in palettes() {
        let grounds = Grounds::from_theme(mode, palette);
        for (ground_name, ground) in [
            ("pane", grounds.pane),
            ("rail", grounds.rail),
            ("menu", grounds.menu),
            ("band", grounds.band),
            ("band_hover", grounds.band_hover),
            ("attention", grounds.attention),
            ("invalid", grounds.invalid),
        ] {
            let ratio = contrast_ratio(ground.on_mark, count_disc(ground));
            if ratio < TEXT_FLOOR {
                problems.push(format!(
                    "{name} {ground_name} count reads at {ratio:.2}, floor {TEXT_FLOOR}"
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
