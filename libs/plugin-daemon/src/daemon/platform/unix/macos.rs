use std::os::fd::RawFd;

unsafe extern "C" {
    fn qol_daemon_socket_is_listening(fd: libc::c_int) -> libc::c_int;
}

pub(super) fn is_listening_socket(fd: RawFd) -> bool {
    unsafe { qol_daemon_socket_is_listening(fd) == 1 }
}
