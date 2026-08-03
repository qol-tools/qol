#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DesktopEnvironment {
    Gnome,
    Cinnamon,
    Kde,
    Unknown,
}

pub(crate) fn classify_desktop(raw: &str) -> DesktopEnvironment {
    for part in raw.split(':') {
        match part.trim().to_ascii_lowercase().as_str() {
            "gnome" => return DesktopEnvironment::Gnome,
            "x-cinnamon" | "cinnamon" => return DesktopEnvironment::Cinnamon,
            "kde" | "plasma" => return DesktopEnvironment::Kde,
            _ => {}
        }
    }
    DesktopEnvironment::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification_uses_the_first_recognized_desktop() {
        assert_eq!(classify_desktop("ubuntu:GNOME"), DesktopEnvironment::Gnome);
        assert_eq!(classify_desktop("X-Cinnamon"), DesktopEnvironment::Cinnamon);
        assert_eq!(classify_desktop("KDE"), DesktopEnvironment::Kde);
        assert_eq!(classify_desktop("plasma"), DesktopEnvironment::Kde);
        assert_eq!(classify_desktop("Hyprland"), DesktopEnvironment::Unknown);
    }
}
