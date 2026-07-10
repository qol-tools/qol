#[cfg(unix)]
pub fn guard_console_pipes() {
    let live = redirect_dead_fds_now(vec![libc::STDOUT_FILENO, libc::STDERR_FILENO]);
    if live.is_empty() {
        return;
    }
    spawn_console_guard(move || watch_fds(live));
}

#[cfg(not(unix))]
pub fn guard_console_pipes() {}

#[cfg(unix)]
fn spawn_console_guard(watch: impl FnOnce() + Send + 'static) {
    let result = std::thread::Builder::new()
        .name("qol-console-guard".into())
        .spawn(watch);
    if let Err(error) = result {
        log::warn!("console pipe guard failed to start: {error}");
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn watch_fds(mut fds: Vec<libc::c_int>) {
    while !fds.is_empty() {
        fds = poll_dead_fds(fds, -1);
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn redirect_dead_fds_now(fds: Vec<libc::c_int>) -> Vec<libc::c_int> {
    poll_dead_fds(fds, 0)
}

#[cfg(all(unix, not(target_os = "macos")))]
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

#[cfg(target_os = "macos")]
fn redirect_dead_fds_now(fds: Vec<libc::c_int>) -> Vec<libc::c_int> {
    monitor_fds(fds, false)
}

#[cfg(target_os = "macos")]
fn watch_fds(fds: Vec<libc::c_int>) {
    monitor_fds(fds, true);
}

#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
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

#[cfg(target_os = "macos")]
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

#[cfg(unix)]
fn redirect_to_null(fd: libc::c_int) {
    let devnull =
        unsafe { libc::open(c"/dev/null".as_ptr() as *const libc::c_char, libc::O_WRONLY) };
    if devnull < 0 {
        return;
    }
    unsafe {
        libc::dup2(devnull, fd);
        libc::close(devnull);
    }
    log::info!("console fd {fd} lost its reader; redirected to /dev/null");
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn pipe_pair() -> (libc::c_int, libc::c_int) {
        let mut fds = [0; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe() failed");
        (fds[0], fds[1])
    }

    fn write_byte(fd: libc::c_int) -> isize {
        unsafe { libc::write(fd, b"x".as_ptr() as *const libc::c_void, 1) }
    }

    #[test]
    fn dead_pipe_write_end_is_redirected_to_devnull() {
        let (read_fd, write_fd) = pipe_pair();
        std::thread::spawn(move || watch_fds(vec![write_fd]));

        unsafe { libc::close(read_fd) };

        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if write_byte(write_fd) == 1 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "guard never redirected the widowed pipe fd; writes still fail"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        unsafe { libc::close(write_fd) };
    }

    #[test]
    fn already_dead_pipe_is_redirected_synchronously() {
        let (read_fd, write_fd) = pipe_pair();
        unsafe { libc::close(read_fd) };

        let live = redirect_dead_fds_now(vec![write_fd]);

        assert!(live.is_empty(), "widowed fd must not be reported live");
        assert_eq!(
            write_byte(write_fd),
            1,
            "write must land in /dev/null immediately, with no watcher thread"
        );
        unsafe { libc::close(write_fd) };
    }

    #[test]
    fn live_pipe_is_left_untouched() {
        let (read_fd, write_fd) = pipe_pair();
        std::thread::spawn(move || watch_fds(vec![write_fd]));
        std::thread::sleep(Duration::from_millis(50));

        assert_eq!(write_byte(write_fd), 1, "healthy pipe must stay writable");
        let mut buf = [0u8; 1];
        let read = unsafe { libc::read(read_fd, buf.as_mut_ptr() as *mut libc::c_void, 1) };
        assert_eq!(
            read, 1,
            "byte must arrive at the live reader, not /dev/null"
        );
    }
}
