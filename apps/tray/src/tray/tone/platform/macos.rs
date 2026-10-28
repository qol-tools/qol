use super::super::{PanelLook, PanelTone};
use std::process::Command;

const SYSTEM_RED_LIGHT: (u8, u8, u8) = (0xff, 0x3b, 0x30);
const SYSTEM_RED_DARK: (u8, u8, u8) = (0xff, 0x45, 0x3a);

pub(crate) fn current() -> PanelLook {
    let tone = Command::new("defaults")
        .args(["read", "-g", "AppleInterfaceStyle"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map_or(PanelTone::Light, |output| {
            tone_from_interface_style(&String::from_utf8_lossy(&output.stdout))
        });
    PanelLook {
        tone,
        update_accent: Some(badge_red(tone)),
    }
}

fn badge_red(tone: PanelTone) -> (u8, u8, u8) {
    match tone {
        PanelTone::Dark => SYSTEM_RED_DARK,
        PanelTone::Light => SYSTEM_RED_LIGHT,
    }
}

fn tone_from_interface_style(style: &str) -> PanelTone {
    if style.trim() == "Dark" {
        PanelTone::Dark
    } else {
        PanelTone::Light
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_dark_interface_style_is_dark() {
        let cases = [
            ("Dark\n", PanelTone::Dark),
            ("Dark", PanelTone::Dark),
            ("Light\n", PanelTone::Light),
            ("", PanelTone::Light),
        ];
        for (style, expected) in cases {
            assert_eq!(tone_from_interface_style(style), expected, "{style:?}");
        }
    }
}
