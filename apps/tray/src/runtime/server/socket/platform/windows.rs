use qol_runtime::local_ipc::LocalStream;
use std::os::windows::io::AsRawSocket;
use windows_sys::Win32::Networking::WinSock::{
    recv, select, FD_SET, MSG_PEEK, SOCKET, SOCKET_ERROR, TIMEVAL,
};

pub(crate) fn peer_is_alive(stream: &LocalStream) -> bool {
    let socket = stream.as_raw_socket() as SOCKET;
    let mut readable = FD_SET {
        fd_count: 1,
        fd_array: [0; 64],
    };
    readable.fd_array[0] = socket;
    let no_wait = TIMEVAL {
        tv_sec: 0,
        tv_usec: 0,
    };
    let ready = unsafe {
        select(
            0,
            &mut readable,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &no_wait,
        )
    };
    if ready == SOCKET_ERROR {
        return false;
    }
    if ready == 0 {
        return true;
    }
    let mut buf = [0u8; 1];
    let n = unsafe { recv(socket, buf.as_mut_ptr(), 1, MSG_PEEK) };
    n > 0
}

/// A Windows restart spawns a fresh tray, so no connection is handed across.
pub(crate) fn register_lifeline_for_exec_handoff(_stream: &LocalStream) {}

pub(crate) fn unregister_lifeline_for_exec_handoff(_stream: &LocalStream) {}
