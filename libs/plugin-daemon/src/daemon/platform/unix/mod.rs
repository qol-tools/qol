//! Unix-domain transport adapter for resident plugin daemons: fd handoff from
//! qol-tray and the socket-file conventions of Unix hosts.

use std::fs;
use std::io::{self, ErrorKind};
use std::os::fd::{FromRawFd, RawFd};
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};

use qol_runtime::local_ipc::LocalListener;

mod platform;

use platform::is_listening_socket;

pub(in crate::daemon) fn fallback_socket_dir(use_tmpdir_env: bool) -> PathBuf {
    if use_tmpdir_env {
        std::env::var("TMPDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp"))
    } else {
        PathBuf::from("/tmp")
    }
}

pub(in crate::daemon) fn remove_socket_file(path: impl AsRef<Path>) {
    let path = path.as_ref();
    let Ok(meta) = fs::symlink_metadata(path) else {
        return;
    };
    if meta.file_type().is_socket() {
        let _ = fs::remove_file(path);
    }
}

/// A daemon only ever receives this variable from the qol-tray that pre-bound
/// the fd for it. Anything else (an unrelated ancestor's leftover, a stale
/// number) names an fd that is not a listening socket here, so it is ignored
/// and the daemon binds its own socket path instead of dying on it.
pub(in crate::daemon) fn inherited_listener() -> Option<LocalListener> {
    let raw = std::env::var(qol_conventions::ENV_DAEMON_LISTENER_FD).ok()?;
    match listener_from_fd_str(&raw) {
        Ok(listener) => Some(listener),
        Err(error) => {
            log::warn!(
                "ignoring {}={raw}: {error}; binding the socket path instead",
                qol_conventions::ENV_DAEMON_LISTENER_FD
            );
            std::env::remove_var(qol_conventions::ENV_DAEMON_LISTENER_FD);
            None
        }
    }
}

fn listener_from_fd_str(raw: &str) -> io::Result<LocalListener> {
    let fd: RawFd = raw.parse().map_err(|_| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!(
                "malformed {}: {raw:?}",
                qol_conventions::ENV_DAEMON_LISTENER_FD
            ),
        )
    })?;
    if !is_listening_socket(fd) {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            format!("fd {fd} is not a listening socket in this process"),
        ));
    }
    restore_cloexec(fd)?;
    Ok(unsafe { LocalListener::from_raw_fd(fd) })
}

/// A pre-bound port fd is adoptable when it is a socket of a kind that can
/// already be serving: a stream socket must be listening, but a datagram
/// socket never is - `SO_ACCEPTCONN` is 0 for every UDP socket, so judging
/// one by that alone rejects a perfectly good handoff and forces the daemon
/// to rebind a port qol-tray still holds.
fn is_adoptable_socket(fd: RawFd) -> bool {
    match socket_opt(fd, libc::SO_TYPE) {
        Some(libc::SOCK_DGRAM) => true,
        Some(libc::SOCK_STREAM) => is_listening_socket(fd),
        _ => false,
    }
}

fn socket_opt(fd: RawFd, option: libc::c_int) -> Option<libc::c_int> {
    let mut value: libc::c_int = 0;
    let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
    let rc = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            option,
            (&mut value as *mut libc::c_int).cast::<libc::c_void>(),
            &mut len,
        )
    };
    (rc == 0).then_some(value)
}

