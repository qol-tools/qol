use std::os::fd::RawFd;

pub(crate) fn is_listening_socket(fd: RawFd) -> bool {
    super::super::socket_opt(fd, libc::SO_ACCEPTCONN).is_some_and(|accepting| accepting != 0)
}
