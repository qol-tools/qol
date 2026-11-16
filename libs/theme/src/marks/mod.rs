mod solid;

use crate::Ground;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mark {
    Qol,
    Settings,
    Shortcuts,
    Hotkeys,
    Updates,
    LinkedDevices,
    Profiles,
    Plugins,
    Notifications,
    App,
    Link,
    PluginAction,
    AltTab,
    Bluetooth,
    CliSessions,
    Controllers,
    Display,
    Explorer,
    Input,
    KeyRemap,
    Launcher,
    Lights,
    Memory,
    OsThemes,
    Pointz,
    RemoveApp,
    Session,
    Shot,
    Sound,
    Voice,
    WindowActions,
    Clipboard,
    ClipText,
    ClipCode,
    ClipImage,
    ClipFile,
    ClipSecret,
    ClipColor,
}

const LINE_LIFT: f32 = 0.25;

impl Mark {
    pub const ALL: [Self; 38] = [
        Self::Qol,
        Self::Settings,
        Self::Shortcuts,
        Self::Hotkeys,
        Self::Updates,
        Self::LinkedDevices,
        Self::Profiles,
        Self::Plugins,
        Self::Notifications,
        Self::App,
        Self::Link,
        Self::PluginAction,
        Self::AltTab,
        Self::Bluetooth,
        Self::CliSessions,
        Self::Controllers,
        Self::Display,
        Self::Explorer,
        Self::Input,
        Self::KeyRemap,
        Self::Launcher,
        Self::Lights,
        Self::Memory,
        Self::OsThemes,
        Self::Pointz,
        Self::RemoveApp,
        Self::Session,
        Self::Shot,
        Self::Sound,
        Self::Voice,
        Self::WindowActions,
        Self::Clipboard,
        Self::ClipText,
        Self::ClipCode,
        Self::ClipImage,
        Self::ClipFile,
        Self::ClipSecret,
        Self::ClipColor,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Qol => "qol",
            Self::Settings => "settings",
            Self::Shortcuts => "shortcuts",
            Self::Hotkeys => "hotkeys",
            Self::Updates => "updates",
            Self::LinkedDevices => "linked-devices",
            Self::Profiles => "profiles",
            Self::Plugins => "plugins",
            Self::Notifications => "notifications",
            Self::App => "app",
            Self::Link => "link",
            Self::PluginAction => "plugin-action",
            Self::AltTab => "alt-tab",
            Self::Bluetooth => "bluetooth",
            Self::CliSessions => "cli-sessions",
            Self::Controllers => "controllers",
            Self::Display => "display",
            Self::Explorer => "explorer",
            Self::Input => "input",
            Self::KeyRemap => "key-remap",
            Self::Launcher => "launcher",
            Self::Lights => "lights",
            Self::Memory => "memory",
            Self::OsThemes => "os-themes",
            Self::Pointz => "pointz",
            Self::RemoveApp => "remove-app",
            Self::Session => "session",
            Self::Shot => "shot",
            Self::Sound => "sound",
            Self::Voice => "voice",
            Self::WindowActions => "window-actions",
            Self::Clipboard => "clipboard",
            Self::ClipText => "clip-text",
            Self::ClipCode => "clip-code",
            Self::ClipImage => "clip-image",
            Self::ClipFile => "clip-file",
            Self::ClipSecret => "clip-secret",
            Self::ClipColor => "clip-color",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mark| mark.name() == name)
    }

    pub fn markup(self) -> &'static str {
        solid::markup(self)
    }

    pub fn svg(self, line: u32) -> String {
        self.markup()
            .replace("currentColor", &format!("#{line:06x}"))
    }

    pub fn line_on(ground: &Ground) -> u32 {
        qol_color::mix_rgb(ground.mark, ground.ink, LINE_LIFT)
    }

    pub fn app_svg(self) -> Option<String> {
        self.app_body().map(|body| {
            format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="256" height="256" viewBox="0 0 256 256">{body}</svg>"#
            )
        })
    }

    fn app_body(self) -> Option<&'static str> {
        match self {
            Self::Explorer => Some(
                r##"<path d="M22.4 22H104L125.6 51.7H230A18 18 0 0 1 248 69.7V216A18 18 0 0 1 230 234H26A18 18 0 0 1 8 216V36.4A14.4 14.4 0 0 1 22.4 22z" fill="#f3f2f0" stroke="#16171a" stroke-width="16" paint-order="stroke"/><path d="M168.2 80.7L127.2 153.7L59.1 195.3L100 127.8z" fill="#16171a" stroke="#16171a" stroke-width="4.1" stroke-linejoin="round"/><path d="M141.9 157.1L113.6 173.5L85.3 157.1L85.3 124.4L113.6 108L141.9 124.4z" fill="#f3f2f0" stroke="#16171a" stroke-width="9.5" stroke-linejoin="round"/><circle cx="113.6" cy="140.7" r="10.9" fill="#16171a"/>"##,
            ),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Mark;
    use std::collections::HashSet;

    #[test]
    fn every_mark_has_a_unique_name_that_reads_back() {
        let names: HashSet<_> = Mark::ALL.iter().map(|mark| mark.name()).collect();
        assert_eq!(names.len(), Mark::ALL.len());
        for mark in Mark::ALL {
            assert_eq!(Mark::from_name(mark.name()), Some(mark));
        }
    }

    #[test]
    fn unknown_names_are_not_marks() {
        assert_eq!(Mark::from_name("Bluetooth"), None);
        assert_eq!(Mark::from_name(""), None);
    }

    #[test]
    fn every_mark_is_a_standalone_svg() {
        for mark in Mark::ALL {
            let svg = mark.svg(0x123456);
            assert!(svg.starts_with("<svg xmlns="), "{}", mark.name());
            assert!(svg.contains("#123456"), "{}", mark.name());
            assert!(!svg.contains("currentColor"), "{}", mark.name());
            assert!(svg.ends_with("</svg>"), "{}", mark.name());
        }
    }

    #[test]
    fn app_art_is_a_standalone_svg_only_where_drawn() {
        let explorer = Mark::Explorer.app_svg().unwrap();
        assert!(explorer.starts_with("<svg xmlns="));
        assert!(explorer.ends_with("</svg>"));
        assert_eq!(Mark::Sound.app_svg(), None);
    }

    #[test]
    fn markup_takes_the_ink_it_is_drawn_with() {
        for mark in Mark::ALL {
            assert!(mark.markup().contains("currentColor"), "{}", mark.name());
        }
    }

    #[test]
    fn svg_is_the_one_drawing_in_the_given_ink() {
        for mark in Mark::ALL {
            assert_eq!(
                mark.svg(0x123456),
                mark.markup().replace("currentColor", "#123456"),
                "{}",
                mark.name()
            );
        }
    }

    #[test]
    fn lines_lift_the_mark_a_quarter_toward_the_ink() {
        let theme = crate::dark_theme_with_accent_key("violet");
        let pane = crate::Grounds::from_theme(theme.mode, theme.system).pane;
        assert_eq!(
            Mark::line_on(&pane),
            qol_color::mix_rgb(pane.mark, pane.ink, 0.25)
        );
        let band = crate::Grounds::from_theme(theme.mode, theme.system).band;
        assert_eq!(Mark::line_on(&band), band.ink);
    }

    #[test]
    fn every_mark_is_one_16_px_drawing() {
        for mark in Mark::ALL {
            let svg = mark.markup();
            assert!(
                svg.starts_with(r#"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16""#),
                "{}",
                mark.name()
            );
            assert!(svg.ends_with("</svg>"), "{}", mark.name());
        }
    }

    #[test]
    fn marks_take_the_ink_and_keep_their_masks_black_and_white() {
        for mark in Mark::ALL {
            let svg = mark.markup();
            let (head, drawing) = svg.split_once("</defs>").unwrap_or(("", svg));
            assert!(drawing.contains("currentColor"), "{}", mark.name());
            assert!(!drawing.contains("#000"), "{}", mark.name());
            if let Some((_, mask)) = head.split_once("<mask") {
                assert!(!mask.contains("currentColor"), "{}", mark.name());
            }
        }
    }
}
