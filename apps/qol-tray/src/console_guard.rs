#[cfg(unix)]
pub fn guard_console_pipes() {
    let live = redirect_dead_fds(vec![libc::STDOUT_FILENO, libc::STDERR_FILENO], 0);
    if live.is_empty() {
        return;
    }
    let result = std::thread::Builder::new()
        .name("qol-console-guard".into())
        .spawn(|| watch_fds(live));
    if let Err(error) = result {
        log::warn!("console pipe guard failed to start: {error}");
    }
}

#[cfg(not(unix))]
pub fn guard_console_pipes() {}

#[cfg(unix)]
fn watch_fds(mut fds: Vec<libc::c_int>) {
    while !fds.is_empty() {
        fds = redirect_dead_fds(fds, -1);
    }
}

#[cfg(unix)]
fn redirect_dead_fds(fds: Vec<libc::c_int>, timeout_ms: libc::c_int) -> Vec<libc::c_int> {
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

        let live = redirect_dead_fds(vec![write_fd], 0);

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
