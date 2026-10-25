use qol_platform::LinuxDisplayBackend;

use super::session::Session;
use crate::report::{Blocker, BlockerKind};
use crate::role::Role;

const PORTAL_BUS_NAME: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const PORTAL_INTERFACE: &str = "org.freedesktop.portal.GlobalShortcuts";
const PROPERTIES_INTERFACE: &str = "org.freedesktop.DBus.Properties";
const PORTAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

const ABSENT_ERRORS: [&str; 5] = [
    "org.freedesktop.DBus.Error.ServiceUnknown",
    "org.freedesktop.DBus.Error.NameHasNoOwner",
    "org.freedesktop.DBus.Error.UnknownObject",
    "org.freedesktop.DBus.Error.UnknownInterface",
    "org.freedesktop.DBus.Error.UnknownProperty",
];

#[derive(Debug, PartialEq, Eq)]
enum PortalAnswer {
    Offered,
    Missing,
    Unreadable(String),
}

pub(super) fn current(session: &Session) -> Vec<Blocker> {
    blockers_for(session.display, read_portal)
}

fn blockers_for(display: LinuxDisplayBackend, read: impl FnOnce() -> PortalAnswer) -> Vec<Blocker> {
    match display {
        LinuxDisplayBackend::X11 | LinuxDisplayBackend::Unknown => Vec::new(),
        LinuxDisplayBackend::Wayland => match read() {
            PortalAnswer::Offered => Vec::new(),
            PortalAnswer::Missing => vec![Blocker {
                kind: BlockerKind::ShortcutPortalMissing,
                roles: Role::slot_roles(),
                detail: format!(
                    "xdg-desktop-portal does not offer {PORTAL_INTERFACE} on this session"
                ),
            }],
            PortalAnswer::Unreadable(reason) => {
                log::debug!("host defaults: GlobalShortcuts portal unreadable: {reason}");
                Vec::new()
            }
        },
    }
}

fn classify_error(name: &str) -> Option<PortalAnswer> {
    ABSENT_ERRORS
        .contains(&name)
        .then_some(PortalAnswer::Missing)
}

fn read_portal() -> PortalAnswer {
    let connection = match zbus::blocking::connection::Builder::session()
        .map(|builder| builder.method_timeout(PORTAL_TIMEOUT))
        .and_then(|builder| builder.build())
    {
        Ok(connection) => connection,
        Err(error) => return PortalAnswer::Unreadable(format!("session bus: {error}")),
    };
    let reply = connection.call_method(
        Some(PORTAL_BUS_NAME),
        PORTAL_PATH,
        Some(PROPERTIES_INTERFACE),
        "Get",
        &(PORTAL_INTERFACE, "version"),
    );
    match reply {
        Ok(_) => PortalAnswer::Offered,
        Err(zbus::Error::MethodError(name, _, _)) => classify_error(name.as_str())
            .unwrap_or_else(|| PortalAnswer::Unreadable(format!("portal answered {name}"))),
        Err(error) => PortalAnswer::Unreadable(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn kinds(display: LinuxDisplayBackend, answer: PortalAnswer) -> Vec<BlockerKind> {
        blockers_for(display, || answer)
            .into_iter()
            .map(|blocker| blocker.kind)
            .collect()
    }

    #[test]
    fn only_a_wayland_session_without_the_portal_is_blocked() {
        let cases = [
            (
                "wayland, portal offered",
                LinuxDisplayBackend::Wayland,
                PortalAnswer::Offered,
                false,
            ),
            (
                "wayland, portal missing",
                LinuxDisplayBackend::Wayland,
                PortalAnswer::Missing,
                true,
            ),
            (
                "wayland, bus unreadable",
                LinuxDisplayBackend::Wayland,
                PortalAnswer::Unreadable("no bus".to_string()),
                false,
            ),
            (
                "x11, portal missing",
                LinuxDisplayBackend::X11,
                PortalAnswer::Missing,
                false,
            ),
            (
                "unknown display",
                LinuxDisplayBackend::Unknown,
                PortalAnswer::Missing,
                false,
            ),
        ];
        for (label, display, answer, blocked) in cases {
            let expected = if blocked {
                vec![BlockerKind::ShortcutPortalMissing]
            } else {
                Vec::new()
            };
            assert_eq!(kinds(display, answer), expected, "case: {label}");
        }
    }

    #[test]
    fn the_blocker_stops_every_slot_role_and_names_the_portal() {
        let blockers = blockers_for(LinuxDisplayBackend::Wayland, || PortalAnswer::Missing);
        assert_eq!(blockers.len(), 1);
        assert_eq!(blockers[0].roles, Role::slot_roles());
        assert!(blockers[0].detail.contains(PORTAL_INTERFACE));
    }

    #[test]
    fn the_bus_is_only_read_on_wayland() {
        for display in [LinuxDisplayBackend::X11, LinuxDisplayBackend::Unknown] {
            let reads = Cell::new(0);
            blockers_for(display, || {
                reads.set(reads.get() + 1);
                PortalAnswer::Missing
            });
            assert_eq!(reads.get(), 0, "display: {display:?}");
        }
    }

    #[test]
    fn dbus_errors_classify_as_absent_or_unreadable() {
        let cases = [
            ("org.freedesktop.DBus.Error.ServiceUnknown", true),
            ("org.freedesktop.DBus.Error.NameHasNoOwner", true),
            ("org.freedesktop.DBus.Error.UnknownObject", true),
            ("org.freedesktop.DBus.Error.UnknownInterface", true),
            ("org.freedesktop.DBus.Error.UnknownProperty", true),
            ("org.freedesktop.DBus.Error.NoReply", false),
            ("org.freedesktop.DBus.Error.AccessDenied", false),
            ("", false),
        ];
        for (name, absent) in cases {
            assert_eq!(
                classify_error(name),
                absent.then_some(PortalAnswer::Missing),
                "error: {name:?}"
            );
        }
    }
}
