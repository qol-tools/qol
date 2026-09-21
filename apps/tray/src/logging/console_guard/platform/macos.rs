use super::unix_common::{redirect_to_null, spawn_console_guard};

pub(in crate::logging::console_guard) fn guard_console_pipes() {
    let live = redirect_dead_fds_now(vec![libc::STDOUT_FILENO, libc::STDERR_FILENO]);
    if live.is_empty() {
        return;
    }
    spawn_console_guard(move || watch_fds(live));
}

pub(super) fn redirect_dead_fds_now(fds: Vec<libc::c_int>) -> Vec<libc::c_int> {
    monitor_fds(fds, false)
}

pub(super) fn watch_fds(fds: Vec<libc::c_int>) {
    monitor_fds(fds, true);
}

fn monitor_fds(mut fds: Vec<libc::c_int>, keep_watching: bool) -> Vec<libc::c_int> {
    if fds.is_empty() {
        return fds;
    }
    let queue = unsafe { libc::kqueue() };
    if queue < 0 {
        return fds;
    }
    let changes: Vec<_> = fds.iter().copied().map(pipe_write_filter).collect();
    let mut events: Vec<_> = (0..fds.len()).map(|_| empty_kqueue_event()).collect();
    let timeout = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    let mut register = true;

    // Drain current write readiness once, then wait for EV_CLEAR state changes such as EV_EOF.
    loop {
        let (changes_ptr, change_count, timeout_ptr) = if register {
            (
                changes.as_ptr(),
                changes.len(),
                std::ptr::from_ref(&timeout),
            )
        } else {
            (std::ptr::null(), 0, std::ptr::null())
        };
        let count = unsafe {
            libc::kevent(
                queue,
                changes_ptr,
                change_count as libc::c_int,
                events.as_mut_ptr(),
                events.len() as libc::c_int,
                timeout_ptr,
            )
        };
        if count < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        register = false;
        for event in &events[..count as usize] {
            let flags = event.flags;
            let fd = event.ident as libc::c_int;
            if flags & libc::EV_EOF != 0 && fds.contains(&fd) {
                redirect_to_null(fd);
                fds.retain(|&live_fd| live_fd != fd);
            }
        }
        if !keep_watching || fds.is_empty() {
            break;
        }
    }
    unsafe { libc::close(queue) };
    fds
}

fn pipe_write_filter(fd: libc::c_int) -> libc::kevent {
    libc::kevent {
        ident: fd as libc::uintptr_t,
        filter: libc::EVFILT_WRITE,
        flags: libc::EV_ADD | libc::EV_CLEAR,
        fflags: 0,
        data: 0,
        udata: std::ptr::null_mut(),
    }
}

fn empty_kqueue_event() -> libc::kevent {
    libc::kevent {
        ident: 0,
        filter: 0,
        flags: 0,
        fflags: 0,
        data: 0,
        udata: std::ptr::null_mut(),
    }
}
