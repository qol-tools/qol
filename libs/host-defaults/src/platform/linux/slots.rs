use qol_desktop_settings::desktop::DesktopEnvironment;
use qol_platform::LinuxDisplayBackend;

use super::session::Session;
use crate::report::{Lifetime, Owner, Report, Restore, SlotRoute, State};
use crate::role::Role;

const WAYLAND_REASON: &str =
    "Wayland needs the GlobalShortcuts portal, which qol does not use yet.";

struct Tool {
    keys: &'static [&'static str],
    owner: &'static str,
}

enum Answer {
    Route(SlotRoute),
    NoHostTool,
    Impossible(String),
    Held { owner: String, reason: String },
    Unknown(String),
}

const BARE_MODIFIERS: [&str; 4] = ["Super", "Ctrl", "Alt", "Shift"];

pub(super) fn route(session: &Session, role: Role) -> Option<SlotRoute> {
    match answer(session, role) {
        Answer::Route(route) => Some(route),
        Answer::NoHostTool | Answer::Impossible(_) | Answer::Held { .. } | Answer::Unknown(_) => {
            None
        }
    }
}

pub(super) fn probe(session: &Session, role: Role) -> Report {
    let report = |state, owner, triggers| Report {
        role,
        state,
        owner,
        triggers,
        kept: Vec::new(),
        restore: Restore::Exact,
        lifetimes: vec![Lifetime::PortableSession],
    };
    match answer(session, role) {
        Answer::Route(route) => report(State::Silent, Owner::Named(route.owner), route.keys),
        Answer::NoHostTool => report(State::Silent, Owner::NoHostTool, Vec::new()),
        Answer::Impossible(reason) => report(
            State::Impossible { reason },
            unreadable_owner(role),
            Vec::new(),
        ),
        Answer::Held { owner, reason } => report(
            State::Impossible { reason },
            Owner::Named(owner),
            Vec::new(),
        ),
        Answer::Unknown(reason) => report(State::Unknown { reason }, Owner::Unreadable, Vec::new()),
    }
}

fn unreadable_owner(role: Role) -> Owner {
    match role {
        Role::SlotClipboardHistory | Role::SlotDictation => Owner::NoHostTool,
        Role::FolderOpen
        | Role::FolderReveal
        | Role::SlotLauncher
        | Role::SlotWindowSwitcher
        | Role::SlotScreenshot => Owner::Unreadable,
    }
}

fn answer(session: &Session, role: Role) -> Answer {
    match session.display {
        LinuxDisplayBackend::Wayland => return Answer::Impossible(WAYLAND_REASON.to_string()),
        LinuxDisplayBackend::Unknown => {
            return Answer::Unknown("the display server could not be read".to_string())
        }
        LinuxDisplayBackend::X11 => {}
    }
    let tool = match session.desktop {
        DesktopEnvironment::Cinnamon => cinnamon_tool(role),
        DesktopEnvironment::Gnome => gnome_tool(role),
        DesktopEnvironment::Kde | DesktopEnvironment::Unknown => {
            return Answer::Unknown(unsupported_desktop(&session.raw_desktop))
        }
    };
    let Some(tool) = tool else {
        return Answer::NoHostTool;
    };
    if let Some(bare) = tool.keys.iter().find(|key| BARE_MODIFIERS.contains(key)) {
        return Answer::Held {
            owner: tool.owner.to_string(),
            reason: format!(
                "qol cannot answer {bare} pressed on its own yet, so {} keeps it.",
                tool.owner
            ),
        };
    }
    Answer::Route(SlotRoute {
        keys: tool.keys.iter().map(|key| key.to_string()).collect(),
        owner: tool.owner.to_string(),
    })
}

fn unsupported_desktop(raw: &str) -> String {
    if raw.is_empty() {
        return "slot keys are not built for an unnamed desktop yet".to_string();
    }
    format!("slot keys are not built for the {raw} desktop yet")
}

fn cinnamon_tool(role: Role) -> Option<Tool> {
    match role {
        Role::SlotLauncher => Some(Tool {
            keys: &["Super"],
            owner: "Cinnamon's menu",
        }),
        Role::SlotWindowSwitcher => Some(Tool {
            keys: &["Alt+Tab"],
            owner: "Cinnamon",
        }),
        Role::SlotScreenshot => Some(Tool {
            keys: &["Print"],
            owner: "GNOME Screenshot",
        }),
        Role::FolderOpen
        | Role::FolderReveal
        | Role::SlotClipboardHistory
        | Role::SlotDictation => None,
    }
}

