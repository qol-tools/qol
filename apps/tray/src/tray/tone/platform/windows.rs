use super::super::PanelTone;
use qol_platform::native::registry::{read_dword, Hive};

const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";

pub(crate) fn current() -> PanelTone {
    tone_from_system_uses_light_theme(
        read_dword(Hive::CurrentUser, PERSONALIZE, "SystemUsesLightTheme")
            .ok()
            .flatten(),
    )
}

fn tone_from_system_uses_light_theme(value: Option<u32>) -> PanelTone {
    if value == Some(1) {
        PanelTone::Light
    } else {
        PanelTone::Dark
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_light_theme_flag_of_one_is_light() {
        let cases = [
            (Some(1), PanelTone::Light),
            (Some(0), PanelTone::Dark),
            (Some(2), PanelTone::Dark),
            (None, PanelTone::Dark),
        ];
        for (value, expected) in cases {
            assert_eq!(
                tone_from_system_uses_light_theme(value),
                expected,
                "{value:?}"
            );
        }
    }
}
