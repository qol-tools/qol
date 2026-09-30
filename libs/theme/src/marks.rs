use crate::Ground;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mark {
    Qol,
    Settings,
    Shortcuts,
    Hotkeys,
    Updates,
    LinkedDevices,
    App,
    Link,
    PluginAction,
    AltTab,
    Bluetooth,
    CliSessions,
    Controllers,
    Display,
    IdeCheckout,
    KeyRemap,
    Launcher,
    Lights,
    Memory,
    OsThemes,
    Pointz,
    RemoveApp,
    Shot,
    Sound,
    Voice,
    WindowActions,
}

const LINE_LIFT: f32 = 0.25;

impl Mark {
    pub const ALL: [Self; 26] = [
        Self::Qol,
        Self::Settings,
        Self::Shortcuts,
        Self::Hotkeys,
        Self::Updates,
        Self::LinkedDevices,
        Self::App,
        Self::Link,
        Self::PluginAction,
        Self::AltTab,
        Self::Bluetooth,
        Self::CliSessions,
        Self::Controllers,
        Self::Display,
        Self::IdeCheckout,
        Self::KeyRemap,
        Self::Launcher,
        Self::Lights,
        Self::Memory,
        Self::OsThemes,
        Self::Pointz,
        Self::RemoveApp,
        Self::Shot,
        Self::Sound,
        Self::Voice,
        Self::WindowActions,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Qol => "qol",
            Self::Settings => "settings",
            Self::Shortcuts => "shortcuts",
            Self::Hotkeys => "hotkeys",
            Self::Updates => "updates",
            Self::LinkedDevices => "linked-devices",
            Self::App => "app",
            Self::Link => "link",
            Self::PluginAction => "plugin-action",
            Self::AltTab => "alt-tab",
            Self::Bluetooth => "bluetooth",
            Self::CliSessions => "cli-sessions",
            Self::Controllers => "controllers",
            Self::Display => "display",
            Self::IdeCheckout => "ide-checkout",
            Self::KeyRemap => "key-remap",
            Self::Launcher => "launcher",
            Self::Lights => "lights",
            Self::Memory => "memory",
            Self::OsThemes => "os-themes",
            Self::Pointz => "pointz",
            Self::RemoveApp => "remove-app",
            Self::Shot => "shot",
            Self::Sound => "sound",
            Self::Voice => "voice",
            Self::WindowActions => "window-actions",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mark| mark.name() == name)
    }

    pub fn markup(self) -> String {
        self.document("currentColor")
    }

    pub fn svg(self, line: u32) -> String {
        self.document(&format!("#{line:06x}"))
    }

    pub fn line_on(ground: &Ground) -> u32 {
        qol_color::mix_rgb(ground.mark, ground.ink, LINE_LIFT)
    }

    fn document(self, stroke: &str) -> String {
        format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 48 48" fill="none" stroke="{stroke}" stroke-width="4" stroke-linecap="round" stroke-linejoin="round">{}</svg>"#,
            self.body()
        )
    }

    fn body(self) -> &'static str {
        match self {
            Self::Qol => {
                r#"<path d="M12.45 8.2A6 6 0 1 1 8.2 12.45"/><path d="M18.24 18.24L36 36"/><path stroke-width="2.5" d="M37 11L23 25"/><path stroke-width="6" d="M21 27l-9 9"/>"#
            }
            Self::Settings => {
                r#"<path d="M21 11.4L20.9 6.3L27.1 6.3L27 11.4L30.8 12.9L34.4 9.3L38.7 13.6L35.1 17.2L36.6 21L41.7 20.9L41.7 27.1L36.6 27L35.1 30.8L38.7 34.4L34.4 38.7L30.8 35.1L27 36.6L27.1 41.7L20.9 41.7L21 36.6L17.2 35.1L13.6 38.7L9.3 34.4L12.9 30.8L11.4 27L6.3 27.1L6.3 20.9L11.4 21L12.9 17.2L9.3 13.6L13.6 9.3L17.2 12.9Z"/><circle cx="24" cy="24" r="5.5"/>"#
            }
            Self::Shortcuts => {
                r#"<rect x="7" y="7" width="34" height="34" rx="8"/><path d="M18 32v-4a8 8 0 0 1 8-8h7"/><path d="M28 15l5 5-5 5"/>"#
            }
            Self::Hotkeys => {
                r#"<rect x="6" y="9" width="36" height="30" rx="7"/><path d="M26 14l-7 11h9l-6 10"/>"#
            }
            Self::Updates => {
                r#"<path d="M38.5 19A15 15 0 0 0 11 15"/><path d="M10 7v8h8"/><path d="M9.5 29A15 15 0 0 0 37 33"/><path d="M38 41v-8h-8"/>"#
            }
            Self::LinkedDevices => {
                r#"<rect x="5" y="9" width="27" height="20" rx="3"/><path d="M13 37h11M18.5 29v8"/><rect x="34" y="17" width="10" height="21" rx="3"/><path stroke-width="4" d="M39 33h.01"/>"#
            }
            Self::App => {
                r#"<rect x="8" y="8" width="13" height="13" rx="3.5"/><rect x="27" y="8" width="13" height="13" rx="3.5"/><rect x="8" y="27" width="13" height="13" rx="3.5"/><rect x="27" y="27" width="13" height="13" rx="3.5"/>"#
            }
            Self::Link => {
                r#"<path d="M21 27l6-6"/><path d="M25 13.5l3.5-3.5a8 8 0 0 1 11.3 11.3L36.3 24.8"/><path d="M23 34.5L19.5 38a8 8 0 0 1-11.3-11.3l3.5-3.5"/>"#
            }
            Self::PluginAction => {
                r#"<path d="M18 6v9M30 6v9"/><path d="M11 15h26v8a13 13 0 0 1-26 0z"/><path d="M24 36v7"/>"#
            }
            Self::AltTab => {
                r#"<path d="M17 27H9a4 4 0 0 1-4-4V11a4 4 0 0 1 4-4h18a4 4 0 0 1 4 4v8"/><rect x="17" y="19" width="26" height="21" rx="4"/><path d="M17 26h26"/>"#
            }
            Self::Bluetooth => r#"<path d="M14 16l19 16-9 9V7l9 9-19 16"/>"#,
            Self::CliSessions => {
                r#"<rect x="5" y="8" width="38" height="32" rx="5"/><path d="M13 18l6 6-6 6"/><path d="M23 31h11"/>"#
            }
            Self::Controllers => {
                r#"<path d="M15 14h18a8 8 0 0 1 7.8 6.3l2.4 11a5 5 0 0 1-8.6 4.4L30 31H18l-4.6 4.7a5 5 0 0 1-8.6-4.4l2.4-11A8 8 0 0 1 15 14z"/><path d="M15 20v7M11.5 23.5h7"/><path stroke-width="4" d="M32 21h.01"/><path stroke-width="4" d="M36 26h.01"/>"#
            }
            Self::Display => {
                r#"<rect x="4" y="7" width="40" height="28" rx="4"/><path d="M17 41h14M24 35v6"/><circle cx="24" cy="21" r="4"/><path stroke-width="2.5" d="M31.5 21L33.5 21M29.3 26.3L30.7 27.7M24 28.5L24 30.5M18.7 26.3L17.3 27.7M16.5 21L14.5 21M18.7 15.7L17.3 14.3M24 13.5L24 11.5M29.3 15.7L30.7 14.3"/>"#
            }
            Self::IdeCheckout => {
                r#"<circle cx="15" cy="11" r="4"/><circle cx="15" cy="37" r="4"/><circle cx="33" cy="17" r="4"/><path d="M15 15v18"/><path d="M33 21v1a8 8 0 0 1-8 8h-2a8 8 0 0 0-8 3"/>"#
            }
            Self::KeyRemap => {
                r#"<rect x="5" y="5" width="17" height="17" rx="4.5"/><rect x="26" y="26" width="17" height="17" rx="4.5"/><path d="M27 11h4a5 5 0 0 1 5 5v4"/><path d="M32 17l4 4 4-4"/><path d="M21 37h-4a5 5 0 0 1-5-5v-4"/><path d="M16 31l-4-4-4 4"/>"#
            }
            Self::Launcher => {
                r#"<rect x="4" y="13" width="40" height="22" rx="8"/><path d="M13 19.5l4.5 4.5-4.5 4.5"/><path stroke-width="2.5" d="M24 19v10"/>"#
            }
            Self::Lights => {
                r#"<path d="M17.5 30.5a11 11 0 1 1 13 0c-1.6 1.3-2.5 3-2.5 5v.5h-8v-.5c0-2-.9-3.7-2.5-5z"/><path d="M20 42h8"/>"#
            }
            Self::Memory => {
                r#"<path d="M24 11a6 6 0 0 0-11 1.5 6.5 6.5 0 0 0-5 10 6.5 6.5 0 0 0 2.5 10 6 6 0 0 0 9.5 5.5A4.5 4.5 0 0 0 24 36z"/><path d="M24 11a6 6 0 0 1 11 1.5 6.5 6.5 0 0 1 5 10 6.5 6.5 0 0 1-2.5 10 6 6 0 0 1-9.5 5.5A4.5 4.5 0 0 1 24 36z"/><path d="M24 11v25"/><path stroke-width="2.5" d="M17 20h-3M31 20h3M18 29h-4M30 29h4"/>"#
            }
            Self::OsThemes => {
                r#"<path d="M24 6a18 18 0 0 0 0 36c2 0 3.2-1.4 3.2-3.1 0-.9-.4-1.6-.9-2.3-.6-.7-.9-1.4-.9-2.3 0-1.8 1.4-3.3 3.3-3.3h3.8A11.5 11.5 0 0 0 44 19.5C44 12 35 6 24 6z"/><path stroke-width="4" d="M14.5 22h.01"/><path stroke-width="4" d="M19 14h.01"/><path stroke-width="4" d="M28.5 13.5h.01"/><path stroke-width="4" d="M35 20h.01"/>"#
            }
            Self::Pointz => {
                r#"<rect x="13" y="4" width="22" height="40" rx="5"/><circle cx="24" cy="22" r="3"/><circle cx="24" cy="22" r="8.5"/><path d="M21 38h6"/>"#
            }
            Self::RemoveApp => {
                r#"<path d="M8 13h32"/><path d="M18 13V9.5A2.5 2.5 0 0 1 20.5 7h7A2.5 2.5 0 0 1 30 9.5V13"/><path d="M12 13l1.8 25.3a4 4 0 0 0 4 3.7h12.4a4 4 0 0 0 4-3.7L36 13"/><path d="M20 21v13M28 21v13"/>"#
            }
            Self::Shot => {
                r#"<path d="M6 16V10a4 4 0 0 1 4-4h6M32 6h6a4 4 0 0 1 4 4v6M42 32v6a4 4 0 0 1-4 4h-6M16 42h-6a4 4 0 0 1-4-4v-6"/><circle cx="24" cy="24" r="7.5"/>"#
            }
            Self::Sound => {
                r#"<path d="M7 18.5h7.5L25 10v28l-10.5-8.5H7z"/><path d="M31 18a8.5 8.5 0 0 1 0 12"/><path d="M36 12.5a16 16 0 0 1 0 23"/>"#
            }
            Self::Voice => {
                r#"<rect x="17" y="5" width="14" height="24" rx="7"/><path d="M10 22a14 14 0 0 0 28 0"/><path d="M24 36v7"/>"#
            }
            Self::WindowActions => {
                r#"<rect x="5" y="8" width="38" height="32" rx="5"/><path d="M24 8v32M24 24h19"/>"#
            }
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
            assert!(svg.contains(r##"stroke="#123456""##), "{}", mark.name());
            assert!(svg.ends_with("</svg>"), "{}", mark.name());
        }
    }

    #[test]
    fn markup_takes_the_ink_it_is_drawn_with() {
        for mark in Mark::ALL {
            assert!(
                mark.markup().contains(r#"stroke="currentColor""#),
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
}