/// qol-tray clears CLOEXEC on a pre-bound fd so it survives the exec into
/// this daemon's binary. That cleared flag would otherwise keep propagating
/// into every further child this daemon spawns (e.g. a launched app, a
/// terminal, ffmpeg), leaking the listener past this process. Callers must
/// invoke this immediately after adopting any fd handed off via the
/// `QOL_TRAY_DAEMON_*_FD` env vars, before wrapping it in a socket type.
pub fn restore_cloexec(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    let set = unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) };
    if set < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Looks up the fd qol-tray pre-bound for a named extra port (declared via
/// `[[daemon.extra_ports]]` in plugin.toml), e.g. `inherited_port_fd("discovery")`
/// for a port named `discovery`. Returns `None` if qol-tray didn't pre-bind
/// this port - the caller should fall back to binding it directly.
pub fn inherited_port_fd(name: &str) -> Option<RawFd> {
    let env_name = format!(
        "{}_{}",
        qol_conventions::ENV_DAEMON_PORT_FD,
        name.to_uppercase()
    );
    fd_from_env(&env_name)
}

/// Looks up the fd qol-tray pre-bound for the daemon's single top-level
/// `port` (declared as `port = ...` directly under `[daemon]` in
/// plugin.toml, as opposed to a named `[[daemon.extra_ports]]` entry).
/// Returns `None` if qol-tray didn't pre-bind it - the caller should fall
/// back to binding it directly.
pub fn inherited_primary_port_fd() -> Option<RawFd> {
    fd_from_env(qol_conventions::ENV_DAEMON_PORT_FD)
}

fn fd_from_env(env_name: &str) -> Option<RawFd> {
    let raw = std::env::var(env_name).ok()?;
    let fd: RawFd = raw.parse().ok()?;
    if !is_adoptable_socket(fd) {
        log::warn!("ignoring {env_name}={raw}: fd {fd} is not a pre-bound socket in this process");
        std::env::remove_var(env_name);
        return None;
    }
    Some(fd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::tests::{daemon_listener_fd_env_lock, fallback_config, temp_socket_name};
    use crate::daemon::*;
    use std::os::unix::net::{UnixListener, UnixStream};

    #[test]
    fn bind_listener_uses_inherited_fd_when_env_var_present() {
        use std::os::fd::IntoRawFd;

        let _lock = daemon_listener_fd_env_lock();
        let socket_name = temp_socket_name("inherited");
        let path = PathBuf::from("/tmp").join(socket_name);
        let _ = fs::remove_file(&path);
        let pre_bound = UnixListener::bind(&path).unwrap();
        let fd = pre_bound.into_raw_fd();
        std::env::set_var(qol_conventions::ENV_DAEMON_LISTENER_FD, fd.to_string());

        let config = fallback_config(temp_socket_name("unused-when-inherited"));
        let result = bind_listener(&config);

        std::env::remove_var(qol_conventions::ENV_DAEMON_LISTENER_FD);

        let (_listener, bound_path) = result.unwrap();
        assert_eq!(
            bound_path, None,
            "an inherited listener does not own its socket path and must not unlink it"
        );
        remove_socket_file(path);
    }

    /// The env var is only meaningful for the daemon qol-tray bound the fd
    /// for. Leaked into any other process (a terminal opened from a plugin,
    /// then `qol dev`, then every daemon) it named an fd that was closed or
    /// unrelated, and daemons died on it instead of binding their own socket.
    #[test]
    fn bind_listener_ignores_an_inherited_fd_that_is_not_a_listening_socket() {
        use std::os::fd::AsRawFd;

        let _lock = daemon_listener_fd_env_lock();
        let not_a_socket = fs::File::open("/dev/null").unwrap();
        let socket_name = temp_socket_name("fallback-after-bogus-fd");
        let config = fallback_config(socket_name);
        let expected = PathBuf::from("/tmp").join(socket_name);
        let _ = fs::remove_file(&expected);

        for bogus in [not_a_socket.as_raw_fd().to_string(), "999999".to_owned()] {
            std::env::set_var(qol_conventions::ENV_DAEMON_LISTENER_FD, &bogus);
            let (_listener, bound_path) = bind_listener(&config).unwrap();
            assert_eq!(bound_path.as_deref(), Some(expected.as_path()));
            assert!(
                std::env::var_os(qol_conventions::ENV_DAEMON_LISTENER_FD).is_none(),
                "a rejected handoff must not linger and suppress socket cleanup"
            );
            remove_socket_file(expected.clone());
        }
    }

    #[test]
    fn listener_from_fd_str_rejects_malformed_value() {
        let error = listener_from_fd_str("not-a-number").unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn listening_socket_validation_preserves_pending_connections_and_rejects_other_fds() {
        use std::os::fd::{AsRawFd, OwnedFd};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let _pending = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (connected, _peer) = UnixStream::pair().unwrap();
        let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let file = fs::File::open("/dev/null").unwrap();
        let raw = unsafe { libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0) };
        assert!(raw >= 0);
        let unbound = unsafe { OwnedFd::from_raw_fd(raw) };

        for (label, fd, expected) in [
            ("listener", listener.as_raw_fd(), true),
            ("connected stream", connected.as_raw_fd(), false),
            ("unbound stream", unbound.as_raw_fd(), false),
            ("datagram", udp.as_raw_fd(), false),
            ("file", file.as_raw_fd(), false),
            ("invalid", -1, false),
        ] {
            assert_eq!(is_listening_socket(fd), expected, "{label}");
            if fd >= 0 {
                assert!(unsafe { libc::fcntl(fd, libc::F_GETFD) } >= 0, "{label}");
            }
        }

        listener.set_nonblocking(true).unwrap();
        assert!(
            listener.accept().is_ok(),
            "validation must not accept clients"
        );
        assert!(is_adoptable_socket(udp.as_raw_fd()));
        assert!(!is_adoptable_socket(unbound.as_raw_fd()));
        assert!(!is_adoptable_socket(connected.as_raw_fd()));
    }

    fn fd_has_cloexec(fd: RawFd) -> bool {
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        assert!(flags >= 0, "fd {fd} must be open");
        flags & libc::FD_CLOEXEC != 0
    }

    #[test]
    fn restore_cloexec_sets_the_close_on_exec_flag() {
        use std::os::fd::IntoRawFd;

        let path = PathBuf::from(format!(
            "/tmp/qol-plugin-daemon-test-restore-cloexec-{}.sock",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let fd = listener.into_raw_fd();
        unsafe { libc::fcntl(fd, libc::F_SETFD, 0) };
        assert!(
            !fd_has_cloexec(fd),
            "test setup must start with cloexec cleared"
        );

        restore_cloexec(fd).unwrap();

        assert!(fd_has_cloexec(fd), "restore_cloexec must set FD_CLOEXEC");
        unsafe { libc::close(fd) };
        let _ = fs::remove_file(&path);
    }

    // Regression test for the leak this whole redesign was meant to close:
    // qol-tray clears CLOEXEC so the fd survives its own exec into the
    // daemon. Once adopted here, that cleared flag must not keep propagating
    // into every further child the daemon spawns.
    #[test]
    fn listener_from_fd_str_restores_cloexec_on_the_adopted_fd() {
        use std::os::fd::IntoRawFd;

        let path = PathBuf::from(format!(
            "/tmp/qol-plugin-daemon-test-adopt-cloexec-{}.sock",
            std::process::id()
        ));
        let _ = fs::remove_file(&path);
        let pre_bound = UnixListener::bind(&path).unwrap();
        let fd = pre_bound.into_raw_fd();
        unsafe { libc::fcntl(fd, libc::F_SETFD, 0) };

        let listener = listener_from_fd_str(&fd.to_string()).unwrap();

        assert!(
            fd_has_cloexec(fd),
            "adopting an inherited fd must re-arm cloexec so it can't leak \
             into a further child this daemon spawns"
        );
        drop(listener);
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn inherited_port_fd_reads_the_named_env_var() {
        use std::os::fd::AsRawFd;

        let _lock = daemon_listener_fd_env_lock();
        let env_name = format!("{}_TESTPORT", qol_conventions::ENV_DAEMON_PORT_FD);
        let pre_bound = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        std::env::set_var(&env_name, pre_bound.as_raw_fd().to_string());

        let fd = inherited_port_fd("testport");

        std::env::remove_var(&env_name);
        assert_eq!(fd, Some(pre_bound.as_raw_fd()));
    }

    // Regression test: `SO_ACCEPTCONN` is 0 for every UDP socket, so judging
    // a pre-bound port fd by "is it listening" rejected every datagram
    // handoff. qol-pointz then rebound its discovery port, hit
    // EADDRINUSE against the qol-tray-held socket, and crash-looped.
    #[test]
    fn inherited_port_fd_adopts_a_pre_bound_udp_socket() {
        use std::os::fd::AsRawFd;

        let _lock = daemon_listener_fd_env_lock();
        let env_name = format!("{}_TESTUDPPORT", qol_conventions::ENV_DAEMON_PORT_FD);
        let pre_bound = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        std::env::set_var(&env_name, pre_bound.as_raw_fd().to_string());

        let fd = inherited_port_fd("testudpport");

        std::env::remove_var(&env_name);
        assert_eq!(
            fd,
            Some(pre_bound.as_raw_fd()),
            "a datagram socket is never listening, but it is still a valid handoff"
        );
    }

    #[test]
    fn inherited_port_fd_ignores_a_number_that_is_not_a_listening_socket() {
        let _lock = daemon_listener_fd_env_lock();
        let env_name = format!("{}_BOGUSPORT", qol_conventions::ENV_DAEMON_PORT_FD);
        std::env::set_var(&env_name, "999999");

        let fd = inherited_port_fd("bogusport");

        assert_eq!(fd, None, "a leaked or stale fd number must not be adopted");
        assert!(
            std::env::var_os(&env_name).is_none(),
            "a rejected handoff variable is dropped so nothing downstream trusts it"
        );
    }

    #[test]
    fn inherited_port_fd_returns_none_when_env_var_absent() {
        let _lock = daemon_listener_fd_env_lock();
        let env_name = format!("{}_ABSENTPORT", qol_conventions::ENV_DAEMON_PORT_FD);
        std::env::remove_var(&env_name);

        assert_eq!(inherited_port_fd("absentport"), None);
    }

    #[test]
    fn inherited_port_fd_returns_none_for_malformed_value() {
        let _lock = daemon_listener_fd_env_lock();
        let env_name = format!("{}_MALFORMEDPORT", qol_conventions::ENV_DAEMON_PORT_FD);
        std::env::set_var(&env_name, "not-a-number");

        let fd = inherited_port_fd("malformedport");

        std::env::remove_var(&env_name);
        assert_eq!(
            fd, None,
            "a malformed port fd falls back to direct binding rather than propagating an error"
        );
    }

    #[test]
    fn inherited_primary_port_fd_reads_the_unsuffixed_env_var() {
        use std::os::fd::AsRawFd;

        let _lock = daemon_listener_fd_env_lock();
        let pre_bound = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        std::env::set_var(
            qol_conventions::ENV_DAEMON_PORT_FD,
            pre_bound.as_raw_fd().to_string(),
        );

        let fd = inherited_primary_port_fd();

        std::env::remove_var(qol_conventions::ENV_DAEMON_PORT_FD);
        assert_eq!(fd, Some(pre_bound.as_raw_fd()));
    }

    #[test]
    fn inherited_primary_port_fd_returns_none_when_env_var_absent() {
        let _lock = daemon_listener_fd_env_lock();
        std::env::remove_var(qol_conventions::ENV_DAEMON_PORT_FD);

        assert_eq!(inherited_primary_port_fd(), None);
    }
}
