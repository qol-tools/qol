use qol_runtime::local_ipc::LocalStream;
use std::os::fd::AsRawFd;

pub(super) fn peer_is_alive(stream: &LocalStream) -> bool {
    let fd = stream.as_raw_fd();
    let mut buf = [0u8; 1];
    let n = unsafe {
        libc::recv(
            fd,
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len(),
            libc::MSG_PEEK | libc::MSG_DONTWAIT,
        )
    };
    if n == 0 {
        return false;
    }
    if n > 0 {
        return true;
    }
    std::io::Error::last_os_error().kind() == std::io::ErrorKind::WouldBlock
}

pub(super) fn register_lifeline_for_exec_handoff(stream: &LocalStream) {
    crate::lifeline_handoff::register(stream.as_raw_fd());
}

pub(super) fn unregister_lifeline_for_exec_handoff(stream: &LocalStream) {
    crate::lifeline_handoff::unregister(stream.as_raw_fd());
}
