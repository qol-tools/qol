use super::unix_common::{redirect_to_null, spawn_console_guard};

pub(in crate::logging::console_guard) fn guard_console_pipes() {
    let live = redirect_dead_fds_now(vec![libc::STDOUT_FILENO, libc::STDERR_FILENO]);
    if live.is_empty() {
        return;
    }
    spawn_console_guard(move || watch_fds(live));
}

pub(super) fn watch_fds(mut fds: Vec<libc::c_int>) {
    while !fds.is_empty() {
        fds = poll_dead_fds(fds, -1);
    }
}

pub(super) fn redirect_dead_fds_now(fds: Vec<libc::c_int>) -> Vec<libc::c_int> {
    poll_dead_fds(fds, 0)
}

fn poll_dead_fds(fds: Vec<libc::c_int>, timeout_ms: libc::c_int) -> Vec<libc::c_int> {
    let mut poll_fds: Vec<libc::pollfd> = fds
        .iter()
        .map(|&fd| libc::pollfd {
            fd,
            events: 0,
            revents: 0,
        })
        .collect();
    let rc = unsafe {
        libc::poll(
            poll_fds.as_mut_ptr(),
            poll_fds.len() as libc::nfds_t,
            timeout_ms,
        )
    };
    if rc < 0 {
        if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            return fds;
        }
        return Vec::new();
    }
    let mut live = Vec::new();
    for poll_fd in &poll_fds {
        if poll_fd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            redirect_to_null(poll_fd.fd);
        } else {
            live.push(poll_fd.fd);
        }
    }
    live
}
