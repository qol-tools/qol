use super::super::PanelTone;
use std::process::Command;

pub(crate) fn current() -> PanelTone {
    Command::new("defaults")
        .args(["read", "-g", "AppleInterfaceStyle"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map_or(PanelTone::Light, |output| {
            tone_from_interface_style(&String::from_utf8_lossy(&output.stdout))
        })
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
