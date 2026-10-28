use super::super::{PanelLook, PanelTone};
use qol_desktop_settings::gsettings;

const ST_WARNING: (u8, u8, u8) = (0xf5, 0x79, 0x3e);

pub(crate) fn current() -> PanelLook {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if is_cinnamon(&desktop) {
        return PanelLook {
            tone: gsettings::get("org.cinnamon.theme", "name")
                .map_or(PanelTone::Dark, |name| tone_from_cinnamon_theme(&name)),
            update_accent: Some(ST_WARNING),
        };
    }
    PanelLook {
        tone: gsettings::get("org.gnome.desktop.interface", "color-scheme")
            .map_or(PanelTone::Dark, |scheme| tone_from_color_scheme(&scheme)),
        update_accent: None,
    }
}

fn is_cinnamon(desktop: &str) -> bool {
    desktop.to_ascii_lowercase().contains("cinnamon")
}

fn tone_from_cinnamon_theme(name: &str) -> PanelTone {
    if name.to_ascii_lowercase().contains("dark") {
        PanelTone::Dark
    } else {
        PanelTone::Light
    }
}

fn tone_from_color_scheme(scheme: &str) -> PanelTone {
    if scheme == "prefer-light" {
        PanelTone::Light
    } else {
        PanelTone::Dark
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cinnamon_is_found_in_any_desktop_list_position() {
        let cases = [
            ("X-Cinnamon", true),
            ("cinnamon", true),
            ("ubuntu:GNOME", false),
            ("KDE", false),
            ("", false),
        ];
        for (desktop, expected) in cases {
            assert_eq!(is_cinnamon(desktop), expected, "{desktop:?}");
        }
    }

    #[test]
    fn cinnamon_theme_name_with_dark_is_dark() {
        let cases = [
            ("Mint-Y-Dark-Aqua", PanelTone::Dark),
            ("Mint-Y-DARK", PanelTone::Dark),
            ("Mint-Y-Aqua", PanelTone::Light),
            ("Mint-L", PanelTone::Light),
            ("", PanelTone::Light),
        ];
        for (name, expected) in cases {
            assert_eq!(tone_from_cinnamon_theme(name), expected, "{name:?}");
        }
    }

    #[test]
    fn only_prefer_light_color_scheme_is_light() {
        let cases = [
            ("prefer-dark", PanelTone::Dark),
            ("prefer-light", PanelTone::Light),
            ("default", PanelTone::Dark),
            ("", PanelTone::Dark),
        ];
        for (scheme, expected) in cases {
            assert_eq!(tone_from_color_scheme(scheme), expected, "{scheme:?}");
        }
    }
}
