use std::io;

use qol_process::{ProcessEntry, TcpListenerEntry};

pub(super) fn explain_bind_error(port: u16, error: io::Error) -> anyhow::Error {
    let in_use = match error.kind() {
        io::ErrorKind::AddrInUse => true,
        io::ErrorKind::PermissionDenied => false,
        _ => return error.into(),
    };
    let reason = describe(
        port,
        in_use,
        &qol_process::tcp_listeners().unwrap_or_default(),
        &qol_process::processes().unwrap_or_default(),
    );
    anyhow::Error::new(error).context(reason)
}

fn describe(
    port: u16,
    in_use: bool,
    listeners: &[TcpListenerEntry],
    processes: &[ProcessEntry],
) -> String {
    let holder = listeners.iter().find(|entry| entry.port == port);
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

    fn listener(port: u16, pid: Option<u32>, uid: Option<u32>) -> TcpListenerEntry {
        TcpListenerEntry { port, pid, uid }
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

        let reason = explain_bind_error(port, error).to_string();

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
            describe(43210, true, &listeners, &processes),
            "port 43210 is already in use by qol-tray.exe (pid 4242)"
        );
    }

    #[test]
    fn a_holder_without_a_listed_process_is_named_by_pid() {
        assert_eq!(
            describe(43210, true, &[listener(43210, Some(4242), None)], &[]),
            "port 43210 is already in use by pid 4242"
        );
    }

    #[test]
    fn a_holder_this_account_cannot_inspect_is_named_by_user_id() {
        assert_eq!(
            describe(43210, true, &[listener(43210, None, Some(1001))], &[]),
            "port 43210 is already in use by a program of user id 1001"
        );
    }

    #[test]
    fn an_unlisted_holder_is_still_reported() {
        assert_eq!(
            describe(43210, true, &[listener(8080, Some(7), None)], &[]),
            "port 43210 is already in use by another program"
        );
    }

    #[test]
    fn a_refused_port_nobody_listens_on_is_blamed_on_the_system() {
        assert_eq!(
            describe(43210, false, &[], &[]),
            "port 43210 is blocked by the system and no program listens on it"
        );
    }

    #[test]
    fn a_refused_port_with_a_listener_names_that_listener() {
        assert_eq!(
            describe(
                43210,
                false,
                &[listener(43210, Some(4242), None)],
                &[process(4242, "guard.exe")]
            ),
            "port 43210 is already in use by guard.exe (pid 4242)"
        );
    }
}
