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
