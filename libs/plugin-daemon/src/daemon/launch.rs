use std::io::{self, ErrorKind};
use std::path::PathBuf;

pub fn launched_as_daemon(args: &[String]) -> bool {
    args.is_empty() && handed_socket().is_ok()
}

pub fn handed_socket() -> io::Result<PathBuf> {
    std::env::var_os(qol_conventions::ENV_DAEMON_SOCKET)
        .map(PathBuf::from)
        .ok_or_else(|| {
            io::Error::new(
                ErrorKind::NotFound,
                format!("{} is not set", qol_conventions::ENV_DAEMON_SOCKET),
            )
        })
}

pub fn asked_to_replace_existing() -> bool {
    std::env::var_os(qol_conventions::ENV_DAEMON_REPLACE_EXISTING).is_some()
}
