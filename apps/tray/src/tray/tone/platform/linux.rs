use super::super::PanelTone;
use std::process::Command;

pub(crate) fn current() -> PanelTone {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if is_cinnamon(&desktop) {
        return gsettings("org.cinnamon.theme", "name")
            .map_or(PanelTone::Dark, |name| tone_from_cinnamon_theme(&name));
    }
    gsettings("org.gnome.desktop.interface", "color-scheme")
        .map_or(PanelTone::Dark, |scheme| tone_from_color_scheme(&scheme))
}

fn gsettings(schema: &str, key: &str) -> Option<String> {
    let output = Command::new("gsettings")
        .args(["get", schema, key])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
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
    if unquote(scheme) == "prefer-dark" {
        PanelTone::Dark
    } else {
        PanelTone::Light
    }
}

fn unquote(value: &str) -> &str {
    let value = value.trim();
    value
        .strip_prefix('\'')
        .and_then(|inner| inner.strip_suffix('\''))
        .unwrap_or(value)
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
            ("'Mint-Y-Dark-Aqua'\n", PanelTone::Dark),
            ("'Mint-Y-DARK'", PanelTone::Dark),
            ("'Mint-Y-Aqua'\n", PanelTone::Light),
            ("'Mint-L'", PanelTone::Light),
            ("", PanelTone::Light),
        ];
        for (name, expected) in cases {
            assert_eq!(tone_from_cinnamon_theme(name), expected, "{name:?}");
        }
    }

    #[test]
    fn only_prefer_dark_color_scheme_is_dark() {
        let cases = [
            ("'prefer-dark'\n", PanelTone::Dark),
            ("prefer-dark", PanelTone::Dark),
            ("'prefer-light'\n", PanelTone::Light),
            ("'default'\n", PanelTone::Light),
            ("", PanelTone::Light),
        ];
        for (scheme, expected) in cases {
            assert_eq!(tone_from_color_scheme(scheme), expected, "{scheme:?}");
        }
    }
}
