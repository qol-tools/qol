use std::io;
use std::net::{IpAddr, SocketAddrV4};

use qol_process::{ProcessEntry, TcpListenerEntry};

pub(super) fn explain_bind_error(address: SocketAddrV4, error: io::Error) -> anyhow::Error {
    let in_use = match error.kind() {
        io::ErrorKind::AddrInUse => true,
        io::ErrorKind::PermissionDenied => false,
        _ => return error.into(),
    };
    let reason = describe(
        address,
        in_use,
        &qol_process::tcp_listeners().unwrap_or_default(),
        &qol_process::processes().unwrap_or_default(),
    );
    anyhow::Error::new(error).context(reason)
}

fn describe(
    address: SocketAddrV4,
    in_use: bool,
    listeners: &[TcpListenerEntry],
    processes: &[ProcessEntry],
) -> String {
    let port = address.port();
    let holder = listeners.iter().find(|entry| {
        entry.port == port
            && (entry.address.is_unspecified() || entry.address == IpAddr::V4(*address.ip()))
    });
    match holder.map(|entry| (entry.pid, entry.uid)) {
        Some((Some(pid), _)) => match processes.iter().find(|process| process.pid == pid) {
            Some(process) => format!(
                "port {port} is already in use by {} (pid {pid})",
                process.exe
            ),
            None => format!("port {port} is already in use by pid {pid}"),
        },
        Some((None, Some(uid))) => {
            format!("port {port} is already in use by a program of user id {uid}")
        }
        _ if in_use => format!("port {port} is already in use by another program"),
        _ => format!("port {port} is blocked by the system and no program listens on it"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn loopback(port: u16) -> SocketAddrV4 {
        SocketAddrV4::new(Ipv4Addr::LOCALHOST, port)
    }

    fn listener(port: u16, pid: Option<u32>, uid: Option<u32>) -> TcpListenerEntry {
        TcpListenerEntry {
            address: Ipv4Addr::LOCALHOST.into(),
            port,
            pid,
            uid,
        }
    }

    fn process(pid: u32, exe: &str) -> ProcessEntry {
        ProcessEntry {
            pid,
            parent: 1,
            exe: exe.to_owned(),
        }
    }

    #[test]
    fn a_port_this_process_holds_is_explained_with_this_process_as_the_holder() {
        let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = held.local_addr().unwrap().port();
        let error = std::net::TcpListener::bind(("127.0.0.1", port)).unwrap_err();

        let reason = explain_bind_error(loopback(port), error).to_string();

        assert!(
            reason.starts_with(&format!("port {port} is already in use by "))
                && reason.ends_with(&format!("(pid {})", std::process::id())),
            "{reason}"
        );
    }

    #[test]
    fn the_holder_is_named_by_program_and_pid() {
        let listeners = [
            listener(8080, Some(7), None),
            listener(43210, Some(4242), Some(1001)),
        ];
        let processes = [process(7, "web.exe"), process(4242, "qol-tray.exe")];

        assert_eq!(
            describe(loopback(43210), true, &listeners, &processes),
            "port 43210 is already in use by qol-tray.exe (pid 4242)"
        );
    }

    #[test]
    fn a_holder_without_a_listed_process_is_named_by_pid() {
        assert_eq!(
            describe(
                loopback(43210),
                true,
                &[listener(43210, Some(4242), None)],
                &[]
            ),
            "port 43210 is already in use by pid 4242"
        );
    }

    #[test]
    fn a_holder_this_account_cannot_inspect_is_named_by_user_id() {
        assert_eq!(
            describe(
                loopback(43210),
                true,
                &[listener(43210, None, Some(1001))],
                &[]
            ),
            "port 43210 is already in use by a program of user id 1001"
        );
    }

    #[test]
    fn an_unlisted_holder_is_still_reported() {
        assert_eq!(
            describe(loopback(43210), true, &[listener(8080, Some(7), None)], &[]),
            "port 43210 is already in use by another program"
        );
    }

    #[test]
    fn a_refused_port_nobody_listens_on_is_blamed_on_the_system() {
        assert_eq!(
            describe(loopback(43210), false, &[], &[]),
            "port 43210 is blocked by the system and no program listens on it"
        );
    }

    #[test]
    fn a_refused_port_with_a_listener_names_that_listener() {
        assert_eq!(
            describe(
                loopback(43210),
                false,
                &[listener(43210, Some(4242), None)],
                &[process(4242, "guard.exe")]
            ),
            "port 43210 is already in use by guard.exe (pid 4242)"
        );
    }

    #[test]
    fn only_a_holder_that_can_block_the_address_is_named() {
        let at = |address: [u8; 4], pid| TcpListenerEntry {
            address: IpAddr::from(address),
            port: 43210,
            pid: Some(pid),
            uid: None,
        };
        let processes = [process(7, "lan.exe"), process(8, "any.exe")];

        assert_eq!(
            describe(
                loopback(43210),
                true,
                &[at([192, 168, 1, 5], 7)],
                &processes
            ),
            "port 43210 is already in use by another program"
        );
        assert_eq!(
            describe(
                loopback(43210),
                true,
                &[at([192, 168, 1, 5], 7), at([0, 0, 0, 0], 8)],
                &processes
            ),
            "port 43210 is already in use by any.exe (pid 8)"
        );
    }
}