fn gnome_tool(role: Role) -> Option<Tool> {
    match role {
        Role::SlotLauncher => Some(Tool {
            keys: &["Super"],
            owner: "Activities",
        }),
        Role::SlotWindowSwitcher => Some(Tool {
            keys: &["Alt+Tab"],
            owner: "GNOME Shell",
        }),
        Role::SlotScreenshot => Some(Tool {
            keys: &["Print"],
            owner: "GNOME Screenshot",
        }),
        Role::FolderOpen
        | Role::FolderReveal
        | Role::SlotClipboardHistory
        | Role::SlotDictation => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qol_hotkeys::grammar;

    const SLOTS: [Role; 5] = [
        Role::SlotLauncher,
        Role::SlotWindowSwitcher,
        Role::SlotScreenshot,
        Role::SlotClipboardHistory,
        Role::SlotDictation,
    ];

    fn x11(desktop: &str) -> Session {
        Session::from_parts(desktop, LinuxDisplayBackend::X11)
    }

    #[test]
    fn x11_desktops_route_the_keys_to_the_host_owner() {
        let cases = [
            (
                "X-Cinnamon",
                Role::SlotWindowSwitcher,
                "Alt+Tab",
                "Cinnamon",
            ),
            (
                "X-Cinnamon",
                Role::SlotScreenshot,
                "Print",
                "GNOME Screenshot",
            ),
            (
                "ubuntu:GNOME",
                Role::SlotWindowSwitcher,
                "Alt+Tab",
                "GNOME Shell",
            ),
            ("GNOME", Role::SlotScreenshot, "Print", "GNOME Screenshot"),
        ];
        for (desktop, role, key, owner) in cases {
            let session = x11(desktop);
            let route = route(&session, role).unwrap();
            assert_eq!(route.keys, vec![key.to_string()], "{desktop} {role:?}");
            assert_eq!(route.owner, owner, "{desktop} {role:?}");
            let report = probe(&session, role);
            assert_eq!(report.state, State::Silent);
            assert_eq!(report.owner, Owner::Named(owner.to_string()));
            assert_eq!(report.triggers, vec![key.to_string()]);
            assert_eq!(report.restore, Restore::Exact);
            assert_eq!(report.lifetimes, vec![Lifetime::PortableSession]);
        }
    }

    #[test]
    fn roles_without_a_host_tool_have_no_route_and_no_row() {
        for desktop in ["X-Cinnamon", "GNOME"] {
            for role in [Role::SlotClipboardHistory, Role::SlotDictation] {
                let session = x11(desktop);
                assert_eq!(route(&session, role), None);
                let report = probe(&session, role);
                assert_eq!(report.state, State::Silent);
                assert_eq!(report.owner, Owner::NoHostTool);
                assert!(report.triggers.is_empty());
            }
        }
    }

    #[test]
    fn wayland_is_impossible_for_every_slot() {
        for role in SLOTS {
            let session = Session::from_parts("GNOME", LinuxDisplayBackend::Wayland);
            assert_eq!(route(&session, role), None);
            let report = probe(&session, role);
            assert_eq!(
                report.state,
                State::Impossible {
                    reason: "Wayland needs the GlobalShortcuts portal, which qol does not use yet."
                        .to_string()
                }
            );
        }
    }

    #[test]
    fn an_unreadable_display_server_is_unknown_not_success() {
        let session = Session::from_parts("X-Cinnamon", LinuxDisplayBackend::Unknown);
        for role in SLOTS {
            assert!(matches!(probe(&session, role).state, State::Unknown { .. }));
            assert_eq!(route(&session, role), None);
        }
    }

    #[test]
    fn other_desktops_are_unknown_and_name_the_desktop() {
        let cases = [
            ("KDE", "slot keys are not built for the KDE desktop yet"),
            ("XFCE", "slot keys are not built for the XFCE desktop yet"),
            ("", "slot keys are not built for an unnamed desktop yet"),
        ];
        for (desktop, reason) in cases {
            let session = x11(desktop);
            for role in SLOTS {
                assert_eq!(route(&session, role), None);
                assert_eq!(
                    probe(&session, role).state,
                    State::Unknown {
                        reason: reason.to_string()
                    },
                    "{desktop} {role:?}"
                );
            }
        }
    }

    #[test]
    fn routed_keys_parse_in_the_hotkey_grammar() {
        for desktop in ["X-Cinnamon", "GNOME"] {
            for role in SLOTS {
                let Some(route) = route(&x11(desktop), role) else {
                    continue;
                };
                for key in route.keys {
                    assert!(grammar::parse(&key).is_some(), "{key}");
                }
            }
        }
    }

    #[test]
    fn the_launcher_is_impossible_because_a_bare_super_cannot_be_bound() {
        let cases = [
            (
                "X-Cinnamon",
                "Cinnamon's menu",
                "qol cannot answer Super pressed on its own yet, so Cinnamon's menu keeps it.",
            ),
            (
                "GNOME",
                "Activities",
                "qol cannot answer Super pressed on its own yet, so Activities keeps it.",
            ),
        ];
        for (desktop, owner, reason) in cases {
            let session = x11(desktop);
            assert_eq!(route(&session, Role::SlotLauncher), None, "{desktop}");
            let report = probe(&session, Role::SlotLauncher);
            assert_eq!(
                report.state,
                State::Impossible {
                    reason: reason.to_string()
                },
                "{desktop}"
            );
            assert_eq!(report.owner, Owner::Named(owner.to_string()), "{desktop}");
            assert!(report.triggers.is_empty(), "{desktop}");
        }
    }
}
